use std::ffi::{c_int, c_long};
#[allow(unused_imports)]
use std::hint::black_box;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

pub fn prepare_mimalloc() {
  // Reserve 512 MiB from mimalloc
  // mimalloc crate does not expose
  // necessary mi_reserve_os_memory function for this
  #[allow(dead_code)]
  const MI_OPTION_SHOW_ERRORS: c_int = 0;
  #[allow(dead_code)]
  const MI_OPTION_VERBOSE: c_int = 2;
  #[allow(dead_code)]
  const MI_OPTION_ARENA_EAGER_COMMIT: c_int = 4;
  #[allow(dead_code)]
  const MI_OPTION_PURGE_DELAY: c_int = 15;
  
  extern "C" {
    fn mi_reserve_os_memory(size: usize, commit: bool, allow_large: bool) -> c_int;
    fn mi_option_set(option: c_int, val: c_long);
    fn mi_option_enable(option: c_int);
  }
  
  unsafe {
    mi_option_enable(MI_OPTION_SHOW_ERRORS);
    mi_option_enable(MI_OPTION_ARENA_EAGER_COMMIT);
    mi_option_set(MI_OPTION_PURGE_DELAY, 30_000);
  };
  
  if unsafe { mi_reserve_os_memory(512 * 1024 * 1024, true, false) } != 0 {
    panic!("Error reserving memory");
  }
  
  // Need to use the half a gig to actually commits it
  // Alloc half a gig and zero it and dealloc it
  // let mut tmp =unsafe { black_box(Box::<[u8; 768 * 1024 * 1024]>::new_uninit().assume_init()) };
  // tmp.fill(0xFC);
  // println!("Prepared the memory!");
}
