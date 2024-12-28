#![allow(clippy::needless_return)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::{io::{self, Write}, sync::{atomic::Ordering, Arc}, thread::{self, JoinHandle}, time::{Duration, Instant}};

use std::sync::atomic::AtomicBool;
use data_collector::DataCollector;

use uwugc::{GCParams, HeapArc, Params};

mod data_collector;

static QUIT_THREADS: AtomicBool = AtomicBool::new(false);
const MAX_SIZE: usize = 512 * 1024 * 1024;
const POLL_RATE: u64 = 20;
const TRIGGER_SIZE: usize = 32 * 1024 * 1024;

#[cfg(not(miri))]
mod non_miri;

fn start_stat_thread(heap: HeapArc, stat_collector: Arc<DataCollector<HeapStatRecord>>) -> JoinHandle<()> {
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

fn main() {
  println!("Hello, world!");
  #[cfg(not(miri))]
  non_miri::prepare_mimalloc();
  
  let heap = HeapArc::new(Params {
    gc_params: GCParams {
      poll_rate: POLL_RATE,
      trigger_size: TRIGGER_SIZE
    },
    max_size: MAX_SIZE
  });
  let stat_collector = Arc::new(DataCollector::new(4096));
  
  stat_collector.add_consumer_fn(|data: &HeapStatRecord| {
    let HeapStatRecord { max_size, usage, trigger_size } = data.clone();
    let usage = (usage as f32) / 1024.0 / 1024.0;
    let max_size = (max_size as f32) / 1024.0 / 1024.0;
    let trigger_size = (trigger_size as f32) / 1024.0 / 1024.0;
    print!("Usage: {usage: >8.2} MiB  Max: {max_size: >8.2} MiB  Trigger: {trigger_size: >8.2} MiB\r");
    io::stdout().flush().unwrap();
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
  
  let complete_time = (start_time.elapsed().as_millis() as f32) / 1024.0;
  
  drop(ctx);
  
  QUIT_THREADS.store(true, Ordering::Relaxed);
  println!();
  println!("Shutting down!");
  
  if let Some(thrd) = stat_thread {
    thrd.join().unwrap();
  }
  drop(stat_collector);
  
  println!("Test time was {complete_time:.2} secs");
  println!("Quitting :3");
  drop(heap);
}
