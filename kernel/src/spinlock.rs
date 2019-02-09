use crate::arch;
use arch::{cpu_relax, mycpu_id, xswap, CPU};
use core::cell::UnsafeCell;
use core::marker::{Send, Sync};
use core::ops::{Deref, DerefMut};

#[derive(Debug)]
pub struct Spinlock {
    locked: u64,
    name: &'static str,
    cpu: i64,
    _pcs: [u64; 10],
}

unsafe impl Send for Spinlock {}
unsafe impl Sync for Spinlock {}

impl Spinlock {
    pub const fn new(name: &'static str) -> Spinlock {
        Spinlock {
            locked: 0,
            name,
            cpu: -1,
            _pcs: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        }
    }

    pub fn acquire(&mut self) {
        unsafe { CPU::push_intr_disable() };
        assert!(!self.holding(), "nested lock: {}", self.name);
        while xswap(&mut self.locked, 1) != 0 {
            cpu_relax();
        }
        self.cpu = i64::from(mycpu_id());
    }

    pub fn release(&mut self) {
        assert!(self.holding(), "unlocking unheld lock {}", self.name);
        self.cpu = -1;
        xswap(&mut self.locked, 0);
        unsafe { CPU::pop_intr_disable() };
    }

    pub fn holding(&self) -> bool {
        without_intrs(|| self.locked != 0 && self.cpu == i64::from(mycpu_id()))
    }
}

#[derive(Debug)]
pub struct SpinMutex<T: ?Sized> {
    lock: UnsafeCell<Spinlock>,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized> Send for SpinMutex<T> {}
unsafe impl<T: ?Sized> Sync for SpinMutex<T> {}

impl<T> SpinMutex<T> {
    pub const fn new(name: &'static str, data: T) -> SpinMutex<T> {
        SpinMutex {
            lock: UnsafeCell::new(Spinlock::new(name)),
            data: UnsafeCell::new(data),
        }
    }

    pub fn acquire(&self) {
        unsafe { &mut *self.lock.get() }.acquire();
    }

    pub fn release(&self) {
        unsafe { &mut *self.lock.get() }.release();
    }

    pub fn lock(&self) -> MutexGuard<T> {
        self.acquire();
        MutexGuard {
            lock: &self.lock,
            data: unsafe { &mut *self.data.get() },
        }
    }

    pub fn lock_ref(&self) -> &Spinlock {
        unsafe { &*self.lock.get() }
    }

    pub fn holding(&self) -> bool {
        self.lock_ref().holding()
    }

    pub fn with_lock<U, F: FnMut(&mut T) -> U>(&self, mut thunk: F) -> U {
        self.acquire();
        let r = thunk(unsafe { &mut *self.data.get() });
        self.release();
        r
    }
}

pub struct MutexGuard<'a, T: ?Sized + 'a> {
    lock: &'a UnsafeCell<Spinlock>,
    data: &'a mut T,
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.data
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.data
    }
}

impl<'a, T: ?Sized> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        unsafe { &mut *self.lock.get() }.release();
    }
}

pub fn without_intrs<U, F: FnMut() -> U>(mut thunk: F) -> U {
    unsafe { CPU::push_intr_disable() };
    let r = thunk();
    unsafe { CPU::pop_intr_disable() };
    r
}
