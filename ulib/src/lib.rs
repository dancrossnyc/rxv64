#![feature(asm)]
#![feature(c_variadic)]
#![feature(global_asm)]
#![feature(naked_functions)]
#![no_std]

use core::cmp;
use core::ffi;
use core::ptr;
use core::slice;

mod malloc;
mod rvprintf;
mod sysx86_64;
#[cfg(test)]
mod tests;

unsafe fn cstr2slice<'a>(s: *const u8) -> &'a [u8] {
    slice::from_raw_parts(s, strlen(s))
}

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
    inner(slice::from_raw_parts_mut(dst, size), cstr2slice(src))
}

#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    let mut k = 0;
    while *s.offset(k) != 0 {
        k += 1;
    }
    k as usize
}

#[no_mangle]
pub unsafe extern "C" fn strchr(s: *const u8, c: u8) -> *const u8 {
    fn inner(s: &[u8], c: u8) -> Option<&u8> {
        let off = s.iter().position(|ch| *ch == c)?;
        Some(&s[off])
    }
    match inner(cstr2slice(s), c) {
        Some(p) => p as *const u8,
        None => ptr::null(),
    }
}

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
    inner(cstr2slice(p), cstr2slice(q))
}

pub unsafe extern "C" fn atoi(s: *const u8) -> i32 {
    fn inner(s: &[u8]) -> i32 {
        s.iter()
            .take_while(|c| b'0' <= **c && **c <= b'9')
            .fold(0, |sum, c| sum * 10 + i32::from(*c - b'0'))
    }
    inner(cstr2slice(s))
}

pub unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, len: usize) -> *mut u8 {
    ptr::copy(src, dst, len);
    dst
}

pub unsafe extern "C" fn memset(dst: *mut u8, c: u8, n: usize) -> *mut u8 {
    ptr::write_bytes(dst, c, n);
    dst
}

#[no_mangle]
pub unsafe extern "C" fn rvprintf(fd: i32, fmt: *const u8, ap: ffi::VaList) {
    rvprintf::rvprintf(fd, cstr2slice(fmt), ap);
}

pub unsafe extern "C" fn malloc(n: usize) -> *mut u8 {
    malloc::krmalloc(n)
}

pub unsafe extern "C" fn free(p: *mut u8) {
    malloc::krfree(p);
}

#[cfg(not(test))]
#[panic_handler]
#[no_mangle]
pub extern "C" fn panic(_: &core::panic::PanicInfo) -> ! {
    #[allow(clippy::empty_loop)]
    loop {}
}
