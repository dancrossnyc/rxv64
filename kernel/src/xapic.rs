// Ideally we would program to the x2APIC specification,
// but that's not universally available; in particular,
// TCG emulation in QEMU does not exist.  So we use the
// xAPIC instead.

use crate::arch;
use crate::param;
use crate::trap;
use crate::volatile;
use bitflags::bitflags;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, Ordering};
use core::time::Duration;

enum XAPICRegs {
    ID = 0x0020 / 4,
    _VER = 0x0030 / 4,
    TPR = 0x0080 / 4,
    EOI = 0x00B0 / 4,
    SVR = 0x00F0 / 4,
    ESR = 0x0280 / 4,
    ICRLO = 0x0300 / 4,
    ICRHI = 0x0310 / 4,
    TIMER = 0x0320 / 4,
    _PCINT = 0x0340 / 4,
    _LINT0 = 0x0350 / 4,
    _LINT1 = 0x0360 / 4,
    _ERROR = 0x0370 / 4,
    TICR = 0x0380 / 4,
    _TCCR = 0x0390 / 4,
    TDCR = 0x03E0 / 4,
}
const SIZE: usize = (0x03E0 + 4) / 4;

const INIT: u32 = 0b101 << 8; // INIT/RESET
const STARTUP: u32 = 0b110 << 8; // INIT/RESET
const LEVEL: u32 = 1 << 15; // Level (vs edge).
const ASSERT: u32 = 1 << 14; // Assert interrupt (vs deassert)
const DEASSERT: u32 = 0; // Assert interrupt (vs deassert)
const DELIVS: u32 = 0x0000_1000; // Delivery status
const PERIODIC: u32 = 0x0002_0000;

pub const INTR_TIMER: u32 = 8;
const SPURIOUS_VEC: u32 = trap::INTR0 + 31;

type XAPICMMIO = [u32; SIZE];

static XAPIC: AtomicPtr<XAPICMMIO> = AtomicPtr::new(null_mut());

bitflags! {
    pub struct SVRFlags: u32 {
        const ENABLE = 0x0000_0100;
    }
}

pub unsafe fn init() {
    const MSR_APIC_BASE: u32 = 0x0000_001b;
    const XAPIC_MODE: u64 = 0x800;
    unsafe {
        arch::wrmsr(MSR_APIC_BASE, arch::rdmsr(MSR_APIC_BASE) | XAPIC_MODE);

        let xptr = XAPIC.load(Ordering::Acquire);
        assert!((arch::mycpu_id() == 0 && xptr.is_null()) || !xptr.is_null());

        const MMIO_MASK: u64 = !0xFFF;
        let xapic_ptr = param::KERNBASE + (arch::rdmsr(MSR_APIC_BASE) & MMIO_MASK) as usize;
        XAPIC.store(xapic_ptr as *mut XAPICMMIO, Ordering::Release);
        write(XAPICRegs::SVR, SVRFlags::ENABLE.bits() | SPURIOUS_VEC);

        write(XAPICRegs::TDCR, 0xb);
        write(XAPICRegs::TIMER, PERIODIC | trap::INTR0 + INTR_TIMER);
        write(XAPICRegs::TICR, 2_000_000_000 / 1000); // assume 2GHz clock

        write(XAPICRegs::ESR, 0);
        write(XAPICRegs::ESR, 0);

        write(XAPICRegs::EOI, 0);

        write(XAPICRegs::TPR, 0);
    }
}

fn xapic_ref() -> &'static XAPICMMIO {
    let xapic = XAPIC.load(Ordering::Acquire);
    assert_ne!(xapic, null_mut());
    unsafe { &*xapic }
}

fn xapic_mut() -> &'static mut XAPICMMIO {
    let xapic = XAPIC.load(Ordering::Acquire);
    assert_ne!(xapic, null_mut());
    unsafe { &mut *xapic }
}

unsafe fn read(index: XAPICRegs) -> u32 {
    let xapic = xapic_ref();
    volatile::read(&xapic[index as usize])
}

unsafe fn write(index: XAPICRegs, value: u32) {
    let xapic = xapic_mut();
    volatile::write(&mut xapic[index as usize], value);
    volatile::read(&xapic[XAPICRegs::ID as usize]);
}

unsafe fn wait_delivery() {
    for _ in 0..100_000 {
        if unsafe { read(XAPICRegs::ICRLO) } & DELIVS == 0 {
            break;
        }
        arch::cpu_relax();
    }
}

pub unsafe fn eoi() {
    unsafe {
        write(XAPICRegs::EOI, 0);
    }
}

pub unsafe fn send_init_ipi(apic_id: u32) {
    unsafe {
        write(XAPICRegs::ICRHI, apic_id << 24);
        write(XAPICRegs::ICRLO, INIT | LEVEL | ASSERT);
        wait_delivery();
        arch::sleep(Duration::from_micros(200));
        write(XAPICRegs::ICRLO, INIT | LEVEL | DEASSERT);
        wait_delivery();
    }
    arch::sleep(Duration::from_micros(100));
}

pub unsafe fn send_sipi(apic_id: u32, vector: u8) {
    unsafe {
        write(XAPICRegs::ICRHI, apic_id << 24);
        write(XAPICRegs::ICRLO, STARTUP | u32::from(vector));
        wait_delivery();
    }
}
