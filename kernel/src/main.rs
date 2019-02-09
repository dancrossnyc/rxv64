#![feature(asm)]
#![feature(const_fn_trait_bound)]
#![feature(const_mut_refs)]
#![feature(core_intrinsics)]
#![feature(global_asm)]
#![feature(lang_items)]
#![feature(naked_functions)]
#![feature(proc_macro_hygiene)]
#![cfg_attr(test, allow(dead_code))]
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(clippy::upper_case_acronyms)]

mod acpi;
mod bio;
mod cga;
mod console;
mod exec;
mod file;
mod fs;
mod fslog;
mod initcode;
mod ioapic;
mod kalloc;
mod kbd;
mod kmem;
mod param;
mod pci;
mod pipe;
mod proc;
mod sd;
mod sleeplock;
mod smp;
mod spinlock;
mod syscall;
mod sysfile;
mod trap;
mod uart;
mod vm;
mod x86_64;
mod xapic;

#[cfg(test)]
use std::print;
#[cfg(test)]
use std::println;

use crate::vm::PageTable;
use crate::x86_64 as arch;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
use arch::pic as PIC;
use arch::Page;
use arch::CPU;
use core::result;
use core::sync::atomic::AtomicBool;

type Result<T> = result::Result<T, &'static str>;

pub unsafe trait FromZeros {}

unsafe impl<T: ?Sized> FromZeros for *const T {}
unsafe impl<T: ?Sized> FromZeros for *mut T {}
unsafe impl FromZeros for bool {}
unsafe impl FromZeros for char {}
unsafe impl FromZeros for f32 {}
unsafe impl FromZeros for f64 {}
unsafe impl FromZeros for isize {}
unsafe impl FromZeros for usize {}
unsafe impl FromZeros for i8 {}
unsafe impl FromZeros for u8 {}
unsafe impl FromZeros for i16 {}
unsafe impl FromZeros for u16 {}
unsafe impl FromZeros for i32 {}
unsafe impl FromZeros for u32 {}
unsafe impl FromZeros for i64 {}
unsafe impl FromZeros for u64 {}

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
static mut PERCPU0: Page = Page::empty();
static mut KPGTBL: PageTable = PageTable::empty();

/// # Safety
///
/// Starting an operating system is inherently unsafe.
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
#[no_mangle]
pub unsafe extern "C" fn main(boot_info: u64) {
    CPU::init(&mut PERCPU0, 0);
    console::init();
    println!("rxv64...");
    PIC::init();
    trap::vector_init();
    trap::init();
    kalloc::early_init(kmem::early_pages());
    kmem::early_init(boot_info);
    vm::init(&mut KPGTBL);
    vm::switch(&KPGTBL);
    kmem::init();
    acpi::init();
    ioapic::init(acpi::ioapics());
    xapic::init();
    kbd::init();
    uart::init();
    pci::init();
    sd::init();
    bio::init();
    pipe::init();
    syscall::init();
    proc::init(&KPGTBL);
    smp::init();
    smp::start_others(acpi::cpus());

    let semaphore = AtomicBool::new(false);
    mpmain(0, &semaphore);
}

/// # Safety
///
/// Starting a CPU is inherently unsafe.
#[no_mangle]
pub unsafe extern "C" fn mpenter(percpu: &mut Page, id: u32, semaphore: &AtomicBool) {
    CPU::init(percpu, id);
    trap::init();
    vm::switch(&KPGTBL);
    xapic::init();
    syscall::init();
    mpmain(id, semaphore)
}

fn mpmain(id: u32, semaphore: &AtomicBool) {
    println!("cpu{} starting", id);
    signal_up(semaphore);
    proc::scheduler();
}

fn signal_up(semaphore: &AtomicBool) {
    use core::sync::atomic::Ordering;
    semaphore.store(true, Ordering::Relaxed);
}

#[cfg(not(test))]
mod runtime {
    use core::panic::PanicInfo;

    #[panic_handler]
    pub extern "C" fn panic(info: &PanicInfo) -> ! {
        use crate::panic_println;
        panic_println!("RUST PANIC: {:?}", info);
        #[allow(clippy::empty_loop)]
        loop {}
    }

    #[lang = "eh_personality"]
    extern "C" fn eh_personality() {}
}
