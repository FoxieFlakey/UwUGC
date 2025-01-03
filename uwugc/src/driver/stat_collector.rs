// This collect various statistics for driver to
// better aid in its decision

use std::{cell::LazyCell, sync::{Arc, Weak}, thread::{self, JoinHandle}, time::Duration};

use bounded_vec_deque::BoundedVecDeque;
use parking_lot::{Condvar, Mutex};

use crate::{allocator::HeapAlloc, heap::State as HeapState, objects_manager, CycleStatSum};

#[derive(Clone, Copy)]
pub struct StatItem {
  pub alloc_rate: f64,
  pub dealloc_rate: f64,
  
  pub heap_size: f64,
  pub heap_usage: f64,
  
  pub average_cycle_stats: Option<CycleStatSum>
}

pub struct Parameter {
  pub update_period: Duration,
  pub window_size: usize
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum RunState {
  Paused,
  Unpaused,
  Stopped
}

struct InnerState {
  run_state: Mutex<RunState>,
  wakeup_count: Mutex<u64>,
  wakeup: Condvar,
  param: Parameter,
  
  latest_stat: Mutex<Option<StatItem>>
}

pub struct StatCollector {
  inner_state: Arc<InnerState>,
  thrd: Mutex<Option<JoinHandle<()>>>
}

impl StatCollector {
  pub fn shutdown_and_wait(&self) {
    self.set_run_state(RunState::Stopped);
    self.thrd.lock().take().unwrap().join().unwrap();
  }
  
  fn set_run_state(&self, new_state: RunState) {
    let mut run_state = self.inner_state.run_state.lock();
    if *run_state == new_state {
      return;
    }
    
    assert_ne!(*run_state, RunState::Stopped, "Stat collector is already stopped cannot start it again");
    
    *run_state = new_state;
    drop(run_state);
    
    // There will be only one thread so its fine
    *self.inner_state.wakeup_count.lock() += 1;
    self.inner_state.wakeup.notify_one();
  }
  
  pub fn unpause(&self) {
    self.set_run_state(RunState::Unpaused);
  }
  
  pub fn get_stat(&self) -> Option<StatItem> {
    *self.inner_state.latest_stat.lock()
  }
  
  pub fn new<A: HeapAlloc>(heap: Weak<HeapState<A>>, param: Parameter) -> Self {
    let state = Arc::new(InnerState {
      run_state: Mutex::new(RunState::Paused),
      latest_stat: Mutex::new(None),
      wakeup: Condvar::new(),
      wakeup_count: Mutex::new(0),
      param
    });
    let wakeup = Arc::new(Condvar::new());
    
    Self {
      inner_state: state.clone(),
      thrd: Mutex::new(Some(thread::spawn(move || {
        let heap = LazyCell::new(|| heap.upgrade().unwrap());
        
        let mut window = BoundedVecDeque::<StatItem>::new(state.param.window_size);
        let mut prev_total_alloc = 0;
        let mut prev_total_dealloc = 0;
        
        let mut capture_stat = || {
          let rate_multiplier = Duration::from_secs(1).as_secs_f64() / state.param.update_period.as_secs_f64();
          
          let objects_manager::Stats {
            alloc_success_bytes: current_total_alloc,
            dealloc_bytes: current_total_dealloc,
            ..
          } = heap.object_manager.get_lifetime_stats();
          
          let stat = StatItem {
            heap_size: heap.object_manager.get_max_size() as f64,
            heap_usage: heap.object_manager.get_usage() as f64,
            alloc_rate: (current_total_alloc - prev_total_alloc) as f64 * rate_multiplier,
            dealloc_rate: (current_total_dealloc - prev_total_dealloc) as f64 * rate_multiplier,
            
            average_cycle_stats: None
          };
          
          prev_total_alloc = current_total_alloc;
          prev_total_dealloc = current_total_dealloc;
          
          stat
        };
        
        'poll_loop: loop {
          // Wait until something wakes up stat collector
          let mut wakeup_count = state.wakeup_count.lock();
          let current_count = *wakeup_count;
          while current_count == *wakeup_count {
            // Timeout reached do other stuffs
            if wakeup.wait_for(&mut wakeup_count, state.param.update_period).timed_out() {
              break;
            }
          }
          drop(wakeup_count);
          
          // Check for run state update
          let run_state = state.run_state.lock();
          match *run_state {
            RunState::Paused => continue 'poll_loop,
            RunState::Unpaused => (),
            RunState::Stopped => break 'poll_loop,
          }
          drop(run_state);
          
          // On first unpause, eagerly upgrade the heap pointer from weak to strong
          LazyCell::force(&heap);
          
          let new_item = capture_stat();
          window.push_back(new_item);
          
          let mut averaged_stat: Option<StatItem> = None;
          
          for item in &window {
            averaged_stat = Some(averaged_stat.map_or_else(|| item.clone(), |curr| {
              StatItem {
                alloc_rate: curr.alloc_rate + item.alloc_rate,
                dealloc_rate: curr.dealloc_rate + item.dealloc_rate,
                heap_size: curr.heap_size + item.heap_size,
                heap_usage: curr.heap_usage + item.heap_usage,
                
                // Will be computed later
                average_cycle_stats: None
              }
            }));
          }
          
          // Average all the data
          let item_count = window.len() as f64;
          let averaged_stat = averaged_stat.map(|x| {
            StatItem {
              alloc_rate: x.alloc_rate / item_count,
              dealloc_rate: x.dealloc_rate / item_count,
              heap_size: x.heap_size / item_count,
              heap_usage: x.heap_usage / item_count,
              average_cycle_stats: None
            }
          });
          
          let averaged_stat = averaged_stat.map(|x| {
            // Now calculate the cycle times
            let gc_stats = heap.get_gc_stats();
            let cycle_count = u64::try_from(gc_stats.history.len()).unwrap();
            let average_cycle_stat = gc_stats.history
              .iter()
              
              // Create sum of everything
              .map(|x| CycleStatSum::from(*x))
              .reduce(|acc, curr| {
                acc + curr
              })
              
              // Then average
              .map(|sum| {
                CycleStatSum {
                  steps_time: Default::default(),
                  
                  cycle_time: sum.cycle_time.div_f64(cycle_count as f64),
                  stw_time: sum.stw_time.div_f64(cycle_count as f64),
                  total_bytes: sum.total_bytes / cycle_count,
                  dead_bytes: sum.dead_bytes / cycle_count,
                  live_bytes: sum.live_bytes / cycle_count,
                  total_objects: sum.total_objects / cycle_count,
                  dead_objects: sum.dead_objects / cycle_count,
                  live_objects: sum.live_objects / cycle_count,
                }
              });
            
            StatItem {
              average_cycle_stats: average_cycle_stat,
              ..x
            }
          });
          
          // Someone else tries to access it but it is always recalculated
          // so its fine if didn't get the lock
          if let Some(mut guard) = state.latest_stat.try_lock() {
            *guard = averaged_stat;
          }
        }
        
        println!("Quiting stat collector...");
      })))
    }
  }
}

