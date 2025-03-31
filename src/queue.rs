use core::{alloc::Layout, marker::PhantomData, mem::MaybeUninit, sync::atomic::{fence, AtomicU64, Ordering}};

macro static_assert($cond:expr) {
  const _ : () = if !$cond { panic!() } else {};
}

macro if_spec_off {
  ($cond:expr,
  $when_true:block,
  $when_false:block
  ) => {
    {
      let cond = core::hint::black_box($cond);
      #[allow(unused_unsafe)] unsafe { core::arch::asm!("LFENCE", options(nostack, preserves_flags)) };
      if cond { $when_true } else { $when_false }
    }
  },
  ($cond:expr,
  $when_true:block
  ) => {
    {
      let cond = core::hint::black_box($cond);
      #[allow(unused_unsafe)] unsafe { core::arch::asm!("LFENCE", options(nostack, preserves_flags)) };
      if cond { $when_true } else { }
    }
  }
}
fn test(inp: usize, val: &mut usize) {
  let cond = inp % 3 == 0;
  if_spec_off!(cond, {
    *val += 1;
  })
}

#[repr(C)]
struct WriterMetadata {
  w_index: AtomicU64,
}

#[repr(C)]
struct ReaderMetadata {
  r_index: AtomicU64,
}

#[repr(C)]
struct Page {
  w_mtd: WriterMetadata,
  _bytes1: MaybeUninit<[u8; 2048 - size_of::<WriterMetadata>()]>,
  r_mtd: ReaderMetadata,
  _bytes2: MaybeUninit<[u8; 2048 - size_of::<ReaderMetadata>()]>,
}

struct PipeInRef<T> {
  backing_store_ptr: *mut Page,
  _phantom: PhantomData<*mut T>
}

impl <T>  PipeInRef<T> {
  fn push(&self, item: T) {
    let w_mtd = unsafe{&(*self.backing_store_ptr).w_mtd};
    let r_mtd = unsafe{&(*self.backing_store_ptr).r_mtd};
    let w_index = w_mtd.w_index.load(Ordering::Acquire);
    let r_index = r_mtd.r_index.load(Ordering::Acquire);
    let index_bump = w_index + 1;
    let next_write = index_bump * ((index_bump == 2) as u64);
    let full = r_index == next_write;
    if_spec_off!(full, {

    })
  }
}

#[inline]
fn make_pipe<T>() -> PipeInRef<T> {

  let mem = unsafe { std::alloc::alloc(Layout::from_size_align_unchecked(4096, 4096)) };

  let page_mem = mem.cast::<Page>();

  unsafe {
    (*page_mem).r_mtd = ReaderMetadata {
      r_index: AtomicU64::new(0)
    };
    (*page_mem).w_mtd = WriterMetadata {
      w_index: AtomicU64::new(0)
    };
  }
  fence(Ordering::SeqCst);

  return PipeInRef { backing_store_ptr: page_mem, _phantom: PhantomData }
}

#[test]
fn basic() {
  let pipe = make_pipe::<u32>();

}
