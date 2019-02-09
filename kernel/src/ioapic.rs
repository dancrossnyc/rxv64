// The I/O APIC is a fundamental component of the Intel
// interrupt controller architecture, but is shockingly
// antiquated.  Documentation on programming it, let
// alone it's architecture, is chipset specific and sparse.
//
// We program to a published spec, but one that seems to
// have disappeared from Intel's web site.  However, we
// rely on the fact that it was once published along with
// Intel's seeming commitment to backwards compatibility
// in perpetuity to keep our code working.
//
// Ultimately, we only use the IOAPIC for remapping
// IRQs and destinations processors for legacy ISA bus
// devices anyway.  Anything involving PCI pretty much
// requires ACPI AML support, which we don't provide.

use crate::acpi;
use crate::param;
use bitflags::bitflags;
use core::ptr::{null_mut, read_volatile, write_volatile};

#[repr(C)]
struct IOAPIC {
    reg: u32,
    _unused0: u32,
    _unused1: u32,
    _unused2: u32,
    value: u32,
}

bitflags! {
    pub struct IntrFlags: u32 {
        const DISABLED = 0x0001_0000;
        const _LEVEL = 0x0000_8000;
        const _ACTIVE_LOW = 0x0000_2000;
        const _LOGICAL = 0x0000_0800;
    }
}

enum IOAPICRegs {
    ID = 0,
    VER = 1,
    TABLE = 16,
}

static mut IOAPIC: *mut IOAPIC = null_mut();
static mut MAXINTR: u32 = 0;
static mut ID: u32 = 0;

pub unsafe fn init(ioapics: &[acpi::IOAPICT]) {
    assert_eq!(IOAPIC, null_mut());
    assert!(!ioapics.is_empty());
    IOAPIC = (param::KERNBASE + ioapics[0].phys_addr() as usize) as *mut IOAPIC;
    MAXINTR = (read(IOAPICRegs::VER) >> 16) & 0xFF;
    ID = read(IOAPICRegs::ID) >> 24;
    for k in 0..=MAXINTR {
        write_table(k, IntrFlags::DISABLED, 32 + k, 0);
    }
}

pub unsafe fn enable(irq: u32, cpu: u32) {
    write_table(irq, IntrFlags::empty(), irq + 32, cpu);
}

unsafe fn read(index: IOAPICRegs) -> u32 {
    assert_ne!(IOAPIC, null_mut());
    let ioapic = &mut *IOAPIC;
    write_volatile(&mut ioapic.reg, index as u32);
    read_volatile(&ioapic.value)
}

unsafe fn _write(index: IOAPICRegs, value: u32) {
    assert_ne!(IOAPIC, null_mut());
    let ioapic = &mut *IOAPIC;
    write_volatile(&mut ioapic.reg, index as u32);
    write_volatile(&mut ioapic.value, value);
}

unsafe fn write_table(offset: u32, flags: IntrFlags, irq: u32, cpu: u32) {
    assert_ne!(IOAPIC, null_mut());
    let ioapic = &mut *IOAPIC;
    let index = IOAPICRegs::TABLE as u32;
    write_volatile(&mut ioapic.reg, index + offset * 2);
    write_volatile(&mut ioapic.value, flags.bits() | irq);
    write_volatile(&mut ioapic.reg, index + offset * 2 + 1);
    write_volatile(&mut ioapic.value, cpu << 24);
}
