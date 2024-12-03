use std::{sync::{Arc, Weak}, thread::{self, JoinHandle}, time::Duration};
use parking_lot::{Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::heap::Heap;

pub struct GCParams {
  pub trigger_size: usize,
  pub poll_rate: u64
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GCCommand {
  RunGC
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GCRunState {
  Paused,
  Running,
  Stopped
}

#[derive(Clone)]
struct GCCommandStruct {
  command: Option<GCCommand>,
  submit_count: u64,
  execute_count: u64
}

struct GCInnerState {
  gc_lock: RwLock<()>,
  owner: Weak<Heap>,
  params: GCParams,
  
  run_state: Mutex<GCRunState>,
  run_state_changed_event: Condvar,
  
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
    self.set_gc_run_state(GCRunState::Stopped);
    self.thread.take().unwrap().join().unwrap();
  }
}

impl GCState {
  fn set_gc_run_state(&self, state: GCRunState) {
    *self.inner_state.run_state.lock() = state;
    
    // Notify the GC of run state change, there will be
    // only one primary thread so notify_one is better choice
    self.inner_state.run_state_changed_event.notify_one();
  }
  
  // Wait for any currently executing command to be completed
  fn wait_for_gc<'a>(&'a self, submit_count: Option<u64>, cmd_control: Option<MutexGuard<'a, GCCommandStruct>>) -> MutexGuard<'a, GCCommandStruct> {
    let mut cmd_control = cmd_control
      .or_else(|| Some(self.inner_state.command.lock()))
      .unwrap();
    
    let submit_count = submit_count
      .or(Some(cmd_control.submit_count))
      .unwrap();
    
    while submit_count > cmd_control.execute_count {
      self.inner_state.cmd_executed_event.wait(&mut cmd_control);
    }
    return cmd_control;
  }
  
  fn call_gc(&self, cmd: GCCommand) {
    let mut cmd_control = self.inner_state.command.lock();
    if let Some(current_cmd) = cmd_control.command {
      let combine_command =  match cmd {
        // GCCommand::RunGC can be combined with potentially
        // in progress RunGC command because there no need
        // to fire multiple GCs in a row because multiple
        // thread coincidentally attempt to do that when one
        // is really enough until next time
        GCCommand::RunGC => true
      };
      
      if combine_command && current_cmd == cmd {
        let _ = self.wait_for_gc(None, Some(cmd_control));
        return;
      }
    }
    
    // Wait for any previous command to be executed
    cmd_control = self.wait_for_gc(None, Some(cmd_control));
    
    // There must not be any command in execution, GC replace it
    // with None after completing a command
    assert_eq!(cmd_control.command.is_none(), true);
    cmd_control.submit_count += 1;
    cmd_control.command = Some(cmd);
    
    // Wait for current command to be executed
    drop(self.wait_for_gc(Some(cmd_control.submit_count), Some(cmd_control)));
  }
  
  fn process_command(gc_state: &Arc<GCInnerState>, heap: &Heap, cmd_struct: GCCommandStruct) {
    match cmd_struct.command.unwrap() {
      GCCommand::RunGC => {
        heap.gc_state.run_gc_internal(&heap);
      }
    }
    
    let mut cmd_control = gc_state.command.lock();
    cmd_control.command = None;
    cmd_control.execute_count += 1;
    drop(cmd_control);
    
    gc_state.cmd_executed_event.notify_all();
  }
  
  fn do_gc_heuristics(gc_state: &Arc<GCInnerState>, heap: &Heap, cmd_control: &mut GCCommandStruct) {
    if heap.get_usage() >= gc_state.params.trigger_size {
      cmd_control.submit_count += 1;
      cmd_control.command = Some(GCCommand::RunGC);
    }
  }
  
  fn gc_poll(inner: &Arc<GCInnerState>, heap: &Heap) {
    let mut cmd_control = inner.command.lock();
    
    // If there no command to be executed
    // let 'do_gc_heuristics' decide what
    // to do next based on heuristics, it
    // may injects a new command into cmd_control
    // to send command, instead relying other
    // if statement and special conditions
    if cmd_control.command.is_none() {
      Self::do_gc_heuristics(&inner, &heap, &mut cmd_control);
    }
    
    let cmd_struct = cmd_control.clone();
    let cmd = cmd_control.command;
    drop(cmd_control);
    
    if let Some(_) = cmd {
      Self::process_command(&inner, &heap, cmd_struct);
    }
  }
  
  pub fn unpause_gc(&self) {
    self.set_gc_run_state(GCRunState::Running);
  }
  
  pub fn new(params: GCParams, owner: Weak<Heap>) -> GCState {
    let inner_state = Arc::new(GCInnerState {
      gc_lock: RwLock::new(()),
      owner,
      params,
      
      run_state: Mutex::new(GCRunState::Paused),
      run_state_changed_event: Condvar::new(),
      
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
        
        'poll_loop: loop {
          // Check GC run state
          let mut run_state = inner.run_state.lock();
          'run_state_poll_loop: loop {
            match *run_state {
              // If GC paused, continue waiting for state changed event
              // or GC got spurious wake up during paused
              GCRunState::Paused => (),
              // If GC running, break out of this loop to execute normally
              GCRunState::Running => break 'run_state_poll_loop,
              // If GC is stopped, break of of outer poll loop to quit
              GCRunState::Stopped => break 'poll_loop
            }
            
            inner.run_state_changed_event.wait(&mut run_state);
          }
          drop(run_state);
          
          // If 'heap' can't be upgraded, that signals the GC thread
          // to shutdown, incase if Heap is dropped after run state check
          let heap = match inner.owner.upgrade() {
            Some(x) => x,
            None => break
          };
          Self::gc_poll(&inner, &heap);
          thread::sleep(Duration::from_millis(sleep_delay_milisec));
        }
        
        println!("Shutting down GC");
      }))
    }
  }
  
  pub fn block_gc(&self) -> GCLockCookie {
    return GCLockCookie {
      _cookie: self.inner_state.gc_lock.read()
    }
  }
  
  pub fn block_mutators(&self) -> GCExclusiveLockCookie {
    return GCExclusiveLockCookie {
      _cookie: self.inner_state.gc_lock.write()
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
      obj.set_mark_bit(&heap.object_manager);
    }
    
    // Step 3 (Concurrent): Sweep dead objects and reset mark flags 
    // SAFETY: just marked live objects and dead objects
    // is well dead
    unsafe { sweeper.sweep_and_reset_mark_flag() };
  }
}

