#![feature(c_variadic)]
#![feature(exposed_provenance)]
#![feature(naked_functions)]
#![feature(strict_provenance)]
#![cfg_attr(not(any(test, clippy)), no_std)]
#![forbid(unsafe_op_in_unsafe_fn)]

use core::cmp;
use core::ffi;
use core::ptr;
use core::slice;

mod malloc;
mod rvdprintf;
mod sysx86_64;
#[cfg(test)]
mod tests;

/// # Safety
/// The input string may not be NUL-terminated.
unsafe fn cstr2slice<'a>(s: *const u8) -> &'a [u8] {
    unsafe { slice::from_raw_parts(s, strlen(s)) }
}

/// # Safety
/// C strings
#[no_mangle]
pub unsafe extern "C" fn strlcpy(dst: *mut u8, src: *const u8, size: usize) -> usize {
    fn inner(dst: &mut [u8], src: &[u8]) -> usize {
        let k = if src.len() < dst.len() {
            src.len()
        } else {
            dst.len() - 1
        };
        dst[..k].clone_from_slice(&src[..k]);
        dst[k] = b'\0';
        src.len()
    }
    let dst = unsafe { slice::from_raw_parts_mut(dst, size) };
    let src = unsafe { cstr2slice(src) };
    inner(dst, src)
}

/// # Safety
/// C strings
#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    let mut k = 0;
    while unsafe { *s.offset(k) } != 0 {
        k += 1;
    }
    k as usize
}

/// # Safety
/// C strings
#[no_mangle]
pub unsafe extern "C" fn strchr(s: *const u8, c: u8) -> *const u8 {
    fn inner(s: &[u8], c: u8) -> Option<&u8> {
        let off = s.iter().position(|ch| *ch == c)?;
        Some(&s[off])
    }
    match unsafe { inner(cstr2slice(s), c) } {
        Some(p) => p as *const u8,
        None => ptr::null(),
    }
}

/// # Safety
/// C strings
#[no_mangle]
pub unsafe extern "C" fn strcmp(p: *const u8, q: *const u8) -> i32 {
    fn inner(p: &[u8], q: &[u8]) -> i32 {
        let len = cmp::max(p.len(), q.len());
        for k in 0..len {
            let a = i32::from(*p.get(k).unwrap_or(&0));
            let b = i32::from(*q.get(k).unwrap_or(&0));
            if a != b {
                return a - b;
            }
        }
        0
    }
    inner(unsafe { cstr2slice(p) }, unsafe { cstr2slice(q) })
}

/// # Safety
/// C strings
#[no_mangle]
pub unsafe extern "C" fn atoi(s: *const u8) -> i32 {
    fn inner(s: &[u8]) -> i32 {
        s.iter()
            .take_while(|c| b'0' <= **c && **c <= b'9')
            .fold(0, |sum, c| sum * 10 + i32::from(*c - b'0'))
    }
    inner(unsafe { cstr2slice(s) })
}

/// # Safety
/// C pointers
#[no_mangle]
pub unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    //ptr::copy(src, dst, len);
    //dst
    unsafe { memcpy(dst, src, len) }
}

/// # Safety
/// C pointers
#[no_mangle]
pub unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    for k in 0..len {
        unsafe {
            ptr::write(dst.add(k), ptr::read(src.add(k)));
        }
    }
    dst
}

/// # Safety
/// C pointers
#[no_mangle]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    for k in 0..n {
        unsafe {
            let aa = ptr::read(a.add(k));
            let bb = ptr::read(b.add(k));
            if aa < bb {
                return -1;
            }
            if aa > bb {
                return 1;
            }
        }
    }
    0
}

/// # Safety
/// C pointers
#[no_mangle]
pub unsafe extern "C" fn memset(dst: *mut u8, c: u8, n: usize) -> *mut u8 {
    for k in 0..n {
        unsafe {
            ptr::write(dst.add(k), c);
        }
    }
    dst
}

/// # Safety
/// C strings and variadic args.
#[no_mangle]
pub unsafe extern "C" fn rvdprintf(fd: i32, fmt: *const u8, ap: ffi::VaList) {
    rvdprintf::rvdprintf(fd, unsafe { cstr2slice(fmt) }, ap);
}

/// # Safety
/// C interface
#[cfg(not(any(test, clippy)))]
#[no_mangle]
pub unsafe extern "C" fn malloc(n: usize) -> *mut u8 {
    unsafe { malloc::krmalloc(n) }
}

/// # Safety
/// C interface
#[cfg(not(any(test, clippy)))]
#[no_mangle]
pub unsafe extern "C" fn free(p: *mut u8) {
    unsafe {
        malloc::krfree(p);
    }
}

#[cfg(not(any(test, clippy)))]
#[panic_handler]
pub fn panic(_info: &core::panic::PanicInfo) -> ! {
    #[allow(clippy::empty_loop)]
    loop {}
}
