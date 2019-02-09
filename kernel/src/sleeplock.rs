use crate::proc::{self, myproc};
use crate::spinlock::SpinMutex as Mutex;
use core::cell::Cell;

// A lock that it's possible to sleep on,
// for slow resources (such as IO devices).

#[derive(Debug)]
pub struct Sleeplock {
    lock: Mutex<()>,
    locked: Cell<bool>,
    pid: Cell<u32>,

    name: &'static str,
}

impl Sleeplock {
    pub const fn new(name: &'static str) -> Sleeplock {
        Sleeplock {
            lock: Mutex::new("sleeplock", ()),
            locked: Cell::new(false),
            name,
            pid: Cell::new(0),
        }
    }

    fn as_chan(&self) -> usize {
        self as *const Sleeplock as usize
    }

    pub fn acquire(&self) {
        assert!(!self.holding(), "nested sleep lock: {}", self.name);
        self.lock.with_lock(|_| {
            while self.locked.get() {
                myproc().sleep(self.as_chan(), &self.lock);
            }
            self.locked.set(true);
            self.pid.set(myproc().pid());
        });
    }

    pub fn release(&self) {
        assert!(self.holding(), "unlocking unheld sleep lock {}", self.name);
        self.lock.with_lock(|_| {
            self.locked.set(false);
            self.pid.set(0);
            proc::wakeup(self.as_chan());
        });
    }

    pub fn holding(&self) -> bool {
        self.lock.with_lock(|_| self.pid.get() == myproc().pid())
    }
}
