use super::*;

mod strlcopy_tests {
    #[test]
    fn empty() {
        let s = b"\0";
        let mut d: [u8; 128] = [0xFF; 128];
        let r = unsafe { super::strlcpy(d.as_mut_ptr(), s.as_ptr(), d.len()) };
        assert_eq!(d[0], 0);
        assert_eq!(r, 0);
    }

    #[test]
    fn shorter() {
        let s = b"abcd\0";
        let mut d: [u8; 128] = [0xFF; 128];
        let r = unsafe { super::strlcpy(d.as_mut_ptr(), s.as_ptr(), d.len()) };
        assert_eq!(r, 4);
        assert!(d.starts_with(b"abcd\0\xFF"));
        assert!(d.ends_with(b"\xFF\xFF\xFF\xFF"));
    }

    #[test]
    fn longer() {
        let s = b"0123456789ABCDEF\0";
        let mut d: [u8; 4] = [0xFF; 4];
        let r = unsafe { super::strlcpy(d.as_mut_ptr(), s.as_ptr(), d.len()) };
        assert_eq!(r, 16);
        assert_eq!(d, *b"012\0");
    }

    #[test]
    fn exact() {
        let s = b"0123456789ABCDEF\0";
        let mut d: [u8; 17] = [0xFF; 17];
        let r = unsafe { super::strlcpy(d.as_mut_ptr(), s.as_ptr(), d.len()) };
        assert_eq!(r, 16);
        assert_eq!(d, *s);
    }

    #[test]
    fn truncate1() {
        let s = "0123456789ABCDEF\0";
        let mut d: [u8; 16] = [0xFF; 16];
        let r = unsafe { super::strlcpy(d.as_mut_ptr(), s.as_ptr(), d.len()) };
        assert_eq!(r, 16);
        assert_eq!(d, *b"0123456789ABCDE\0");
    }
}
mod strlen_tests {
    #[test]
    fn empty() {
        let s = b"\0";
        assert_eq!(unsafe { super::strlen(s.as_ptr()) }, 0);
    }

    #[test]
    fn simple() {
        let s = b"asdf\0";
        assert_eq!(unsafe { super::strlen(s.as_ptr()) }, 4);
    }
}

mod strchr_tests {
    #[test]
    fn empty() {
        let s = b"\0";
        assert!(unsafe { super::strchr(s.as_ptr(), b'a').is_null() });
    }

    #[test]
    fn notfound() {
        let s = b"xyzzy\0";
        assert!(unsafe { super::strchr(s.as_ptr(), b'a').is_null() });
    }

    #[test]
    fn first() {
        let s = b"xyzzy\0";
        assert_eq!(unsafe { super::strchr(s.as_ptr(), b'x') }, &s[0]);
    }

    #[test]
    fn second() {
        let s = b"xyzzy\0";
        assert_eq!(unsafe { super::strchr(s.as_ptr(), b'y') }, &s[1]);
    }

    #[test]
    fn last() {
        let s = b"1234\0";
        assert_eq!(unsafe { super::strchr(s.as_ptr(), b'4') }, &s[3]);
    }
}

mod strcmp_tests {
    #[test]
    fn empty() {
        let a = b"\0";
        let b = b"\0";
        assert_eq!(unsafe { super::strcmp(a.as_ptr(), b.as_ptr()) }, 0);
    }

    #[test]
    fn aempty() {
        let a = b"\0";
        let b = b"asdf\0";
        assert_eq!(unsafe { super::strcmp(a.as_ptr(), b.as_ptr()) }, -(b'a' as i32));
    }

    #[test]
    fn bempty() {
        let a = b"asdf\0";
        let b = b"\0";
        assert_eq!(unsafe { super::strcmp(a.as_ptr(), b.as_ptr()) }, b'a' as i32);
    }

    #[test]
    fn same() {
        let a = b"abc\0";
        let b = b"abc\0";
        assert_eq!(unsafe { super::strcmp(a.as_ptr(), b.as_ptr()) }, 0);
    }

    #[test]
    fn differ1() {
        let a = b"abd\0";
        let b = b"abc\0";
        assert_eq!(unsafe { super::strcmp(a.as_ptr(), b.as_ptr()) }, 1);
    }

    #[test]
    fn differn1() {
        let a = b"abc\0";
        let b = b"abd\0";
        assert_eq!(unsafe { super::strcmp(a.as_ptr(), b.as_ptr()) }, -1);
    }

    #[test]
    fn longera() {
        let a = b"abcdefg\0";
        let b = b"abc\0";
        assert_eq!(unsafe { super::strcmp(a.as_ptr(), b.as_ptr()) }, 'd' as i32);
    }

    #[test]
    fn longerb() {
        let a = b"abc\0";
        let b = b"abcdefg\0";
        assert_eq!(
            unsafe { super::strcmp(a.as_ptr(), b.as_ptr()) },
            -('d' as i32)
        );
    }

    #[test]
    fn differ1longera() {
        let a = b"abddefg\0";
        let b = b"abc\0";
        assert_eq!(unsafe { super::strcmp(a.as_ptr(), b.as_ptr()) }, 1);
    }

    #[test]
    fn differ1longerb() {
        let a = b"abd\0";
        let b = b"abcdefg\0";
        assert_eq!(unsafe { super::strcmp(a.as_ptr(), b.as_ptr()) }, 1);
    }
}

mod atoi_tests {
    #[test]
    fn empty() {
        let s = b"\0";
        assert_eq!(unsafe { super::atoi(s.as_ptr()) }, 0);
    }

    #[test]
    fn zero() {
        let s = b"0\0";
        assert_eq!(unsafe { super::atoi(s.as_ptr()) }, 0);
    }

    #[test]
    fn one() {
        let s = b"1\0";
        assert_eq!(unsafe { super::atoi(s.as_ptr()) }, 1);
    }

    #[test]
    fn two() {
        let s = b"2\0";
        assert_eq!(unsafe { super::atoi(s.as_ptr()) }, 2);
    }

    #[test]
    fn long() {
        let s = b"99999\0";
        assert_eq!(unsafe { super::atoi(s.as_ptr()) }, 99999);
    }
}
