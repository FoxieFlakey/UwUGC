#![feature(file_buffered)]

#![allow(clippy::needless_return)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::{fs::File, hint::black_box, io::{self, Write}, mem::MaybeUninit, sync::atomic::Ordering, thread, time::{Duration, Instant}};

use std::sync::atomic::AtomicBool;

use tabled::{settings::Style, Table, Tabled};
use uwugc::{CycleState, CycleStep, GCParams, GCRunReason, GCStats, HeapArc, HeapStats, Params};

static QUIT_THREADS: AtomicBool = AtomicBool::new(false);
const MAX_SIZE: usize = 768 * 1024 * 1024;
const POLL_RATE: u32 = 20;

#[cfg(not(miri))]
mod non_miri;

// Information of heap at a point
#[derive(Clone)]
#[allow(dead_code)]
struct HeapStatRecord {
  max_size: usize,
  usage: usize,
  trigger_size: usize
}

fn main() {
  println!("Hello, world!");
  #[cfg(not(miri))]
  non_miri::prepare_mimalloc();
  
  let heap = HeapArc::new(Params {
    gc_params: GCParams {
      poll_rate: POLL_RATE,
      cycle_stats_history_size: 20
    },
    max_size: MAX_SIZE
  });
  
  // A closure for a test
  let ctx = heap.create_context();
  let test = || {
    for _ in 0..1_000_000 {
      const SZ: usize = 2 * 1024 * 1024;
      let mut a = unsafe { black_box(ctx.alloc2(|_, uninit: &mut MaybeUninit<[u8; SZ]>| {
          uninit.as_mut_ptr().write_bytes(0x00, size_of_val(uninit));
        })) };
      let mut data = unsafe { a.borrow_inner_mut() };
      data.fill(black_box(123));
      data = black_box(data);
      for byte in data.iter_mut() {
        *byte += *byte;
      }
      black_box(data);
    }
    
    // Ported from Java version of gc-latency-experiment
    // https://github.com/WillSewell/gc-latency-experiment/blob/f67121ec8a741201414c76d5ba85f9304c774acc/java/Main.java
  };
  // test();
  println!("Warmup completed!");
  
  let start = Instant::now();
  let stat_thread = {
    if true {
      let heap = heap.clone();
      let mut stats_file = File::create_buffered("data.csv").unwrap();
      writeln!(&mut stats_file, "Time,Usage,Heap Size,GC Activity").unwrap();
      Some(
        thread::spawn(move || {
          let mut prev_heap_stats = HeapStats::default();
          let mut prev_usage = 0 as f32;
          
          let rate_update_speed = 1 as f32;
          let rate_conversion_factor = rate_update_speed;
          
          let poll_rate = 10 as f32;
          
          let mut deadline_for_rate_update = Instant::now() + Duration::from_secs_f32(1.0 / rate_update_speed);
          let mut gc_rate = 0.0;
          let mut alloc_rate = 0.0;
          let mut growth = 0.0;
          
          while !QUIT_THREADS.load(Ordering::Relaxed) {
            let usage = heap.get_usage();
            let usage = (usage as f32) / 1024.0 / 1024.0;
            let max_size = (MAX_SIZE as f32) / 1024.0 / 1024.0;
            let info = match heap.get_cycle_state() {
              CycleState::Idle => None,
              CycleState::Running(info) => Some(info)
            };
            let (cycle_activity, state_id) = match info.map(|x| x.step) {
              None                         => ("Idle   (           )", 0),
              Some(CycleStep::SATB)        => ("Active (SATB       )", 1),
              Some(CycleStep::ConcMark)    => ("Active (ConcMark   )", 2),
              Some(CycleStep::FinalRemark) => ("Active (FinalRemark)", 3),
              Some(CycleStep::ConcSweep)   => ("Active (ConcSweep  )", 4),
              Some(CycleStep::Finalize)    => ("Active (Finalize   )", 5)
            };
            let reason = match info.map(|x| x.reason) {
              None                            => "None",
              Some(GCRunReason::Explicit)     => "Explicit",
              Some(GCRunReason::OutOfMemory)  => "Out of memory",
              Some(GCRunReason::Proactive)    => "Proactive",
              Some(GCRunReason::Shutdown)     => "Shutting down"
            };
            let timestamp = start.elapsed().as_secs_f32();
            
            if deadline_for_rate_update < Instant::now() {
              let current_heap_stats = heap.get_lifetime_heap_stats();
              let alloc_rate_bytes = current_heap_stats.alloc_success_bytes as f32 - prev_heap_stats.alloc_success_bytes as f32;
              let gc_rate_bytes = current_heap_stats.dealloc_bytes as f32 - prev_heap_stats.dealloc_bytes as f32;
              gc_rate = (gc_rate_bytes / 1024.0 / 1024.0) * rate_conversion_factor;
              alloc_rate = (alloc_rate_bytes / 1024.0 / 1024.0) * rate_conversion_factor;
              growth = usage - prev_usage;
              
              prev_heap_stats = current_heap_stats;
              prev_usage = usage;
              
              deadline_for_rate_update += Duration::from_secs_f32(1.0 / rate_update_speed);
            }
            
            let growth_direction = if growth.is_sign_positive() { "+" } else { "-" };
            let growth_abs = growth.abs();
            
            writeln!(&mut stats_file, "{timestamp},{usage},{max_size},{state_id}").unwrap();
            
            print!("\r\x1b[3A");
            
            print!("\x1b[2K");
            println!("Usage  : {usage: >8.2} MiB   Max: {max_size: >8.2} MiB   GC: {cycle_activity}");
            
            print!("\x1b[2K");
            println!("GC Reason: {reason}");
            
            print!("\x1b[2K");
            println!("GC Rate: {gc_rate: >8.2} MiB/s Alloc Rate: {alloc_rate: >8.2} MiB/s Growth: {growth_direction}{growth_abs: >7.2} MiB/s");
            
            io::stdout().flush().unwrap();
            
            thread::sleep(Duration::from_secs_f32(1.0 / poll_rate));
          }
          
          println!();
          io::stdout().flush().unwrap();
          stats_file.flush().unwrap();
        })
      )
    } else {
      None
    }
  };
  
  // Raw is 1.5x faster than GC
  let start_time = Instant::now();
  test();
  let complete_time = (start_time.elapsed().as_millis() as f32) / 1024.0;
  let gc_stats = heap.get_gc_stats();
  drop(ctx);
  
  QUIT_THREADS.store(true, Ordering::Relaxed);
  println!();
  println!("Shutting down!");
  
  if let Some(thrd) = stat_thread {
    thrd.join().unwrap();
  }
  
  println!("Test time was {complete_time:.2} secs");
  println!("GC statistics:");
  let GCStats {
    lifetime_cycle_count,
    lifetime_sum,
    history,
    ..
  } = gc_stats;
  
  let cycle_time = lifetime_sum.cycle_time.as_secs_f32() * 1000.0;
  let stw_time = lifetime_sum.stw_time.as_secs_f32() * 1000.0;
  let steps_time = lifetime_sum.steps_time
    .map(|time| time.as_secs_f32() * 1000.0);
  let scanned_bytes = (lifetime_sum.total_bytes as f32) / 1024.0 / 1024.0;
  let fatality_rate = lifetime_sum.dead_bytes as f32 / lifetime_sum.total_bytes as f32 * 100.0;
  
  let cycle_time_avg = cycle_time / lifetime_cycle_count as f32;
  let stw_time_avg = stw_time / lifetime_cycle_count as f32;
  let steps_time_avg = steps_time
    .map(|time| time / lifetime_cycle_count as f32);
  let scanned_bytes_avg = scanned_bytes / lifetime_cycle_count as f32;
  let fatality_rate_avg = fatality_rate / lifetime_cycle_count as f32;
  
  println!("History of {:} recent cycles:", history.len());
  #[derive(Tabled)]
  struct CycleEntry {
    id            : String,
    scanned       : String,
    fatality      : String,
    time          : String,
    stw           : String,
    satb          : String,
    conc_mark     : String,
    final_remark  : String,
    conc_sweep    : String,
    finalize      : String
  }
  
  let table_content = history.as_unbounded()
    .iter()
    .rev()
    .enumerate()
    .map(|(id, cycle)| CycleEntry {
      id            : format!("{:>2}", id + 1),
      time          : format!("{:>8.3} ms", cycle.cycle_time   .as_secs_f32() * 1000.0),
      stw           : format!("{:>8.3} ms", cycle.stw_time     .as_secs_f32() * 1000.0),
      satb          : format!("{:>8.3} ms", cycle.steps_time[0].as_secs_f32() * 1000.0),
      conc_mark     : format!("{:>8.3} ms", cycle.steps_time[1].as_secs_f32() * 1000.0),
      final_remark  : format!("{:>8.3} ms", cycle.steps_time[2].as_secs_f32() * 1000.0),
      conc_sweep    : format!("{:>8.3} ms", cycle.steps_time[3].as_secs_f32() * 1000.0),
      finalize      : format!("{:>8.3} ms", cycle.steps_time[4].as_secs_f32() * 1000.0),
      scanned       : format!("{:>8.2} MiB", (cycle.total_bytes as f32) / 1024.0 / 1024.0),
      fatality      : format!("{:>6.2}%", cycle.dead_bytes as f32 / cycle.total_bytes as f32 * 100.0)
    });
  
  let mut table = Table::new(table_content);
  table.with(Style::rounded());
  let table = table.to_string();
  
  println!("{table}");
  
  println!("Total cycles      count: {lifetime_cycle_count:>12} cycles");
  println!("Total cycle        time: {cycle_time:>12.3} ms  ({cycle_time_avg:>12.3} ms  average per cycle contribution)");
  println!("Total STW          time: {stw_time:>12.3} ms  ({stw_time_avg:>12.3} ms  average per cycle contribution)");
  println!("Total scanned     bytes: {scanned_bytes:>11.2}  MiB ({scanned_bytes_avg:>11.2}  MiB average per cycle contribution)");
  println!("Total dead   percentage: {fatality_rate:>11.2}  %   ({fatality_rate_avg:>11.2}  %   average per cycle contribution)");
  
  let step_names = [
    "SATB",
    "ConcMark",
    "FinalRemark",
    "ConcSweep",
    "Finalize"
  ];
  steps_time.iter()
    .zip(steps_time_avg.iter())
    .enumerate()
    .for_each(|(mut i, (total, avg))| {
      i += 1;
      println!("Total {:<12} time: {total:>12.3} ms  ({avg:>12.3} ms  average per cycle contribution)", step_names[i - 1]);
    });
  println!("Quitting :3");
  drop(heap);
}
