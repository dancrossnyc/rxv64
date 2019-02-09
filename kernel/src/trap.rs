use crate::arch;
use crate::kbd;
use crate::proc;
use crate::spinlock::SpinMutex as Mutex;
use crate::uart;
use crate::xapic;

const INTR0: u32 = 32;
const KBD_INTR: u32 = INTR0 + kbd::INTR;
const EIA0_INTR: u32 = INTR0 + uart::INTR_EIA0;
const TIMER_INTR: u32 = 8 + INTR0;

const PAGE_FAULT: u32 = 14;

static TICKS: Mutex<u64> = Mutex::new("time", 0);

pub fn ticks() -> u64 {
    *TICKS.lock()
}

pub extern "C" fn trap(vecnum: u32, frame: &mut arch::TrapFrame) {
    match vecnum {
        PAGE_FAULT => {
            panic!(
                "page fault at {:x}, rip = {:x}, error = {:x}",
                arch::fault_addr(),
                frame.rip,
                frame.error
            );
        }
        KBD_INTR => {
            assert!(arch::mycpu_id() == 0);
            if let Some(b) = unsafe { kbd::getb() } {
                match b {
                    0x20..=0x7e => crate::print!("{}", char::from(b)),
                    b'\n' => crate::println!(),
                    _ => crate::println!("KBD got {:x}", b),
                }
            }
            unsafe {
                xapic::eoi();
            }
        }
        EIA0_INTR => {
            assert!(arch::mycpu_id() == 0);
            let mut uart = uart::Uart::uart0();
            if let Some(b) = uart.getb() {
                match b {
                    0x20..=0x7e => crate::print!("{}", char::from(b)),
                    b'\n' => crate::println!(),
                    _ => crate::println!("KBD got {:x}", b),
                }
            }
            unsafe {
                xapic::eoi();
            }
        }
        TIMER_INTR => {
            crate::println!("Tick{}!", arch::mycpu_id());
            if arch::mycpu_id() == 0 {
                TICKS.with_lock(|ticks| {
                    *ticks += 1;
                    proc::wakeup(ticks as *const u64 as usize);
                });
            }
            unsafe {
                xapic::eoi();
            }
        }
        _ => {
            crate::println!("Frame: {:x?}!", frame);
            panic!("unanticipated interrupt");
        }
    }

    if vecnum == 40 {
        proc::yield_if_running();
    }
}

static mut IDT: arch::IDT = arch::IDT::empty();

pub unsafe fn vector_init() {
    IDT.init();
}

pub unsafe fn init() {
    IDT.load();
}
