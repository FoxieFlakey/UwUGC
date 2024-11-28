// Needed for objects_manager::context::Context
#![feature(negative_impls)]

use std::{sync::Arc, thread};

use objects_manager::ObjectManager;

mod objects_manager;
mod util;

fn main() {
  println!("Hello, world!");
  
  let manager = Arc::new(ObjectManager::new());
  
  let manager_for_thread = manager.clone();
  let join = thread::spawn(move || {
    let ctx = manager_for_thread.create_context();
    ctx.alloc(|| "Alive 1");
    ctx.alloc(|| 2 as u32);
    ctx.alloc(|| "Alive 2".to_string());
    drop(ctx);
  });
  
  let ctx = manager.create_context();
  ctx.alloc(|| "Alive 1");
  ctx.alloc(|| 2 as u32);
  ctx.alloc(|| "Alive 2".to_string());
  drop(ctx);
  
  join.join().unwrap();
  println!("Sweeping");
  
  // Everything is dead
  unsafe { manager.create_sweeper().sweep() };
  
  println!("Dropping manager");
  drop(manager);
}
