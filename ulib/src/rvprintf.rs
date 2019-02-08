// Rust does not support variadic functions, so we write vprintf() in Rust
// and call it via a stub.
use core::ffi;

extern "C" {
    fn write(fd: i32, p: *const u8, len: usize) -> isize;
}

enum S {
    NORMAL,
    VERB,
}

pub fn rvprintf(fd: i32, fmt: &[u8], mut ap: ffi::VaList) {
    let mut state = S::NORMAL;
    for c in fmt {
        state = match state {
            S::NORMAL => printc(fd, *c),
            S::VERB => printv(fd, *c, &mut ap),
        }
    }
}

fn printc(fd: i32, c: u8) -> S {
    if c == b'%' {
        S::VERB
    } else {
        putc(fd, c);
        S::NORMAL
    }
}

enum Base {
    OCT = 8,
    DEC = 10,
    HEX = 16,
}

fn printv(fd: i32, c: u8, ap: &mut ffi::VaList) -> S {
    match c {
        b'%' => putc(fd, b'%'),
        b'c' => putc(fd, unsafe { ap.arg::<u8>() }),
        b'd' => {
            let d = unsafe { ap.arg::<i32>() };
            if d < 0 {
                printnegint(fd, i64::abs(i64::from(d)) as u64, Base::DEC);
            } else {
                printint(fd, d as u64, Base::DEC);
            }
        }
        b'o' => printint(fd, unsafe { ap.arg::<u64>() }, Base::OCT),
        b'p' | b'x' => printint(fd, unsafe { ap.arg::<u64>() }, Base::HEX),
        b's' => {
            let s = unsafe { ap.arg::<*const u8>() };
            let t = if s.is_null() {
                b"(null)"
            } else {
                unsafe { super::cstr2slice(s) }
            };
            puts(fd, t)
        }
        _ => {
            putc(fd, b'%');
            putc(fd, c);
        }
    };
    S::NORMAL
}

fn printnegint(fd: i32, x: u64, base: Base) {
    putc(fd, b'-');
    printint(fd, x, base);
}

fn printint(fd: i32, mut x: u64, base: Base) {
    const DIGITS: &[u8] = b"0123456789ABCDEF";
    let mut buf: [u8; 32] = [b'0'; 32];
    let mut cnt = 31;
    let mut s;
    let b = base as u64;
    while {
        s = &mut buf[cnt..];
        s[0] = DIGITS[(x % b) as usize];
        x /= b;
        cnt > 0 && x != 0
    } {
        cnt -= 1;
    }
    puts(fd, s);
}

fn putc(fd: i32, b: u8) {
    unsafe {
        write(fd, &b as *const u8, 1);
    }
}

fn puts(fd: i32, bs: &[u8]) {
    unsafe {
        write(fd, bs.as_ptr(), bs.len());
    }
}
