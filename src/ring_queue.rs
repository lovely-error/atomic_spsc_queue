use core::{alloc::Layout, marker::PhantomData, mem::MaybeUninit, ptr::copy_nonoverlapping, sync::atomic::{fence, AtomicU32, Ordering}};

#[repr(C)]
struct Metadata {
  read_index: AtomicU32,
  write_index: AtomicU32
}


pub struct RingQueue<T> {
  raw_queue: RingQueueRaw,
  _phantom: PhantomData<T>
}
impl <T> RingQueue<T> {
  pub fn new(capacity:usize) -> Self {
    Self { raw_queue: new_ring_queue(Layout::new::<Metadata>(), Layout::new::<T>(), capacity), _phantom: PhantomData }
  }
  pub fn enqueue_item(&self, item: &MaybeUninit<T>) -> bool {
    enqueue_item_prim(&self.raw_queue, Layout::new::<Metadata>(), Layout::new::<T>(), item.as_ptr().cast())
  }
  pub fn dequeue_item(&self, item: &mut MaybeUninit<T>) -> bool {
    dequeue_item_prim(&self.raw_queue, Layout::new::<Metadata>(), Layout::new::<T>(), item.as_mut_ptr().cast())
  }
  /// ensure to drain the q
  pub unsafe fn dispose(self) {
    destroy(self.raw_queue, Layout::new::<Metadata>(), Layout::new::<T>());
  }
}

struct RingQueueRaw {
  backing_store: *mut (),
  capacity: usize,
}
unsafe impl Sync for RingQueueRaw {}

fn indexing_adjusted_capacity(capacity:usize) -> usize {
  capacity + 2
}

#[inline(always)]
fn alloc_ring_queue_backing_store(
  metadata_layout:Layout,
  item_layout:Layout,
  capacity:usize,
) -> *mut () {
  let midpoint = metadata_layout.size().next_multiple_of(item_layout.align());
  let indexing_adjusted_capacity = indexing_adjusted_capacity(capacity);
  let total_size = midpoint + item_layout.size() * indexing_adjusted_capacity;

  let align = metadata_layout.align().max(item_layout.align());
  let mem_ptr = unsafe { std::alloc::alloc(Layout::from_size_align_unchecked(total_size, align)) };

  let mid_ptr = mem_ptr.map_addr(|addr| addr + midpoint);

  return mid_ptr.cast::<()>()
}

#[inline(always)]
fn mid_to_origin_ptr(
  mid_ptr:*mut (),
  metadata_layout:Layout,
  item_layout:Layout
) -> *mut () {
  let align = metadata_layout.align().max(item_layout.align());
  mid_ptr.map_addr(|addr| (addr - metadata_layout.size()) & !(align - 1))
}


fn new_ring_queue(
  metadata_layout:Layout,
  item_layout:Layout,
  capacity:usize,
) -> RingQueueRaw {
  if capacity == 0 { panic!("Capacity must not be zero") }
  let mid_ptr = alloc_ring_queue_backing_store(metadata_layout, item_layout, capacity);
  let mtd_ptr = mid_ptr.map_addr(|addr| addr - metadata_layout.size());
  let mtd_ptr = mtd_ptr.cast::<Metadata>();
  let indexing_adjusted_capacity = indexing_adjusted_capacity(capacity);
  let initial_read_index = indexing_adjusted_capacity - 1;
  let initial_write_index = 0;
  unsafe { mtd_ptr.write(Metadata {
    read_index: AtomicU32::new(initial_read_index as _),
    write_index: AtomicU32::new(initial_write_index)
  }) };
  let result = RingQueueRaw {
    backing_store: mid_ptr,
    capacity: capacity
  };
  return result;
}

fn destroy(
  queue: RingQueueRaw,
  metadata_layout:Layout,
  item_layout:Layout,
) {
  let origin_ptr = mid_to_origin_ptr(queue.backing_store, metadata_layout, item_layout);
  let midpoint = metadata_layout.size().next_multiple_of(item_layout.align());
  let indexing_adjusted_capacity = indexing_adjusted_capacity(queue.capacity);
  let total_size = midpoint + item_layout.size() * indexing_adjusted_capacity;
  let align = metadata_layout.align().max(item_layout.align());
  unsafe {
    let layout = Layout::from_size_align_unchecked(total_size, align);
    std::alloc::dealloc(origin_ptr.cast::<u8>(), layout);
  }
}


fn enqueue_item_prim(
  queue: &RingQueueRaw,
  metadata_layout:Layout,
  item_layout:Layout,
  item_data_src_ptr: *const (),
) -> bool {
  let backing_store_ptr = queue.backing_store;
  let mtd_ptr = backing_store_ptr.map_addr(|addr| addr - metadata_layout.size());
  let mtd_ptr = unsafe{&mut *mtd_ptr.cast::<Metadata>()};
  let prior_write_index = mtd_ptr.write_index.load(Ordering::Acquire);
  let bumped_index = prior_write_index + 1;
  let indexing_adjusted_capacity = indexing_adjusted_capacity(queue.capacity);
  let next_write_index = (bumped_index) * (!(bumped_index == (indexing_adjusted_capacity as u32)) as u32);
  let current_read_index = mtd_ptr.read_index.load(Ordering::Relaxed);
  let full = next_write_index == current_read_index;
  if full {
    return false
  }
  let write_slot = backing_store_ptr.map_addr(|addr| addr + ((prior_write_index as usize) * item_layout.size()));
  unsafe { copy_nonoverlapping(item_data_src_ptr.cast::<u8>(), write_slot.cast::<u8>(), item_layout.size()) };
  mtd_ptr.write_index.store(next_write_index, Ordering::Release);

  return true
}


