use std::{
    convert::identity,
    time::{Duration, Instant},
};

use parking_lot::{Condvar, Mutex};

pub struct Event {
    counter: Mutex<u64>,
    condvar: Condvar,
}

#[derive(Clone, Default)]
pub struct EventCookie {
    count: u64,
}

pub enum Timeout {
    // Wait forever
    Infinite,
    // Wait for specified duration
    Duration(Duration),
    // Do not wait, just poll
    NoWait,
}

impl Event {
    pub fn new() -> Self {
        Self {
            condvar: Condvar::new(),
            counter: Mutex::new(0),
        }
    }

    pub fn get_latest_event(&self) -> EventCookie {
        EventCookie {
            count: *self.counter.lock(),
        }
    }

    // Give none to wait_type to wait for any past/future events
    // since construction of Event structure.
    //
    // Past/future is relative to this call
    //
    // Return Err if timed out or Ok if not timed out, both returns the same LastEventCookie representing
    // current latest event
    //
    // If timeout not given, return value always Ok
    pub fn wait_with_timeout(
        &self,
        since: EventCookie,
        timeout: Timeout,
    ) -> Result<EventCookie, EventCookie> {
        let mut counter_ref = self.counter.lock();
        let current = since.count;

        let mut cookie = EventCookie {
            count: *counter_ref,
        };

        match timeout {
            Timeout::Duration(timeout) => {
                if self
                    .condvar
                    .wait_while_for(&mut counter_ref, |x| *x == current, timeout)
                    .timed_out()
                {
                    return Err(cookie);
                }
            }

            Timeout::Infinite => {
                self.condvar.wait_while(&mut counter_ref, |x| *x == current);
            }

            Timeout::NoWait => {
                if *counter_ref == current {
                    return Err(cookie);
                }
            }
        }

        cookie.count = *counter_ref;
        Ok(cookie)
    }

    pub fn wait(&self, since: EventCookie) -> EventCookie {
        self.wait_with_timeout(since, Timeout::Infinite)
            .unwrap_or_else(identity)
    }

    pub fn notify(&self) {
        *self.counter.lock() += 1;
        self.condvar.notify_all();
    }
}

pub struct Completion {
    is_completed: Mutex<bool>,
    condvar: Condvar,
}

impl Completion {
    pub fn new() -> Self {
        Self {
            is_completed: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    pub fn complete(&self) {
        *self.is_completed.lock() = true;
        self.condvar.notify_all();
    }

    pub fn wait_until(&self, deadline: Instant) -> bool {
        !self
            .condvar
            .wait_while_until(
                &mut self.is_completed.lock(),
                |&mut completed| !completed,
                deadline,
            )
            .timed_out()
    }

    pub fn wait(&self) {
        self.condvar
            .wait_while(&mut self.is_completed.lock(), |&mut completed| !completed);
    }

    pub fn reset(&mut self) {
        *self.is_completed.get_mut() = false;
    }
}
