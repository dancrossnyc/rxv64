use crate::cga::Cga;
use crate::spinlock::SpinMutex as Mutex;
use crate::uart::Uart;
use core::fmt;

pub struct Writers {
    uart: Option<Uart>,
    cga: Option<Cga>,
}

impl fmt::Write for Writers {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if let Some(uart) = self.uart.as_mut() {
                if b == b'\n' {
                    uart.putb(b'\r');
                } else if b == b'\x07' {
                    uart.putb(b);
                    uart.putb(b' ');
                }
                uart.putb(b);
            }
            if let Some(cga) = self.cga.as_mut() {
                cga.putb(b);
            }
        }
        Ok(())
    }
}

pub static WRITER: Mutex<Writers> = Mutex::new(
    "cons",
    Writers {
        uart: Some(Uart::uart0()),
        cga: Some(Cga::new()),
    },
);

pub unsafe fn init() {
    let mut writer = WRITER.lock();
    if let Some(cga) = writer.cga.as_mut() {
        cga.blank();
    }
}

// The standard kernel println!() is protected by a mutex.
#[cfg(not(test))]
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[cfg(not(test))]
#[macro_export]
macro_rules! print {
    ($($args:tt)*) => ({
        use $crate::console::print;
        print(format_args!($($args)*));
    })
}

pub fn print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    WRITER.lock().write_fmt(args).unwrap();
}

// These macros do not lock, so that they can be called from
// a panic!() handler on a potentially wedged machine.
#[cfg(not(test))]
#[macro_export]
macro_rules! panic_println {
    () => (uart_print!("\n"));
    ($($arg:tt)*) => ($crate::panic_print!("{}\n", format_args!($($arg)*)));
}

#[cfg(not(test))]
#[macro_export]
macro_rules! panic_print {
    ($($args:tt)*) => ({
        use core::fmt::Write;
        let mut writer = $crate::uart::Uart::uart0();
        writer.write_fmt(format_args!($($args)*)).unwrap();
    })
}
