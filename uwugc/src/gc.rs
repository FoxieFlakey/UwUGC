use std::{cell::LazyCell, ops::{Add, AddAssign}, ptr::{self, NonNull}, sync::{atomic::Ordering, mpsc, Arc, Weak}, thread::{self, JoinHandle}, time::{Duration, Instant}};
use crate::{allocator::HeapAlloc, driver::{self, stat_collector::{Parameter, StatCollector}, Action as DriverAction, Driver}};
use bounded_vec_deque::BoundedVecDeque;
use parking_lot::{Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use portable_atomic::AtomicBool;

use crate::{heap::State as HeapState, objects_manager::{Object, ObjectManager}};

// NOTE: This is considered public API
// therefore be careful with breaking changes
#[derive(Clone)]
pub struct GCParams {
  pub trigger_size: usize,
  pub poll_rate: u64,
  pub cycle_stats_history_size: usize
}

// NOTE: This is considered public API
// therefore be careful with breaking changes
#[derive(Copy, Clone)]
pub struct CycleStat {
  pub cycle_time: Duration,
  pub stw_time: Duration,
  pub steps_time: [Duration; 5],
  pub reason: GCRunReason,
  
  pub total_bytes: u64,
  pub dead_bytes: u64,
  pub live_bytes: u64,
  
  pub total_objects: u64,
  pub dead_objects: u64,
  pub live_objects: u64
}

impl Add for CycleStat {
  type Output = CycleStatSum;
  
  fn add(self, rhs: Self) -> Self::Output {
    CycleStatSum::from(self) + CycleStatSum::from(rhs)
  }
}

// NOTE: This is considered public API
// therefore be careful with breaking changes
#[derive(Copy, Clone, Default)]
pub struct CycleStatSum {
  pub cycle_time: Duration,
  pub stw_time: Duration,
  pub steps_time: [Duration; 5],
  
  pub total_bytes: u64,
  pub dead_bytes: u64,
  pub live_bytes: u64,
  
  pub total_objects: u64,
  pub dead_objects: u64,
  pub live_objects: u64
}

impl From<CycleStat> for CycleStatSum {
  fn from(x: CycleStat) -> Self {
    Self {
      cycle_time: x.cycle_time,
      stw_time: x.stw_time,
      steps_time: x.steps_time,
      dead_bytes: x.dead_bytes,
      total_bytes: x.total_bytes,
      live_bytes: x.live_bytes,
      dead_objects: x.dead_objects,
      total_objects: x.total_objects,
      live_objects: x.live_objects
    }
  }
}

impl Add for CycleStatSum {
  type Output = CycleStatSum;
  
  fn add(self, rhs: Self) -> Self::Output {
    let mut tmp = Self {
      cycle_time: self.cycle_time + rhs.cycle_time,
      stw_time: self.stw_time + rhs.stw_time,
      steps_time: self.steps_time,
      dead_bytes: self.dead_bytes + rhs.dead_bytes,
      total_bytes: self.total_bytes + rhs.total_bytes,
      live_bytes: self.live_bytes + rhs.live_bytes,
      dead_objects: self.dead_objects + rhs.dead_objects,
      total_objects: self.total_objects + rhs.total_objects,
      live_objects: self.live_objects + rhs.live_objects
    };
    tmp.steps_time.iter_mut()
      .zip(rhs.steps_time.iter())
      .for_each(|(lhs, rhs)| *lhs += *rhs);
    tmp
  }
}

impl Add<CycleStat> for CycleStatSum {
  type Output = CycleStatSum;
  
  fn add(self, rhs: CycleStat) -> Self::Output {
    self + Self::from(rhs)
  }
}

impl AddAssign<CycleStat> for CycleStatSum {
  fn add_assign(&mut self, rhs: CycleStat) {
    *self = *self + rhs;
  }
}

// NOTE: This is considered public API
// therefore be careful with breaking changes
#[derive(Clone)]
pub struct GCStats {
  // Use this to detect change, GC always increment
  // this when updating this
  pub sequence_id: u64,
  
  // New CycleStat inserted for every cycle executed by GC
  // and lifetime_sum updated on every cycle
  pub history: BoundedVecDeque<CycleStat>,
  pub lifetime_sum: CycleStatSum,
  pub lifetime_cycle_count: u64
}

// Reasons of why GC is started
// application cannot directly pass
// these reasons, they always treated as
// Explicit GCs
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GCRunReason {
  // Memory has ran out!
  OutOfMemory,
  
  // User has called the GC perhaps the user know that it
  // in low period activity where GC wouldn't able to interfere
  //
  // GC is free to ignore this, its a suggestion
  Explicit,
  
  // An algorithmn tried to trigger the GC
  Proactive,
  
  // Shutdown trigger the final cycle clearing everything
  Shutdown
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GCCommand {
  RunGC(GCRunReason)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GCRunState {
  Paused,
  Running,
  Stopped
}

// NOTE: This is considered public API
// therefore be careful with breaking changes
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CycleStep {
  SATB,
  ConcMark,
  FinalRemark,
  ConcSweep,
  Finalize
}

// NOTE: This is considered public API
// therefore be careful with breaking changes
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CycleInfo {
  pub step: CycleStep,
  pub reason: GCRunReason
}

// NOTE: This is considered public API
// therefore be careful with breaking changes
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CycleState {
  Running(CycleInfo),
  Idle
}

#[derive(Clone)]
struct GCCommandStruct {
  command: Option<GCCommand>,
  submit_count: u64,
  execute_count: u64
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ObjectPtrSend(NonNull<Object>);
impl From<NonNull<Object>> for ObjectPtrSend {
  fn from(value: NonNull<Object>) -> Self {
    Self(value)
  }
}

// SAFETY: Its safe just need a wrapper
unsafe impl Send for ObjectPtrSend {}

struct GCInnerState<A: HeapAlloc> {
  gc_lock: RwLock<()>,
  owner: Weak<HeapState<A>>,
  params: GCParams,
  
  run_state: Mutex<GCRunState>,
  wakeup: Condvar,
  
  // GC will regularly checks this and execute command
  // then wake other
  command: Mutex<GCCommandStruct>,
  cmd_executed_event: Condvar,
  
  // Whether or not to active load barrier
  activate_load_barrier: AtomicBool,
  
  // Remark queue for unmarked object
  // encountered by the load barrier
  remark_queue_sender: mpsc::Sender<ObjectPtrSend>,
  
  // GC statistics
  stats: Mutex<GCStats>,
  
  // State of the cycle
  cycle_state: Mutex<CycleState>,
  
  stat_collector: StatCollector,
  drivers: Mutex<Vec<Box<dyn Driver<A>>>>
}

pub struct GCState<A: HeapAlloc> {
  inner_state: Arc<GCInnerState<A>>,
  thread: Mutex<Option<JoinHandle<()>>>
}

pub struct GCLockCookie<'a, A: HeapAlloc> {
  owner: &'a GCState<A>,
  _cookie: RwLockReadGuard<'a, ()>
}

impl<A: HeapAlloc> GCLockCookie<'_, A> {
  pub fn get_gc(&self) -> &GCState<A> {
    self.owner
  }
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

impl<A: HeapAlloc> GCState<A> {
  pub fn shutdown_gc_and_wait(&self) {
    self.set_gc_run_state(GCRunState::Stopped);
    if let Some(join_handle) = self.thread.lock().take() {
      // If current thread is not GC, then wait for GC
      // if GC then ignore it
      if join_handle.thread().id() != thread::current().id() {
        join_handle.join().unwrap();
      }
    }
    self.inner_state.stat_collector.shutdown_and_wait();
  }
  
  pub fn get_params(&self) -> GCParams {
    self.inner_state.params.clone()
  }
  
  pub fn get_gc_stats(&self) -> GCStats {
    self.inner_state.stats.lock().clone()
  }
  
  // Panics if GC is already stopped (its only
  // one way hatch to be stopped, mainly for clean
  // up code)
  fn set_gc_run_state(&self, state: GCRunState) {
    let mut state_ref = self.inner_state.run_state.lock();
    // Trying to ask GC to change into same state
    // does nothing
    if *state_ref == state {
      return;
    }
    
    assert!(*state_ref != GCRunState::Stopped, "GC is stopped (or in process of stopping)! Cannot change GC run state anymore");
    
    *state_ref = state;
    
    // Notify the GC of run state change
    self.wakeup_gc();
  }
  
  // Wait for any currently executing command to be completed
  fn wait_for_gc<'a>(&'a self, submit_count: Option<u64>, cmd_control: Option<MutexGuard<'a, GCCommandStruct>>) -> MutexGuard<'a, GCCommandStruct> {
    let mut cmd_control = cmd_control
      .or_else(|| Some(self.inner_state.command.lock()))
      .unwrap();
    
    let submit_count = submit_count
      .unwrap_or(cmd_control.submit_count);
    
    while submit_count > cmd_control.execute_count {
      self.inner_state.cmd_executed_event.wait(&mut cmd_control);
    }
    cmd_control
  }
  
  fn wakeup_gc(&self) {
    // there will be only one primary thread so notify_one is better choice
    self.inner_state.wakeup.notify_one();
  }
  
  fn call_gc(&self, cmd: GCCommand) {
    let mut cmd_control = self.inner_state.command.lock();
    if let Some(current_cmd) = cmd_control.command {
      let combine_command =  match (&current_cmd, &cmd) {
        // GCCommand::RunGC can be combined with potentially
        // in progress RunGC command because there no need
        // to fire multiple GCs in a row because multiple
        // thread coincidentally attempt to do that when one
        // is really enough until next time
        
        (GCCommand::RunGC(..), GCCommand::RunGC(..)) => true,
      };
      
      if combine_command {
        drop(self.wait_for_gc(None, Some(cmd_control)));
        return;
      }
    }
    
    // Wakeup GC to respond to changes
    self.wakeup_gc();
    
    // Wait for any previous command to be executed
    cmd_control = self.wait_for_gc(None, Some(cmd_control));
    
    // There must not be any command in execution, GC replace it
    // with None after completing a command
    assert!(cmd_control.command.is_none());
    cmd_control.submit_count += 1;
    cmd_control.command = Some(cmd);
    
    // Wait for current command to be executed
    drop(self.wait_for_gc(Some(cmd_control.submit_count), Some(cmd_control)));
  }
  
  fn process_command(gc_state: &Arc<GCInnerState<A>>, heap: &HeapState<A>, cmd_struct: &GCCommandStruct, private: &GCThreadPrivate) {
    match cmd_struct.command.unwrap() {
      GCCommand::RunGC(reason) => {
        heap.gc.run_gc_internal(heap, reason, private);
      }
    }
    
    let mut cmd_control = gc_state.command.lock();
    cmd_control.command = None;
    cmd_control.execute_count += 1;
    drop(cmd_control);
    
    gc_state.cmd_executed_event.notify_all();
  }
  
  fn do_gc_heuristics(gc_state: &Arc<GCInnerState<A>>, heap: &HeapState<A>, cmd_control: &mut GCCommandStruct) {
    let stat = gc_state.stat_collector.get_stat();
    
    let mut decision = DriverAction::Pass;
    for drv in gc_state.drivers.lock().iter_mut() {
      decision = drv.poll(heap, stat.as_ref());
      
      // RunGC action short circuits
      if let DriverAction::RunGC = decision {
        break;
      }
    }
    
    if let DriverAction::RunGC = decision {
      cmd_control.submit_count += 1;
      cmd_control.command = Some(GCCommand::RunGC(GCRunReason::Proactive));
    }
  }
  
  fn gc_poll(inner: &Arc<GCInnerState<A>>, heap: &HeapState<A>, private: &GCThreadPrivate) {
    let mut cmd_control = inner.command.lock();
    
    // If there no command to be executed
    // let 'do_gc_heuristics' decide what
    // to do next based on heuristics, it
    // may injects a new command into cmd_control
    // to send command, instead relying other
    // if statement and special conditions
    if cmd_control.command.is_none() {
      Self::do_gc_heuristics(inner, heap, &mut cmd_control);
    }
    
    let cmd_struct = cmd_control.clone();
    let cmd = cmd_control.command;
    drop(cmd_control);
    
    if cmd.is_some() {
      Self::process_command(inner, heap, &cmd_struct, private);
    }
  }
  
  pub fn unpause_gc(&self) {
    self.set_gc_run_state(GCRunState::Running);
    self.inner_state.stat_collector.unpause();
  }
  
  pub fn load_barrier(&self, object: &Object, obj_manager: &ObjectManager<A>, _block_gc_cookie: &GCLockCookie<A>) -> bool {
    // Load barrier is deactivated
    if !self.inner_state.activate_load_barrier.load(Ordering::Relaxed) {
      return false;
    }
    
    if object.set_mark_bit(obj_manager) {
      return false;
    }
    
    // SAFETY: References are always non null
    let obj_nonnull = unsafe { NonNull::new_unchecked(ptr::from_ref(object).cast_mut()) };
    
    // SAFETY: This always succeded because the receiver end will
    // only be destroyed if Heap is no longer exist anywhere and there
    // can't be a way this be called as load barrier only can be triggered
    // by Heap existing on mutator code
    unsafe {
      self.inner_state.remark_queue_sender.send(obj_nonnull.into()).unwrap_unchecked();
    }
    true
  }
  
  pub fn new(params: GCParams, owner: Weak<HeapState<A>>) -> GCState<A> {
    let (remark_queue_sender, remark_queue_receiver) = mpsc::channel();
    let inner_state = Arc::new(GCInnerState {
      cmd_executed_event: Condvar::new(),
      
      stats: Mutex::new(GCStats {
        sequence_id: 0,
        history: BoundedVecDeque::new(params.cycle_stats_history_size),
        lifetime_sum: CycleStatSum::default(),
        lifetime_cycle_count: 0
      }),
      
      stat_collector: StatCollector::new(owner.clone(), Parameter {
        update_period: Duration::from_millis(1000 / (params.poll_rate * 2)),
        window_size: params.poll_rate.try_into().unwrap()
      }),
      
      gc_lock: RwLock::new(()),
      owner,
      params,
      remark_queue_sender,
      
      run_state: Mutex::new(GCRunState::Paused),
      wakeup: Condvar::new(),
      
      activate_load_barrier: AtomicBool::new(false),
      
      command: Mutex::new(GCCommandStruct {
        command: None,
        execute_count: 0,
        submit_count: 0
      }),
      cycle_state: Mutex::new(CycleState::Idle),
      drivers: Mutex::new(driver::drivers_list())
    });
    
    let private_data = GCThreadPrivate {
      remark_queue_receiver
    };
    GCState {
      inner_state: inner_state.clone(),
      thread: Mutex::new(Some(thread::spawn(move || {
        let inner = inner_state;
        let sleep_delay_milisec = 1000 / inner.params.poll_rate;
        let heap = LazyCell::new(|| inner.owner.upgrade().unwrap());
        
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
            
            inner.wakeup.wait_for(&mut run_state, Duration::from_millis(sleep_delay_milisec));
          }
          drop(run_state);
          
          // When resumed for first time try get Arc<HeapState> from
          // Weak<HeapState>, GC started as paused because if GC immediately
          // resumes there some part of heap state isn't initialized yet
          LazyCell::force(&heap);
          
          Self::gc_poll(&inner, &heap, &private_data);
        }
        
        println!("Shutting down GC");
        heap.gc.run_gc_internal(&heap, GCRunReason::Shutdown, &private_data);
        println!("Quitting GC");
      })))
    }
  }
  
  pub fn block_gc(&self) -> GCLockCookie<A> {
    GCLockCookie {
      owner: self,
      _cookie: self.inner_state.gc_lock.read()
    }
  }
  
  pub fn block_mutators(&self) -> GCExclusiveLockCookie {
    GCExclusiveLockCookie {
      _cookie: self.inner_state.gc_lock.write()
    }
  }
  
  pub fn run_gc(&self, is_for_oom: bool) {
    if is_for_oom {
      self.call_gc(GCCommand::RunGC(GCRunReason::Explicit));
    } else {
      self.call_gc(GCCommand::RunGC(GCRunReason::OutOfMemory));
    }
  }
  
  // SAFETY: Caller has to make 'obj' is valid
  unsafe fn do_mark(heap: &HeapState<A>, obj: NonNull<Object>) {
    let mut queue = Vec::new();
    queue.push(obj);
    
    while let Some(obj) = queue.pop() {
      // SAFETY: It is reachable by GC and GC controls
      // the lifetime of it, so if it reachs here, then
      // its guaranteed to be alive
      let obj_ref = unsafe { obj.as_ref() };
      if obj_ref.set_mark_bit(&heap.object_manager) {
        // The object is already marked, don't trace it anymore
        continue;
      }
      
      // SAFETY: Objects are in control by GC so objects are valid
      unsafe {
        Object::trace(obj, |reference| {
          if let Some(reference) = reference {
            queue.push(reference);
          }
        });
      }
    }
  }
  
  pub fn get_cycle_state(&self) -> CycleState {
    *self.inner_state.cycle_state.lock()
  }
  
  fn set_cycle_state(&self, new_state: CycleState) {
    *self.inner_state.cycle_state.lock() = new_state;
  }
  
  fn run_gc_internal(&self, heap: &HeapState<A>, reason: GCRunReason, private: &GCThreadPrivate) {
    let cycle_start_time = Instant::now();
    let set_run_step = |step| {
      self.set_cycle_state(CycleState::Running(CycleInfo {
        step,
        reason 
      }));
    };
    
    set_run_step(CycleStep::SATB);
    
    // Step 1 (STW): Take root snapshot and take objects in heap snapshot
    let step1_start = Instant::now();
    let mut block_mutator_cookie = self.block_mutators();
    
    let mut root_snapshot = Vec::new();
    // During shutdown do not trace mutator's root so GC can free everything else
    if !matches!(reason, GCRunReason::Shutdown) {
      // SAFETY: Just blocked the mutators
      unsafe { heap.take_root_snapshot_unlocked(&mut root_snapshot) };
    }
    let sweeper = heap.object_manager.create_sweeper(&mut block_mutator_cookie);
    
    // Step 1.1: Flip the new marked bit value, so that mutator by default
    // creates new objects which is "marked" to GC perspective
    heap.object_manager.flip_new_marked_bit_value();
    
    // Step 1.2: Active load barrier so mutator can start assisting GC
    // during mark process
    self.inner_state.activate_load_barrier.store(true, Ordering::Relaxed);
    drop(block_mutator_cookie);
    let step1_time = step1_start.elapsed();
    
    // Step 2 (Concurrent): Mark objects
    let step2_start = Instant::now();
    set_run_step(CycleStep::ConcMark);
    for obj in root_snapshot {
      // Mark it
      // SAFETY: Object is reference from root that mean
      // mutator still using it therefore GC must keep it alive
      unsafe { Self::do_mark(heap, obj) };
    }
    let step2_time = step2_start.elapsed();
    
    let step3_start = Instant::now();
    set_run_step(CycleStep::FinalRemark);
    // Step 3 (STW): Final remark (to catchup with potentially missed objects)
    // TODO: Move this into independent thread executing along with normal mark
    // so to keep this final remark time to be as low as just signaling that thread
    // and wait that thread
    let block_mutator_cookie = self.block_mutators();
    
    // Step 3.1: Deactivate load barrier, GC does not need mutator assistant anymore
    self.inner_state.activate_load_barrier.store(false, Ordering::Relaxed);
    
    for obj in private.remark_queue_receiver.try_iter() {
      // SAFETY: Object is was loaded by mutator therefore
      // it must be alive at this point so safe
      let obj_ref = unsafe { obj.0.as_ref() };
      
      // Unmark it, so the code for marking can be shared
      // for non final remark and normal mark, because both
      // is exactly the same except that in here it started
      // as marked, so unmark it to remark it later
      obj_ref.unset_mark_bit(&heap.object_manager);
      
      // Mark it
      // SAFETY: GC has control of the objects and because mutator
      // can access it that mean GC must keep it alive
      unsafe { Self::do_mark(heap, obj.0) };
    }
    
    // Step 3.2: Prune descriptor cache from dead descriptors
    // SAFETY: Mutator is being blocked so mutator cannot reference to
    // potential about-to-be swept descriptors and marking process ensure
    // that currently in use descriptors are properly marked
    unsafe { heap.object_manager.prune_descriptor_cache() };
    drop(block_mutator_cookie);
    let step3_time = step3_start.elapsed();
    
    let step4_start = Instant::now();
    set_run_step(CycleStep::ConcSweep);
    
    // Step 4 (Concurrent): Sweep dead objects and reset mark flags 
    // SAFETY: just marked live objects and dead objects
    // is well dead
    let sweep_stats = unsafe { sweeper.sweep_and_reset_mark_flag() };
    let step4_time = step4_start.elapsed();
    
    let step5_start = Instant::now();
    set_run_step(CycleStep::Finalize);
    
    // Step 5 (STW): Finalizations of various stuffs
    let block_mutator_cookie = self.block_mutators();
    
    // Flip the meaning of marked bit value, so on next cycle GC sees new
    // objects which was looking like "marked" to current cycle to be
    // "unmarked"
    heap.object_manager.flip_marked_bit_value();
    drop(block_mutator_cookie);
    let step5_time = step5_start.elapsed();
    
    self.set_cycle_state(CycleState::Idle);
    let cycle_duration = cycle_start_time.elapsed();
    
    let pause_time = step1_time + step3_time + step5_time;
    let stat = CycleStat {
      cycle_time: cycle_duration,
      stw_time: pause_time,
      reason,
      steps_time: [
        step1_time,
        step2_time,
        step3_time,
        step4_time,
        step5_time
      ],
      total_bytes: sweep_stats.total_bytes.try_into().unwrap(),
      dead_bytes: sweep_stats.dead_bytes.try_into().unwrap(),
      live_bytes: sweep_stats.live_bytes.try_into().unwrap(),
      
      total_objects: sweep_stats.total_objects.try_into().unwrap(),
      dead_objects: sweep_stats.dead_objects.try_into().unwrap(),
      live_objects: sweep_stats.live_objects.try_into().unwrap()
    };
    
    let mut stats = self.inner_state.stats.lock();
    stats.sequence_id += 1;
    stats.lifetime_cycle_count += 1;
    stats.lifetime_sum += stat;
    stats.history.push_back(stat);
  }
}

