// Needed for objects_manager::context::Context
#![feature(negative_impls)]

use objects_manager::ObjectManager;

mod objects_manager;

fn main() {
  println!("Hello, world!");
  
  let manager = ObjectManager::new();
  
  let ctx = manager.create_context();
  ctx.alloc(|| "Alive 1");
  ctx.alloc(|| 2 as u32);
  ctx.alloc(|| "Alive 2".to_string());
  ctx.alloc(|| 1 as u32);
  ctx.alloc(|| 2 as u32);
  drop(ctx);
  
  println!("Sweeping");
  
  // Keep non u32 data alive 
  manager.sweep(|obj| obj.borrow_inner::<u32>().is_none());
  
  drop(manager);
}
