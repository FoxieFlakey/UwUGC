use std::{sync::{atomic::Ordering, mpsc, Arc, Weak}, thread::{self, JoinHandle}, time::Duration};
use parking_lot::{Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use portable_atomic::AtomicBool;

use crate::{heap::HeapState, objects_manager::{Object, ObjectManager}};

// NOTE: This is considered public API
// therefore be careful with breaking changes
#[derive(Clone)]
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

#[derive(Clone, Copy, PartialEq, Eq)]
struct ObjectPtrSend(*const Object);
impl From<&Object> for ObjectPtrSend {
  fn from(value: &Object) -> Self {
    return Self(value as *const Object);
  }
}

// SAFETY: Its safe just need a wrapper
unsafe impl Send for ObjectPtrSend {}

struct GCInnerState {
  gc_lock: RwLock<()>,
  owner: Weak<HeapState>,
  params: GCParams,
  
  run_state: Mutex<GCRunState>,
  run_state_changed_event: Condvar,
  
  // GC will regularly checks this and execute command
  // then wake other
  command: Mutex<GCCommandStruct>,
  cmd_executed_event: Condvar,
  
  // Whether or not to active load barrier
  activate_load_barrier: AtomicBool,
  
  // Remark queue for unmarked object
  // encountered by the load barrier
  remark_queue_sender: mpsc::Sender<ObjectPtrSend>
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

// An structure only for containing stuffs neede to run GC
// at the GC thread (mostly containing receiver side of queues
// which can't be put into monolithic GCInnerState structure)
struct GCThreadPrivate {
  remark_queue_receiver: mpsc::Receiver<ObjectPtrSend>
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
  
  fn process_command(gc_state: &Arc<GCInnerState>, heap: &HeapState, cmd_struct: GCCommandStruct, private: &GCThreadPrivate) {
    match cmd_struct.command.unwrap() {
      GCCommand::RunGC => {
        heap.gc_state.run_gc_internal(&heap, private);
      }
    }
    
    let mut cmd_control = gc_state.command.lock();
    cmd_control.command = None;
    cmd_control.execute_count += 1;
    drop(cmd_control);
    
    gc_state.cmd_executed_event.notify_all();
  }
  
  fn do_gc_heuristics(gc_state: &Arc<GCInnerState>, heap: &HeapState, cmd_control: &mut GCCommandStruct) {
    if heap.get_usage() >= gc_state.params.trigger_size {
      cmd_control.submit_count += 1;
      cmd_control.command = Some(GCCommand::RunGC);
    }
  }
  
  fn gc_poll(inner: &Arc<GCInnerState>, heap: &HeapState, private: &GCThreadPrivate) {
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
      Self::process_command(&inner, &heap, cmd_struct, private);
    }
  }
  
  pub fn unpause_gc(&self) {
    self.set_gc_run_state(GCRunState::Running);
  }
  
  pub fn load_barrier(&self, object: &Object, obj_manager: &ObjectManager, _block_gc_cookie: &mut GCLockCookie) -> bool {
    // Load barrier is deactivated
    if !self.inner_state.activate_load_barrier.load(Ordering::Relaxed) {
      return false;
    }
    
    if !object.set_mark_bit(obj_manager) {
      return false;
    }
    
    // SAFETY: This always succeded because the receiver end will
    // only be destroyed if Heap is no longer exist anywhere and there
    // can't be a way this be called as load barrier only can be triggered
    // by Heap existing on mutator code
    unsafe {
      self.inner_state.remark_queue_sender.send(object.into()).unwrap_unchecked();
    }
    return true;
  }
  
  pub fn new(params: GCParams, owner: Weak<HeapState>) -> GCState {
    let (remark_queue_sender, remark_queue_receiver) = mpsc::channel();
    let inner_state = Arc::new(GCInnerState {
      gc_lock: RwLock::new(()),
      owner,
      params,
      remark_queue_sender,
      
      run_state: Mutex::new(GCRunState::Paused),
      run_state_changed_event: Condvar::new(),
      
      activate_load_barrier: AtomicBool::new(false),
      
      command: Mutex::new(GCCommandStruct {
        command: None,
        execute_count: 0,
        submit_count: 0
      }),
      cmd_executed_event: Condvar::new()
    });
    
    let private_data = GCThreadPrivate {
      remark_queue_receiver
    };
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
          Self::gc_poll(&inner, &heap, &private_data);
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
  
  fn do_mark(&self, heap: &HeapState, obj: &Object) {
    let mut queue = Vec::new();
    queue.push(obj as *const Object);
    
    while let Some(obj) = queue.pop() {
      // SAFETY: It is reachable by GC and GC controls
      // the lifetime of it, so if it reachs here, then
      // its guaranteed to be alive
      let obj = unsafe { &*obj };
      obj.set_mark_bit(&heap.object_manager);
      obj.trace(|reference| {
        queue.push(reference.load(Ordering::Relaxed));
      });
    }
  }
  
  fn run_gc_internal(&self, heap: &HeapState, private: &GCThreadPrivate) {
    // Step 1 (STW): Take root snapshot and take objects in heap snapshot
    let mut block_mutator_cookie = self.block_mutators();
    
    let mut root_snapshot = Vec::new();
    // SAFETY: Just blocked the mutators
    unsafe { heap.take_root_snapshot_unlocked(&mut root_snapshot) };
    let sweeper = heap.object_manager.create_sweeper(&mut block_mutator_cookie);
    
    // Step 1.1: Flip the new marked bit value, so that mutator by default
    // creates new objects which is "marked" to GC perspective
    heap.object_manager.flip_new_marked_bit_value();
    
    // Step 1.2: Active load barrier so mutator can start assisting GC
    // during mark process
    self.inner_state.activate_load_barrier.store(false, Ordering::Relaxed);
    drop(block_mutator_cookie);
    
    // Step 2 (Concurrent): Mark objects
    for obj in root_snapshot {
      // SAFETY: Object is reference from root that mean
      // mutator still using it therefore GC must keep it alive
      let obj = unsafe { &*obj };
      
      // Mark it
      self.do_mark(heap, obj);
    }
    
    // Step 2 (STW): Final remark (to catchup with potentially missed objects)
    // TODO: Move this into independent thread executing along with normal mark
    // so to keep this final remark time to be as low as just signaling that thread
    // and wait that thread
    let block_mutator_cookie = self.block_mutators();
    
    // Step 2.1: Deactivate load barrier, GC does not need mutator assistant anymore
    self.inner_state.activate_load_barrier.store(false, Ordering::Relaxed);
    
    for obj in private.remark_queue_receiver.try_iter() {
      // SAFETY: Object is was loaded by mutator therefore
      // it must be alive at this point so safe
      let obj = unsafe { &*obj.0 };
      
      // Unmark it, so the code for marking can be shared
      // for non final remark and normal mark, because both
      // is exactly the same except that in here it started
      // as marked, so unmark it to remark it later
      obj.unset_mark_bit(&heap.object_manager);
      
      // Mark it
      self.do_mark(heap, obj);
    }
    drop(block_mutator_cookie);
    
    // Step 3 (Concurrent): Sweep dead objects and reset mark flags 
    // SAFETY: just marked live objects and dead objects
    // is well dead
    unsafe { sweeper.sweep_and_reset_mark_flag() };
    
    // Step 4 (STW): Finalizations of various stuffs
    let block_mutator_cookie = self.block_mutators();
    
    // Flip the meaning of marked bit value, so on next cycle GC sees new
    // objects which was looking like "marked" to current cycle to be
    // "unmarked"
    heap.object_manager.flip_marked_bit_value();
    drop(block_mutator_cookie);
  }
}

