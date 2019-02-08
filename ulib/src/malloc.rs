// Lightly modified K&R allocator.  Note that sizes are in "units",
// not bytes.
use core::cmp;
use core::mem;
use core::ptr;
use core::slice;

extern "C" {
    fn sbrk(nbytes: isize) -> *mut u8;
}

#[repr(C)]
#[derive(Debug)]
struct Header {
    next: *mut Header,
    nunits: usize,
}

impl Header {
    pub fn as_slice_mut(&mut self) -> &'static mut [u8] {
        let ptr = unsafe { (self as *mut Header).offset(1) };
        let len = (self.nunits - 1) * mem::size_of::<Header>();
        unsafe { slice::from_raw_parts_mut(ptr as *mut u8, len) }
    }

    pub fn end(&mut self) -> *mut Header {
        let ptr = self as *mut Header;
        let nu = self.nunits as isize;
        unsafe { ptr.offset(nu) }
    }
}

static mut FREE_LIST: Option<*mut Header> = None;
static mut BASE: Header = Header {
    next: 0 as *mut Header,
    nunits: 0,
};

fn bytes2units(bytes: usize, unit_size: usize) -> usize {
    assert_ne!(unit_size, 0);
    let aligned = bytes + ((unit_size - (bytes % unit_size)) % unit_size);
    aligned / unit_size
}

#[no_mangle]
pub unsafe extern "C" fn krmalloc(n: usize) -> *mut u8 {
    if let Some(s) = inner_malloc(&mut FREE_LIST, n) {
        s.as_mut_ptr()
    } else {
        ptr::null_mut()
    }
}

fn inner_malloc(free_list: &mut Option<*mut Header>, n: usize) -> Option<&'static mut [u8]> {
    if n != 0 {
        if free_list.is_none() {
            let base = unsafe { &mut BASE };
            base.nunits = 0;
            base.next = base as *mut Header;
            *free_list = Some(base);
        }
        let nunits = bytes2units(n, mem::align_of::<Header>()) + 1;
        let mut prevp = free_list.unwrap();
        let mut pp = unsafe { prevp.as_ref().unwrap().next };
        loop {
            let pnunits = unsafe { pp.as_ref().unwrap().nunits };
            if pnunits >= nunits {
                let mp = unsafe { pp.as_mut().unwrap() };
                let p = if pnunits == nunits {
                    let prev = unsafe { prevp.as_mut().unwrap() };
                    prev.next = mp.next;
                    mp
                } else {
                    mp.nunits -= nunits;
                    let p = unsafe { &mut *(mp as *mut Header).add(mp.nunits) };
                    p.nunits = nunits;
                    p
                };
                *free_list = Some(prevp);
                return Some(p.as_slice_mut());
            }
            if pp == prevp {
                let units = more_units(nunits)?;
                pp = inner_free(free_list, units);
            }
            prevp = pp;
            pp = unsafe { pp.as_ref().unwrap().next };
        }
    }
    None
}

fn more_units(nunits: usize) -> Option<&'static mut Header> {
    let nu = cmp::max(nunits, 4096);
    #[allow(clippy::cast_ptr_alignment)]
    let p = safe_sbrk(nu * mem::size_of::<Header>())? as *mut Header;
    assert_eq!(p as usize % mem::align_of::<Header>(), 0);
    let bt = unsafe { &mut *p };
    bt.nunits = nunits;
    bt.next = p;
    Some(bt)
}

fn safe_sbrk(nbytes: usize) -> Option<*mut u8> {
    let p = unsafe { sbrk(nbytes as isize) };
    if p as usize == !0 {
        None
    } else {
        Some(p)
    }
}

#[no_mangle]
pub unsafe extern "C" fn krfree(p: *mut u8) {
    fn adjptr(p: *mut u8) -> &'static mut Header {
        let off = mem::size_of::<Header>() as isize;
        let hp = unsafe { p.offset(-off) };
        assert_eq!(hp as usize % mem::align_of::<Header>(), 0);
        unsafe {
            #[allow(clippy::cast_ptr_alignment)]
            &mut *(hp as *mut Header)
        }
    }
    inner_free(&mut FREE_LIST, adjptr(p));
}

fn inner_free(free_list: &mut Option<*mut Header>, tag: &mut Header) -> *mut Header {
    assert_ne!(tag.nunits, 0);
    let tp = tag as *mut Header;
    if free_list.is_none() {
        *free_list = Some(tp);
        return tp;
    }
    let mut p = free_list.unwrap();
    loop {
        let np = unsafe { p.as_ref().unwrap().next };
        if (p < tp && tp < np) || (p >= np && (p < tp || tp < np)) {
            if tag.end() == p {
                let next = unsafe { np.as_ref().unwrap() };
                tag.nunits += next.nunits;
                tag.next = next.next;
            } else {
                tag.next = np;
            }
            let mut current = unsafe { p.as_mut().unwrap() };
            if current.end() == tp {
                current.nunits += tag.nunits;
                current.next = tag.next;
            } else {
                current.next = tp;
            }
            *free_list = Some(p);
            return p;
        }
        p = np;
    }
}
