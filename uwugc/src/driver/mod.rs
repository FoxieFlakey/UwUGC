
pub mod stat_collector;

use std::{cmp, marker::PhantomData, time::Duration};

use stat_collector::StatItem;

use crate::{allocator::HeapAlloc, heap::State as HeapState};

pub enum Action {
  RunGC,
  Pass,
  DoNothing
}

pub trait Driver<A: HeapAlloc>: Send + 'static {
  fn poll(&mut self, heap: &HeapState<A>, driver_tick_period: Duration, stat: Option<&StatItem>) -> Action;
}

pub struct SimpleDriver<A: HeapAlloc, F: (FnMut(&HeapState<A>, Duration, Option<&StatItem>) -> Action) + Send + 'static> {
  func: F,
  _phantom: PhantomData<A>
}

impl<A: HeapAlloc + Sync, F: (FnMut(&HeapState<A>, Duration, Option<&StatItem>) -> Action) + Send + 'static> SimpleDriver<A, F> {
  pub fn new(func: F) -> Box<dyn Driver<A>> {
    Box::new(Self {
      _phantom: PhantomData {},
      func
    })
  }
}

impl<A: HeapAlloc, F: (FnMut(&HeapState<A>, Duration, Option<&StatItem>) -> Action) + Send + 'static> Driver<A> for SimpleDriver<A, F> {
  fn poll(&mut self, heap: &HeapState<A>, driver_tick_period: Duration, stat_item: Option<&StatItem>) -> Action {
    (self.func)(heap, driver_tick_period, stat_item)
  }
}

// Create default list of drivers to be used
pub fn drivers_list<A: HeapAlloc>() -> Vec<Box<dyn Driver<A>>> {
  // Driver will trigger GC on every 10%
  // up to 70% of heap occupancy for atleast
  // 5 times
  let warm_count = 5.0;
  let warming_usage_steps = 0.1;
  let max_warming_usage = 0.7;
  
  let mut current_warm_count = 0.0;
  
  Vec::from([
    // First executed
    // Warmup driver make sure so there cycles data
    SimpleDriver::new(move |_, _, stat| {
      // If 'stat' not present do nothing and try again at next poll
      let Some(stat) = stat else { return Action::DoNothing };
      let not_enough_data = stat.average_cycle_stats.is_none();
      
      // Atleast trigger GC warm_count times
      // and trigger more as necessary
      if current_warm_count < warm_count || not_enough_data {
        let threshold = f64::clamp((current_warm_count + 1.0) * warming_usage_steps, 0.0, max_warming_usage);
        let usage_percent = stat.heap_usage / stat.heap_size;
        
        if usage_percent >= threshold {
          current_warm_count += 1.0;
          return Action::RunGC;
        }
        
        // Tell GC to not trigger other driver yet
        return Action::DoNothing;
      }
      
      // Warm enough and have data
      Action::Pass
    }),
    
    // Last executed
    SimpleDriver::new(|_, driver_tick_period, stat| {
      let stat = stat.unwrap();
      let cycle_stats = stat.average_cycle_stats.unwrap();
      
      let time_to_oom = (stat.heap_size - stat.heap_usage) / (stat.alloc_rate + 1.0);
      // Make GC aggressiveness based on used percent
      // the more space is used the more aggresive GC is
      
      // TODO: Document what those knobs does
      // let aggresiveness_scale = 3.0;
      // let aggresiveness_base = 0.4;
      // let aggresiveness_exponent = 2.2;
      // let aggresiveness_offset = 0.3;
      
      // Alternative version
      let aggresiveness_offset = 0.26;
      let aggresiveness_base = -0.6;
      let aggresiveness_scale = 2.25;
      
      // let aggresiveness = (stat.heap_usage / stat.heap_size + aggresiveness_offset).powf(aggresiveness_exponent) * aggresiveness_scale + aggresiveness_base;
      let aggresiveness = (stat.heap_usage / stat.heap_size + aggresiveness_offset) * aggresiveness_scale + aggresiveness_base;
      
      // Set lower bound on cycle time to be driver_tick_period
      // because driver might not have enough time to react later
      let cycle_time = cmp::max(driver_tick_period, cycle_stats.cycle_time);
      
      if Duration::from_secs_f64(time_to_oom * f64::clamp(1.0 - aggresiveness, 0.0, 1.0)) <= cycle_time {
        return Action::RunGC;
      }
      
      Action::Pass
    })
  ])
}

