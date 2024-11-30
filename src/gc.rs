use std::{sync::{Arc, Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak}, thread::{self, JoinHandle}, time::Duration};

use crate::heap::Heap;

pub struct GCParams {
  pub trigger_size: usize,
  pub poll_rate: u64
}

#[derive(Clone)]
enum GCCommand {
  Shutdown,
  RunGC
}

struct GCCommandStruct {
  command: Option<GCCommand>,
  submit_count: u64,
  execute_count: u64
}

struct GCInnerState {
  gc_lock: RwLock<()>,
  owner: Weak<Heap>,
  params: GCParams,
  
  // GC will regularly checks this and execute command
  // then wake other
  command: Mutex<GCCommandStruct>,
  cmd_executed_event: Condvar
}

pub struct GCState {
  inner_state: Arc<GCInnerState>,
  thread: Option<JoinHandle<()>>
}

pub struct GCLockCookie<'a> {
  _cookie: RwLockReadGuard<'a, ()>
}

pub struct GCExclusiveLockCookie<'a> {
  _cookie: RwLockWriteGuard<'a, ()>
}

impl Drop for GCState {
  fn drop(&mut self) {
    self.call_gc(GCCommand::Shutdown);
    self.thread.take().unwrap().join().unwrap();
  }
}

impl GCState {
  // Wait for any currently executing command to be completed
  fn wait_for_gc<'a>(&self, mut cmd_control: MutexGuard<'a, GCCommandStruct>) -> MutexGuard<'a, GCCommandStruct> {
    let submitted_count = cmd_control.submit_count;
    while submitted_count > cmd_control.execute_count {
      cmd_control = self.inner_state.cmd_executed_event.wait(cmd_control).unwrap();
    }
    return cmd_control;
  }
  
  fn call_gc(&self, cmd: GCCommand) {
    // Wait for any previous command to be executed
    let mut cmd_control = self.wait_for_gc(self.inner_state.command.lock().unwrap());
    cmd_control.submit_count += 1;
    cmd_control.command = Some(cmd);
    
    // Wait for current command to be executed
    drop(self.wait_for_gc(cmd_control));
  }
  
  pub fn new(params: GCParams, owner: Weak<Heap>) -> GCState {
    let inner_state = Arc::new(GCInnerState {
      gc_lock: RwLock::new(()),
      owner,
      params,
      
      command: Mutex::new(GCCommandStruct {
        command: None,
        execute_count: 0,
        submit_count: 0
      }),
      cmd_executed_event: Condvar::new()
    });
    
    return GCState {
      inner_state: inner_state.clone(),
      thread: Some(thread::spawn(move || {
        let inner = inner_state;
        let sleep_delay_milisec = 1000 / inner.params.poll_rate;
        
        // Increment execute_count by one and wake others
        // that a command is executed
        let report_as_executed = || {
          let mut cmd_control = inner.command.lock().unwrap();
          cmd_control.command = None;
          cmd_control.execute_count += 1;
          drop(cmd_control);
          
          inner.cmd_executed_event.notify_all();
        };
        
        'poll_loop: loop {
          let cmd = inner.command.lock().unwrap().command.clone();
          if let Some(cmd) = cmd {
            match cmd {
              GCCommand::RunGC => {
                let heap = inner.owner.upgrade().unwrap();
                heap.gc_state.run_gc_internal(&heap);
                report_as_executed();
              },
              GCCommand::Shutdown => {
                report_as_executed();
                break 'poll_loop
              }
            }
          } else {
            // Does default watching rate before running GC
            let heap = inner.owner.upgrade().unwrap();
            
            // If above trigger run the GC
            if heap.get_usage() > inner.params.trigger_size {
              heap.gc_state.run_gc_internal(&heap);
            }
          }
          
          thread::sleep(Duration::from_millis(sleep_delay_milisec));
        }
        
        println!("Shutting down GC");
      }))
    }
  }
  
  pub fn block_gc(&self) -> GCLockCookie {
    return GCLockCookie {
      _cookie: self.inner_state.gc_lock.read().unwrap()
    }
  }
  
  pub fn block_mutators(&self) -> GCExclusiveLockCookie {
    return GCExclusiveLockCookie {
      _cookie: self.inner_state.gc_lock.write().unwrap()
    }
  }
  
  pub fn run_gc(&self) {
    self.call_gc(GCCommand::RunGC);
  }
  
  fn run_gc_internal(&self, heap: &Heap) {
    // Step 1 (STW): Take root snapshot and take objects in heap snapshot
    let block_mutator_cookie = self.block_mutators();
    
    let mut root_snapshot = Vec::new();
    // SAFETY: Just blocked the mutators
    unsafe { heap.take_root_snapshot_unlocked(&mut root_snapshot) };
    let sweeper = heap.object_manager.create_sweeper();
    
    drop(block_mutator_cookie);
    
    // Step 2 (Concurrent): Mark objects
    for obj in root_snapshot {
      // SAFETY: Object is reference from root that mean
      // mutator still using it therefore GC must keep it alive
      let obj = unsafe { &*obj };
      
      // Mark it
      obj.mark();
    }
    
    // Step 3 (Concurrent): Sweep dead objects and reset mark flags 
    // SAFETY: just marked live objects and dead objects
    // is well dead
    unsafe { sweeper.sweep_and_reset_mark_flag() };
  }
}

