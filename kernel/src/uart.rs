use crate::arch::{cpu_relax, inb, outb, sleep};
use crate::ioapic;
use core::fmt;
use core::time::Duration;

const EIA0: u16 = 0x3f8; // aka COM1
pub const INTR_EIA0: u32 = 4;

// Input ports.
const RBR: u16 = 0;
const IER: u16 = 1;
const IIR: u16 = 2;
const LCR: u16 = 3;
const MCR: u16 = 4;
const LSR: u16 = 5;
const _MSR: u16 = 6;
const _SCR: u16 = 7;

// Output ports.
const THR: u16 = 0;
const _FCR: u16 = 2;

// Line status bits.
const LSR_THRE: u8 = 0b0010_0000;

pub struct Uart {
    port: u16,
}

pub unsafe fn init() {
    outb(EIA0 + IIR, 0); // Turn off FIFO

    // 115200 BAUD, 8 data pits, 1 stop bit, no parity.
    outb(EIA0 + LCR, 0x80); // Unlock divisor
    outb(EIA0, 1); // BAUD rate divisor: (115_200u32 / 115_200u32) => 115_200
    outb(EIA0 + 1, 0);
    outb(EIA0 + LCR, 0x03); // lock divisor, 8 data bits.
    outb(EIA0 + MCR, 0);
    outb(EIA0 + IER, 1); // Enable receive interrupts

    // Clear pre-existing interrupt conditions.
    let _ = inb(EIA0 + IIR);
    let _ = inb(EIA0);
    ioapic::enable(INTR_EIA0, 0);
}

impl Uart {
    pub const fn uart0() -> Uart {
        Uart { port: EIA0 }
    }

    fn tx_ready(&mut self) -> bool {
        for _ in 0..128 {
            if unsafe { inb(self.port + LSR) } & LSR_THRE != 0 {
                return true;
            }
            cpu_relax();
        }
        false
    }

    pub fn putb(&mut self, b: u8) {
        while !self.tx_ready() {}
        unsafe { outb(self.port + THR, b) };
    }

    fn rx_ready(&mut self) -> bool {
        for _ in 0..128 {
            if (unsafe { inb(self.port + LSR) } & 0b0010_0000) == 0b0010_0000 {
                return true;
            }
            sleep(Duration::from_micros(1));
        }
        false
    }

    pub fn getb(&mut self) -> Option<u8> {
        if !self.rx_ready() {
            return None;
        }
        let b = unsafe { inb(self.port + RBR) };
        Some(b)
    }
}

impl fmt::Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                self.putb(b'\r');
            }
            self.putb(b);
        }
        Ok(())
    }
}
