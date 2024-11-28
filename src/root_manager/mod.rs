use std::{collections::HashMap, sync::{Arc, Mutex, RwLock}, thread::{self, ThreadId}};

use context::Context;
use intrusive_collections::{intrusive_adapter, LinkedListLink};

use crate::objects_manager::{Object, ObjectManager};

pub use context::ContextHandle;

mod context;

pub(super) struct RootEntry<'a> {
  link: LinkedListLink,
  obj: &'a mut Object
}

intrusive_adapter!(RootEntryAdapter = Box<RootEntry<'static>>: RootEntry { link: LinkedListLink });

pub struct Heap {
  object_manager: ObjectManager,
  contexts: Mutex<HashMap<ThreadId, Arc<Context>>>,
  
  gc_lock: RwLock<()>
}

impl Heap {
  pub fn new() -> Self {
    return Self {
      object_manager: ObjectManager::new(),
      contexts: Mutex::new(HashMap::new()),
      gc_lock: RwLock::new(())
    };
  }
  
  pub fn create_context(&self) -> ContextHandle {
    let mut contexts = self.contexts.lock().unwrap();
    let ctx = contexts.entry(thread::current().id())
      .or_insert_with(|| Arc::new(Context::new()));
    
    return ContextHandle::new(self, self.object_manager.create_context(), ctx.clone());
  }
  
  pub fn take_root_snapshot(&self, buffer: &mut Vec<*mut Object>) {
    let contexts = self.contexts.lock().unwrap();
    for ctx in contexts.values() {
      ctx.for_each_root(|entry| {
        // NOTE: Cast reference to *mut Object because after this
        // return caller must ensure that *mut Object is valid
        // because after this returns no lock ensures that GC isn't
        // actively collect that potential *mut Object
        buffer.push(entry.obj as *const Object as *mut Object);
      });
    }
  }
}


