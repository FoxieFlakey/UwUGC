#![feature(negative_impls)]
#![feature(sync_unsafe_cell)]
#![deny(unsafe_op_in_unsafe_fn)]

use heap::Heap;

mod objects_manager;
mod util;
mod heap;
mod gc;

fn main() {
  println!("Hello, world!");
  
  let heap = Heap::new();
  
  let ctx = heap.create_context();
  let str_ref = ctx.alloc(|| "This going to be gone next");
  
  {
    let str = str_ref.borrow_inner();
    println!("Data: {str}");
  }
  
  let str2_ref = ctx.alloc(|| "Im alive");
  drop(str_ref);
  
  // Run GC and "Alternative" should be gone
  println!("Running GC");
  heap.run_gc();
  println!("Done running GC");
  
  // This should not trigger use-after-free
  {
    let str = str2_ref.borrow_inner();
    println!("Data: {str}");
  }
  
  println!("Test complete!");
  drop(str2_ref);
  drop(ctx);
  
  println!("Quitting :3");
  drop(heap);
}
