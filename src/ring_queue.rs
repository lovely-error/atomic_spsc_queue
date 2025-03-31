use core::{alloc::Layout, marker::PhantomData, mem::MaybeUninit, ptr::copy_nonoverlapping};

#[repr(C)]
struct Metadata {
  read_index: u32,
  write_index: u32,
}

struct RingQueue<T> {
  backing_store: *mut (),
  _phantom: PhantomData<*mut T>
}

struct RingQueueRaw {
  backing_store: *mut (),
}

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
  unsafe { mtd_ptr.write(Metadata {
    read_index: (indexing_adjusted_capacity - 1) as u32, write_index: 0
  }) };
  let result = RingQueueRaw {
    backing_store: mid_ptr
  };
  return result;
}

fn destroy(
  queue: RingQueueRaw,
  metadata_layout:Layout,
  item_layout:Layout,
  capacity:usize
) {
  let origin_ptr = mid_to_origin_ptr(queue.backing_store, metadata_layout, item_layout);
  let midpoint = metadata_layout.size().next_multiple_of(item_layout.align());
  let indexing_adjusted_capacity = indexing_adjusted_capacity(capacity);
  let total_size = midpoint + item_layout.size() * indexing_adjusted_capacity;
  let align = metadata_layout.align().max(item_layout.align());
  unsafe {
    let layout = Layout::from_size_align_unchecked(total_size, align);
    std::alloc::dealloc(origin_ptr.cast::<u8>(), layout);
  }
}


fn enqueue(
  queue: &RingQueueRaw,
  metadata_layout:Layout,
  item_layout:Layout,
  item_data_src_ptr: *const (),
  capacity:usize
) -> bool {
  let backing_store_ptr = queue.backing_store;
  let mtd_ptr = backing_store_ptr.map_addr(|addr| addr - metadata_layout.size());
  let mtd_ptr = unsafe{&mut *mtd_ptr.cast::<Metadata>()};
  let prior_write_index = mtd_ptr.write_index;
  let bumped_index = prior_write_index + 1;
  let indexing_adjusted_capacity = indexing_adjusted_capacity(capacity);
  let next_index = (bumped_index) * (!(bumped_index == (indexing_adjusted_capacity as u32)) as u32);
  let full = next_index == mtd_ptr.read_index;
  if full {
    return false
  }
  let write_slot = backing_store_ptr.map_addr(|addr| addr + ((prior_write_index as usize) * item_layout.size()));
  unsafe { copy_nonoverlapping(item_data_src_ptr.cast::<u8>(), write_slot.cast::<u8>(), item_layout.size()) };
  mtd_ptr.write_index = next_index;

  return true
}

fn dequeue(
  queue: &RingQueueRaw,
  metadata_layout:Layout,
  item_layout:Layout,
  item_data_dst_ptr: *mut (),
  capacity:usize
) -> bool {
  let backing_store_ptr = queue.backing_store;
  let mtd_ptr = backing_store_ptr.map_addr(|addr| addr - metadata_layout.size());
  let mtd_ptr = unsafe{&mut *mtd_ptr.cast::<Metadata>()};
  let bumped_index = mtd_ptr.read_index + 1;
  let indexing_adjusted_capacity = indexing_adjusted_capacity(capacity);
  let next_index = bumped_index * (!(bumped_index == (indexing_adjusted_capacity as u32)) as u32);
  let empty = next_index == mtd_ptr.write_index;
  if empty {
    return false;
  }
  let read_slot = backing_store_ptr.map_addr(|addr| addr + (next_index as usize) * item_layout.size());
  unsafe { copy_nonoverlapping(read_slot.cast::<u8>(), item_data_dst_ptr.cast::<u8>(), item_layout.size()) };
  mtd_ptr.read_index = next_index;
  return true;
}

#[test]
fn basic() {
  let mtd_l = Layout::new::<Metadata>();
  let item_l = Layout::new::<u32>();
  let capacity = 16;
  let q = new_ring_queue(mtd_l, item_l, capacity);
  let item = 777u32;
  let result = enqueue(&q, mtd_l, item_l, &raw const item as _, capacity);
  println!("{}", result);
  let mut out = MaybeUninit::<u32>::uninit();
  let _result = dequeue(&q, mtd_l, item_l, out.as_mut_ptr() as _, capacity);
  println!("{}", unsafe { out.assume_init() });
}
#[test]
fn basic2() {
  let mtd_l = Layout::new::<Metadata>();
  let item_l = Layout::new::<u32>();
  let capacity = 4;
  let q = new_ring_queue(mtd_l, item_l, capacity);
  for item in 0 .. capacity {
    let result = enqueue(&q, mtd_l, item_l, &raw const item as _, capacity);
    println!("{}:{}", item, result);
  }
  let mut out = MaybeUninit::<u32>::uninit();
  for _ in 0 .. capacity {
    let _result = dequeue(&q, mtd_l, item_l, out.as_mut_ptr() as _, capacity);
    println!("{}:{}", _result, unsafe { out.assume_init() });
  }
  destroy(q, mtd_l, item_l, capacity);
}
#[test]
fn basic3() {
  let mtd_l = Layout::new::<Metadata>();
  let item_l = Layout::new::<u64>();
  let capacity = 4;
  let q = new_ring_queue(mtd_l, item_l, capacity);
  for item in 0 .. capacity {
    let result = enqueue(&q, mtd_l, item_l, &raw const item as _, capacity);
    println!("{}:{}", item, result);
  }
  let mut out = MaybeUninit::<u64>::uninit();
  for _ in 0 .. capacity {
    let _result = dequeue(&q, mtd_l, item_l, out.as_mut_ptr() as _, capacity);
    println!("{}:{}", _result, unsafe { out.assume_init() });
  }
  destroy(q, mtd_l, item_l, capacity);
}