fn dequeue_item_prim(
  queue: &RingQueueRaw,
  metadata_layout:Layout,
  item_layout:Layout,
  item_data_dst_ptr: *mut (),
) -> bool {
  let backing_store_ptr = queue.backing_store;
  let mtd_ptr = backing_store_ptr.map_addr(|addr| addr - metadata_layout.size());
  let mtd_ptr = unsafe{&mut *mtd_ptr.cast::<Metadata>()};
  let read_index = mtd_ptr.read_index.load(Ordering::Acquire);
  let bumped_index = read_index + 1;
  let indexing_adjusted_capacity = indexing_adjusted_capacity(queue.capacity);
  let next_index = bumped_index * (!(bumped_index == (indexing_adjusted_capacity as u32)) as u32);
  let write_index = mtd_ptr.write_index.load(Ordering::Relaxed);
  let empty = next_index == write_index;
  if empty {
    return false;
  }
  let read_slot = backing_store_ptr.map_addr(|addr| addr + (next_index as usize) * item_layout.size());
  unsafe { copy_nonoverlapping(read_slot.cast::<u8>(), item_data_dst_ptr.cast::<u8>(), item_layout.size()) };
  mtd_ptr.read_index.store(next_index, Ordering::Release);

  return true;
}

#[test]
fn basic() {
  let mtd_l = Layout::new::<Metadata>();
  let item_l = Layout::new::<u32>();
  let capacity = 16;
  let q = new_ring_queue(mtd_l, item_l, capacity);
  let item = 777u32;
  let result = enqueue_item_prim(&q, mtd_l, item_l, &raw const item as _);
  println!("{}", result);
  let mut out = MaybeUninit::<u32>::uninit();
  let _result = dequeue_item_prim(&q, mtd_l, item_l, out.as_mut_ptr() as _);
  println!("{}", unsafe { out.assume_init() });
}
#[test]
fn basic2() {
  let mtd_l = Layout::new::<Metadata>();
  let item_l = Layout::new::<u32>();
  let capacity = 16;
  let q = new_ring_queue(mtd_l, item_l, capacity);
  for item in 0 .. capacity {
    let result = enqueue_item_prim(&q, mtd_l, item_l, &raw const item as _);
    println!("{}:{}", item, result);
  }

  let mut out = MaybeUninit::<u32>::uninit();
  for _ in 0 .. capacity {
    let _result = dequeue_item_prim(&q, mtd_l, item_l, out.as_mut_ptr() as _);
    println!("{}:{}", _result, unsafe { out.assume_init() });
  }
  destroy(q, mtd_l, item_l);
}
#[test]
fn basic3() {
  let mtd_l = Layout::new::<Metadata>();
  let item_l = Layout::new::<u64>();
  let capacity = 4;
  let q = new_ring_queue(mtd_l, item_l, capacity);
  for item in 0 .. capacity {
    let result = enqueue_item_prim(&q, mtd_l, item_l, &raw const item as _);
    println!("{}:{}", item, result);
  }
  let mut out = MaybeUninit::<u64>::uninit();
  for _ in 0 .. capacity {
    let _result = dequeue_item_prim(&q, mtd_l, item_l, out.as_mut_ptr() as _);
    println!("{}:{}", _result, unsafe { out.assume_init() });
  }
  destroy(q, mtd_l, item_l);
}

#[test]
fn mt_test() {
  const CAPACITY : usize = 4096 * 16;
  let q = RingQueue::<u32>::new(CAPACITY);
  let sync_var = AtomicU32::new(0);
  let producer = unsafe {
    std::thread::Builder::new().spawn_unchecked({
      let sync_var = &sync_var;
      let q = &q;
      move || {
        let _ = sync_var.fetch_add(1, Ordering::AcqRel);
        while sync_var.load(Ordering::Relaxed) != 2 {}
        fence(Ordering::SeqCst);
        for i in 0 .. CAPACITY {
          let i = MaybeUninit::new(i as u32);
          let ok = q.enqueue_item(&i);
          assert!(ok);
          // todo: random sleep here?
        }
      }
    })
  };
  let consumer = unsafe {
    std::thread::Builder::new().spawn_unchecked({
      let sync_var = &sync_var;
      let q = &q;
      move || {
        let _ = sync_var.fetch_add(1, Ordering::AcqRel);
        while sync_var.load(Ordering::Relaxed) != 2 {}
        let mut result = Vec::new();
        result.reserve(CAPACITY);
        fence(Ordering::SeqCst);
        let mut recv_count = 0;
        let mut i = MaybeUninit::uninit();
        loop {
          let ok = q.dequeue_item(&mut i);
          if ok {
            result.push(i.assume_init());
            recv_count += 1;
            if recv_count == CAPACITY { break }
          }
        }
        result
      }
    })
  };
  let val = consumer.unwrap().join().unwrap();
  producer.unwrap().join().unwrap();
  for (a,b) in val.iter().zip(0..) {
    assert!(*a == b)
  }
}