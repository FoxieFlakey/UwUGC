#![feature(negative_impls)]
#![feature(sync_unsafe_cell)]
#![feature(trait_alias)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::{ffi::{c_int, c_long}, hint::black_box, sync::{atomic::Ordering, Arc}, thread::{self, JoinHandle}, time::{Duration, Instant}};

use gc::GCParams;
use heap::{Heap, HeapParams};
use mimalloc::MiMalloc;
use portable_atomic::AtomicBool;
use util::data_collector::DataCollector;

mod objects_manager;
mod util;
mod heap;
mod gc;
mod descriptor;
mod root_refs;

static QUIT_THREADS: AtomicBool = AtomicBool::new(false);
const MAX_SIZE: usize = 512 * 1024 * 1024;
const POLL_RATE: u64 = 10;
const TRIGGER_SIZE: usize = 256 * 1024 * 1024;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn start_stat_thread(heap: Arc<Heap>, stat_collector: Arc<DataCollector<HeapStatRecord>>) -> JoinHandle<()> {
  return thread::spawn(move || {
    while !QUIT_THREADS.load(Ordering::Relaxed) {
      let usage = heap.get_usage();
      stat_collector.put_data(HeapStatRecord {
        max_size: MAX_SIZE,
        usage,
        trigger_size: TRIGGER_SIZE
      });
      thread::sleep(Duration::from_millis(10));
    }
  });
}

// Information of heap at a point
#[derive(Clone)]
#[allow(dead_code)]
struct HeapStatRecord {
  max_size: usize,
  usage: usize,
  trigger_size: usize
}

fn do_test<const LENGTH: usize>(input: &mut [i32; LENGTH]) -> &mut [i32; LENGTH] {
  for byte in input.iter_mut() {
    *byte += 2;
  }
  return input;
}

fn main() {
  println!("Hello, world!");
  
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
  if !true {
    // Alloc half a gig and zero it and dealloc it
    let mut tmp =unsafe { black_box(Box::<[u8; 768 * 1024 * 1024]>::new_uninit().assume_init()) };
    tmp.fill(0xFC);
    println!("Prepared the memory!");
  }
  
  let heap = Heap::new(HeapParams {
    gc_params: GCParams {
      poll_rate: POLL_RATE,
      trigger_size: TRIGGER_SIZE
    },
    max_size: MAX_SIZE
  });
  let stat_collector = Arc::new(DataCollector::new(4096));
  
  stat_collector.add_consumer_fn(|_data: &HeapStatRecord| {
  //   let HeapStatRecord { max_size, usage, trigger_size } = data.clone();
  //   let usage = (usage as f32) / 1024.0 / 1024.0;
  //   let max_size = (max_size as f32) / 1024.0 / 1024.0;
  //   let trigger_size = (trigger_size as f32) / 1024.0 / 1024.0;
  //   print!("Usage: {usage: >8.2} MiB  Max: {max_size: >8.2} MiB  Trigger: {trigger_size: >8.2} MiB\r");
  //   io::stdout().flush().unwrap();
  });
  
  let stat_thread = {
    if true {
      Some(start_stat_thread(heap.clone(), stat_collector.clone()))
    } else {
      None
    }
  };
  
  let ctx = heap.create_context();
  
  // Raw is 1.5x faster than GC
  let start_time = Instant::now();
  let temp = [198; 1024];
  black_box(for _ in 1..200_000 {
    let mut res = ctx.alloc(|| temp);
    black_box(do_test(&mut res));
    black_box(res);
  });
  
  // struct Message {
  //   uwu: u8
  // }
  
  // static MESSAGE_DESCRIPTOR: Descriptor = Descriptor {
  //   fields: Vec::new()
  // };
  
  // unsafe impl Describeable for Message {
  //   fn get_descriptor() -> Option<&'static Descriptor> {
  //     return Some(&MESSAGE_DESCRIPTOR)
  //   }
  // }
  
  // let mut msg = ctx.alloc(|| Message {
  //   uwu: 0x8
  // });
  
  // let mut a = msg.borrow_inner().uwu;
  // println!("UwU: {a}");
  // msg.borrow_inner_mut().uwu = 12;
  // a = msg.borrow_inner().uwu;
  // println!("UwU: {a}");
  
  // drop(msg);
  drop(ctx);
  
  let complete_time = (start_time.elapsed().as_millis() as f32) / 1024.0;
  
  QUIT_THREADS.store(true, Ordering::Relaxed);
  println!("");
  println!("Shutting down!");
  
  if let Some(thrd) = stat_thread {
    thrd.join().unwrap();
  }
  drop(stat_collector);
  
  println!("Test time was {complete_time:.2} secs");
  println!("Quitting :3");
  drop(heap);
}
