use objects_manager::ObjectManager;

mod objects_manager;

fn main() {
  println!("Hello, world!");
  
  let manager = ObjectManager::new();
  manager.alloc(|| "Alive 1");
  manager.alloc(|| 2 as u32);
  manager.alloc(|| "Alive 2".to_string());
  manager.alloc(|| 1 as u32);
  manager.alloc(|| 2 as u32);
  
  println!("Sweeping");
  
  manager.sweep(|obj| obj.borrow_inner::<u32>().is_none());
  
  drop(manager);
}
