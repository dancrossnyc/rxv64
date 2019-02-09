use crate::arch::{self, Page, PAGE_SIZE};
use crate::param::KERNBASE;
use core::slice;

extern "C" {
    static etext: [u64; 0];
    static erodata: [u64; 0];
    //static mut edata: [u64; 0];
    static end: [u64; 0];
}

pub fn text_addr() -> usize {
    0xFFFF_8000_0010_0000
}

pub fn etext_addr() -> usize {
    unsafe { etext.as_ptr() as usize }
}

pub fn erodata_addr() -> usize {
    unsafe { erodata.as_ptr() as usize }
}

pub fn end_addr() -> usize {
    unsafe { end.as_ptr() as usize }
}

pub fn phys_to_ptr<T>(p: u64) -> *const T {
    (p + KERNBASE as u64) as *const T
}

pub fn ref_to_phys<T>(v: &T) -> u64 {
    ptr_to_phys(v)
}

pub fn ptr_to_phys<T>(v: *const T) -> u64 {
    (v as usize - KERNBASE) as u64
}

pub fn phys_to_addr(p: u64) -> usize {
    p as usize + KERNBASE
}

pub fn addr_to_phys(a: usize) -> u64 {
    (a - KERNBASE) as u64
}

pub unsafe fn phys_to_ref<T>(p: u64) -> &'static T {
    &*phys_to_ptr(p)
}

pub fn phys_to_ptr_mut<T>(p: u64) -> *mut T {
    (p + KERNBASE as u64) as *mut T
}

pub unsafe fn _phys_to_mut<T>(p: u64) -> &'static mut T {
    &mut *phys_to_ptr_mut(p)
}

#[repr(C, packed)]
struct BootInfo {
    flags: u32,
    _unused0: [u32; 3],
    cmdline: u32,
    _unused1: [u32; 6],
    memmap_len: u32,
    memmap_addr: u32,
    _unused2: [u32; 18],
}

unsafe fn addr_to_boot_info(addr: usize) -> &'static BootInfo {
    &*(addr as *const BootInfo)
}

pub unsafe fn early_init(boot_info_phys: u64) {
    let boot_info_addr = phys_to_addr(boot_info_phys);
    let boot_info = addr_to_boot_info(boot_info_addr);
    if boot_info.flags & (1 << 6) != 0 {
        let region = core::slice::from_raw_parts(
            phys_to_ptr::<u8>(boot_info.memmap_addr.into()),
            boot_info.memmap_len as usize,
        );
        let mem_map_iter = MemMapIterator { region, offset: 0 };
        for entry in mem_map_iter {
            MEM_MAP[MEM_MAP_NENTRIES] = entry;
            MEM_MAP_NENTRIES += 1;
        }
    }
}
pub fn mem_map<'a>() -> &'a [MemMapEntry] {
    unsafe { &MEM_MAP[..MEM_MAP_NENTRIES] }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MemType {
    Nothing,
    Memory,
    System,
    Reserved,
}

#[derive(Clone, Copy, Debug)]
pub struct MemMapEntry {
    pub start: u64,
    pub end: u64,
    pub typ: MemType,
}

const EMPTY_MAP_ENTRY: MemMapEntry = MemMapEntry {
    start: 0,
    end: 0,
    typ: MemType::Nothing,
};

static mut MEM_MAP: [MemMapEntry; 256] = [EMPTY_MAP_ENTRY; 256];
static mut MEM_MAP_NENTRIES: usize = 0;

pub struct MemMapIterator<'a> {
    region: &'a [u8],
    offset: usize,
}

impl<'a> Iterator for MemMapIterator<'a> {
    type Item = MemMapEntry;

    fn next(&mut self) -> Option<MemMapEntry> {
        const ENTRY_SIZE: usize = 24;
        if self.offset >= self.region.len() || self.region.len() - self.offset < ENTRY_SIZE {
            return None;
        }
        let bs = &self.region[self.offset..self.offset + ENTRY_SIZE];
        let size = arch::read_u32(&bs[0..4]) as usize;
        self.offset += size + 4;
        let start = arch::read_u64(&bs[4..12]);
        let len = arch::read_u64(&bs[12..20]);
        let typ = match arch::read_u32(&bs[20..24]) {
            1 => MemType::Memory,
            2 | 3 | 4 => MemType::System,
            _ => MemType::Reserved,
        };
        Some(MemMapEntry {
            start,
            end: start + len,
            typ,
        })
    }
}

pub const DEVSPACE: u64 = 0xFE00_0000;
pub const GIG4: u64 = 0x1_0000_0000;
pub const EARLY_FREE_END: u64 = 128 * 1024 * 1024;

pub unsafe fn page_slice_mut<'a>(pstart: *mut Page, pend: *mut Page) -> &'a mut [Page] {
    let ustart = pstart as usize;
    let uend = pend as usize;
    assert_eq!(
        ustart % PAGE_SIZE,
        0,
        "page_slice_mut: unaligned start page"
    );
    assert_eq!(uend % PAGE_SIZE, 0, "page_slice_mut: unaligned end page");
    assert!(ustart < uend, "page_slice_mut: bad range");

    let len = (uend - ustart) / PAGE_SIZE;
    slice::from_raw_parts_mut(ustart as *mut Page, len)
}

pub fn early_pages() -> &'static mut [Page] {
    let pend = end_addr() as *mut Page;
    unsafe { page_slice_mut(pend, phys_to_ptr_mut(EARLY_FREE_END)) }
}

pub unsafe fn init() {
    use core::cmp;
    use core::ops::Range;

    unsafe fn phys_to_page_slice_mut(r: Range<u64>) -> &'static mut [Page] {
        page_slice_mut(phys_to_ptr_mut(r.start), phys_to_ptr_mut(r.end))
    }

    use crate::kalloc::free_pages;
    for entry in mem_map() {
        if entry.typ != MemType::Memory || entry.end < EARLY_FREE_END {
            continue;
        }
        let start_phys = cmp::max(entry.start, EARLY_FREE_END);
        free_pages(phys_to_page_slice_mut(start_phys..entry.end));
    }
    // Also free conventional memory.
    free_pages(phys_to_page_slice_mut(0x0_1000..0x0_7000));
    free_pages(phys_to_page_slice_mut(0x0_8000..0x8_0000));
}
