#![feature(negative_impls)]
#![feature(sync_unsafe_cell)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::{io::{self, Write}, sync::{atomic::Ordering, Arc}, thread::{self, JoinHandle}, time::Duration};

use heap::Heap;
use portable_atomic::AtomicBool;

mod objects_manager;
mod util;
mod heap;
mod gc;

static QUIT_THREADS: AtomicBool = AtomicBool::new(false);

fn start_gc_thread(heap: Arc<Heap>) -> JoinHandle<()> {
  return thread::spawn(move ||{
    let heap = heap;
    while !QUIT_THREADS.load(Ordering::Relaxed) {
      // If usage is larger than 128 MiB trigger GC
      if heap.get_usage() > 128 * 1024 * 1024 {
        heap.run_gc();
      }
      
      thread::sleep(Duration::from_millis(1000 / 40));
    }
  });
}

fn main() {
  println!("Hello, world!");
  
  let heap = Arc::new(Heap::new());
  
  let gc_thread = start_gc_thread(heap.clone());
  
  let heap_clone = heap.clone();
  let stat_thread = thread::spawn(move || {
    let heap = heap_clone;
    while !QUIT_THREADS.load(Ordering::Relaxed) {
      // If over 512 MiB usage panics
      if heap.get_usage() > 512 * 1024 * 1024 {
        panic!("Hard limit reached");
      }
      
      let usage = (heap.get_usage() as f64) / 1024f64 / 1024f64;
      print!("\rUsage: {usage} MiB     ");
      io::stdout().flush().unwrap();
      thread::sleep(Duration::from_millis(250));
    }
  });
  
  let ctx = heap.create_context();
  for _ in 1..1_000_000 {
    ctx.alloc(|| [0; 1024]);
  }
  drop(ctx);
  
  println!("");
  println!("Shutting down!");
  
  QUIT_THREADS.store(true, Ordering::Relaxed);
  
  stat_thread.join().unwrap();
  gc_thread.join().unwrap();
  println!("Quitting :3");
  drop(heap);
}
