// Needed for objects_manager::context::Context
#![feature(negative_impls)]

use heap::Heap;

mod objects_manager;
mod util;
mod heap;
mod gc;

fn main() {
  println!("Hello, world!");
  
  let heap = Heap::new();
  
  let ctx = heap.create_context();
  let str_ref = ctx.alloc(|| "String UwU");
  
  let str = str_ref.borrow_inner();
  println!("Str: {str}");
  
  let str2_ref = ctx.alloc(|| "Alternative");
  drop(str_ref);
  
  println!("Printing current root refs");
  let mut root_buffer = Vec::new();
  heap.take_root_snapshot(&mut root_buffer);
  
  for obj in root_buffer {
    let ptr = obj as usize;
    println!("RootRef       {ptr:#016x}");
  }
  
  drop(str2_ref);
  drop(ctx);
  
  println!("Quitting :3");
  drop(heap);
}
