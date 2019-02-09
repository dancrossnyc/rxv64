use crate::arch;
use crate::arch::Page;
use crate::kalloc;
use crate::kmem;
use crate::param;
use crate::Result;
use bitflags::bitflags;
use core::cmp;
use core::fmt;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr::{null_mut, write_volatile};

bitflags! {
    pub struct PageFlags: u64 {
        const PRESENT = 1;
        const WRITE   = 1 << 1;
        const USER    = 1 << 2;
        const WRTHRU  = 1 << 3;
        const NOCACHE = 1 << 4;
        const ACCESS  = 1 << 5;
        const DIRTY   = 1 << 6;
        const HUGE    = 1 << 7;
        const GLOBAL  = 1 << 8;
        const NX      = 1 << 63;
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(transparent)]
pub struct Entry(u64);

impl Entry {
    const PHYS_PAGE_MASK: u64 = 0x0000_7FFF_FFFF_F000;

    pub const fn empty() -> Entry {
        Entry(0)
    }

    fn new(pa: u64, flags: PageFlags) -> Entry {
        Entry((pa & Self::PHYS_PAGE_MASK) | flags.bits())
    }

    fn flags(self) -> PageFlags {
        PageFlags::from_bits_truncate(self.0)
    }

    fn is_present(self) -> bool {
        self.flags().contains(PageFlags::PRESENT)
    }

    fn is_user(self) -> bool {
        self.flags().contains(PageFlags::USER)
    }

    fn is_zero(self) -> bool {
        self.0 == 0
    }

    fn phys_page_addr(self) -> u64 {
        self.0 & Self::PHYS_PAGE_MASK
    }

    fn _disable_user(&mut self) {
        self.0 &= !PageFlags::USER.bits;
    }

    fn enable(&mut self) {
        self.0 |= PageFlags::PRESENT.bits;
    }

    fn clear(&mut self) {
        self.0 = 0;
    }

    fn virt_page_addr(self) -> usize {
        self.phys_page_addr() as usize + param::KERNBASE
    }
}

pub enum Level4 {}
pub enum Level3 {}
pub enum Level2 {}
pub enum Level1 {}

pub trait Node {
    fn index(va: usize) -> usize;
}

impl Node for Level4 {
    fn index(va: usize) -> usize {
        (va >> 39) & 0x1FF
    }
}

impl Node for Level3 {
    fn index(va: usize) -> usize {
        (va >> 30) & 0x1FF
    }
}

impl Node for Level2 {
    fn index(va: usize) -> usize {
        (va >> 21) & 0x1FF
    }
}

impl Node for Level1 {
    fn index(va: usize) -> usize {
        (va >> 12) & 0x1FF
    }
}

pub trait Level: Node {
    type EntryType: Node;
}

impl Level for Level4 {
    type EntryType = Level3;
}

impl Level for Level3 {
    type EntryType = Level2;
}

impl Level for Level2 {
    type EntryType = Level1;
}

// pub trait FreeUserPages: Level {
//     fn free_user_pages(&mut self, start: usize, end: usize) {
//         assert_eq!(start % arch::PAGE_SIZE, 0);
//         assert_eq!(end % arch::PAGE_SIZE, 0);
//         let si = Self::EntryType::index(start);
//         let ei = Self::EntryType::index(end);
//         for entry in self.entries[si..=ei].iter_mut().filter(|e| e.is_present()) {
//             let raw_ptr = entry.virt_page_addr();
//             let next_table = unsafe { &mut *(raw_ptr as *mut Table<Level2>) };
//             next_table.free_user_pages(start, end);
//             if next_table.is_empty() {
//                 kalloc::free(unsafe { &mut *(raw_ptr as *mut arch::Page) });
//                 entry.clear();
//             }
//         }
//     }
// }

#[repr(C, align(4096))]
pub struct Table<L>
where
    L: Node,
{
    entries: [Entry; 512],
    level: PhantomData<L>,
}

impl<L> Table<L>
where
    L: Level,
{
    fn next(&self, va: usize) -> Option<&Table<L::EntryType>> {
        let entry = self.entries[L::index(va)];
        if !entry.is_present() {
            return None;
        }
        let raw_ptr = entry.virt_page_addr();
        Some(unsafe { &*(raw_ptr as *const Table<L::EntryType>) })
    }

    fn next_mut(&mut self, va: usize) -> Option<&mut Table<L::EntryType>> {
        let index = L::index(va);
        let mut entry = self.entries[index];
        if !entry.is_present() {
            let page = kalloc::alloc()?;
            page.clear();
            let flags = PageFlags::PRESENT | PageFlags::USER | PageFlags::WRITE;
            entry = Entry::new(page.phys_addr(), flags);
            unsafe {
                write_volatile(&mut self.entries[index], entry);
            }
        }
        let raw_ptr = entry.virt_page_addr();
        Some(unsafe { &mut *(raw_ptr as *mut Table<L::EntryType>) })
    }

    fn is_empty(&self) -> bool {
        self.entries.iter().all(|entry| entry.is_zero())
    }

    // fn free_user(&mut self) {
    //     for entry in self.entries.iter_mut().take_while(|entry| !entry.is_zero()) {
    //         let raw_ptr = entry.virt_page_addr();
    //         let next_table = unsafe { &mut *(raw_ptr as *mut Table<L::EntryType>) };
    //         next_table.free_user();
    //         kalloc::free(unsafe { &mut *(raw_ptr as *mut arch::Page) });
    //     }
    // }
}

impl Table<Level3> {
    fn free_user_pages(&mut self, start: usize, end: usize) {
        assert_eq!(start % arch::PAGE_SIZE, 0);
        assert_eq!(end % arch::PAGE_SIZE, 0);
        let si = Level3::index(start);
        let ei = Level3::index(end);
        for entry in self.entries[si..=ei].iter_mut().filter(|e| e.is_present()) {
            let raw_ptr = entry.virt_page_addr();
            let next_table = unsafe { &mut *(raw_ptr as *mut Table<Level2>) };
            next_table.free_user_pages(start, end);
            if next_table.is_empty() {
                kalloc::free(unsafe { &mut *(raw_ptr as *mut arch::Page) });
                entry.clear();
            }
        }
    }
}

impl Table<Level2> {
    fn free_user_pages(&mut self, start: usize, end: usize) {
        assert_eq!(start % arch::PAGE_SIZE, 0);
        assert_eq!(end % arch::PAGE_SIZE, 0);
        let si = Level3::index(start);
        let ei = Level3::index(end);
        for entry in self.entries[si..=ei].iter_mut().filter(|e| e.is_present()) {
            let raw_ptr = entry.virt_page_addr();
            let next_table = unsafe { &mut *(raw_ptr as *mut Table<Level1>) };
            next_table.free_user_pages(start, end);
            if next_table.is_empty() {
                kalloc::free(unsafe { &mut *(raw_ptr as *mut arch::Page) });
                entry.clear();
            }
        }
    }
}

impl Table<Level1> {
    pub fn entry(&self, va: usize) -> Option<Entry> {
        let entry = self.entries[Level1::index(va)];
        if entry.is_present() {
            Some(entry)
        } else {
            None
        }
    }

    pub fn entry_mut(&mut self, va: usize) -> Option<&mut Entry> {
        Some(&mut self.entries[Level1::index(va)])
    }

    fn is_empty(&self) -> bool {
        self.entries.iter().all(|entry| entry.is_zero())
    }

    fn free_user_pages(&mut self, start: usize, end: usize) {
        assert_eq!(start % arch::PAGE_SIZE, 0);
        assert_eq!(end % arch::PAGE_SIZE, 0);
        let si = Level1::index(start);
        let ei = Level1::index(end);
        for entry in self.entries[si..=ei].iter_mut().filter(|e| e.is_present()) {
            let raw_ptr = entry.virt_page_addr();
            kalloc::free(unsafe { &mut *(raw_ptr as *mut arch::Page) });
            entry.clear();
        }
    }
}

pub type PageTable = Table<Level4>;

impl PageTable {
    pub const fn empty() -> PageTable {
        PageTable {
            entries: [Entry::empty(); 512],
            level: PhantomData,
        }
    }

    #[allow(dead_code)]
    pub fn translate(&self, va: usize) -> Option<u64> {
        let entry = self
            .next(va)
            .and_then(|p3| p3.next(va))
            .and_then(|p2| p2.next(va))
            .and_then(|p1| p1.entry(va))?;
        let phys_addr = entry.phys_page_addr() + (va % arch::PAGE_SIZE) as u64;
        Some(phys_addr)
    }

    pub fn entry_for(&self, va: usize) -> Option<Entry> {
        self.next(va)
            .and_then(|p3| p3.next(va))
            .and_then(|p2| p2.next(va))
            .and_then(|p1| p1.entry(va))
    }

    pub fn map_to(&mut self, pa: u64, va: usize, flags: PageFlags) -> Result<()> {
        if let Some(entry) = self
            .next_mut(va)
            .and_then(|p3| p3.next_mut(va))
            .and_then(|p2| p2.next_mut(va))
            .and_then(|p1| p1.entry_mut(va))
        {
            let mut new_entry = Entry::new(pa, flags);
            new_entry.enable();
            unsafe {
                write_volatile(entry, new_entry);
            }
            return Ok(());
        }
        Err("Allocation failed")
    }

    pub fn map_phys_range(&mut self, start: u64, end: u64, flags: PageFlags) -> Result<()> {
        for pa in (start..=end).step_by(arch::PAGE_SIZE) {
            self.map_to(pa, kmem::phys_to_addr(pa), flags)?;
        }
        Ok(())
    }

    pub fn dup_kern(&self) -> Option<&'static mut PageTable> {
        let l4page = kalloc::alloc()?;
        l4page.clear();
        let table = unsafe { &mut *(l4page as *mut _ as *mut PageTable) };
        // Copy kernel portion.
        table.entries[256..512].clone_from_slice(&self.entries[256..512]);
        Some(table)
    }

    pub fn dup(&self, size: usize) -> Option<&'static mut PageTable> {
        let table = self.dup_kern()?;
        for k in (0usize..size).step_by(arch::PAGE_SIZE) {
            let entry = self.entry_for(k).expect("entry should exist");
            assert!(entry.is_present(), "dup: page not present");
            let page = kalloc::alloc();
            if page.is_none() {
                free(table);
                return None;
            }
            let page = page.unwrap();
            unsafe {
                use core::intrinsics::volatile_copy_memory;
                let src = entry.virt_page_addr() as *const arch::Page;
                volatile_copy_memory(page, src, 1);
            }
            if table
                .map_to(entry.phys_page_addr(), k, entry.flags())
                .is_err()
            {
                kalloc::free(page);
                free(table);
                return None;
            }
        }
        Some(table)
    }

    pub fn alloc_user(
        &mut self,
        old_size: usize,
        new_size: usize,
        flags: PageFlags,
    ) -> Result<usize> {
        if new_size > param::KERNBASE {
            return Err("alloc_user: new size extends into kernel");
        }
        if new_size < old_size {
            return Ok(old_size);
        }
        for user_addr in (arch::page_round_up(old_size)..new_size).step_by(arch::PAGE_SIZE) {
            let page = kalloc::alloc();
            if page.is_none() {
                self.dealloc_user(new_size, old_size).expect("user dealloc");
                return Err("alloc_user: failed to alloc user page");
            }
            let page = page.unwrap();
            page.clear();
            if let Err(status) = self.map_to(page.phys_addr(), user_addr, flags) {
                self.dealloc_user(new_size, old_size).expect("user dealloc");
                return Err(status);
            }
        }
        Ok(new_size)
    }

    pub fn dealloc_user(&mut self, old_size: usize, new_size: usize) -> Result<usize> {
        if new_size >= old_size {
            return Ok(old_size);
        }
        self.free_user_pages(new_size, arch::page_round_up(old_size));
        Ok(new_size)
    }

    pub fn free_user_pages(&mut self, start: usize, end: usize) {
        let start = arch::page_round_up(start);
        let end = arch::page_round_down(end);
        let si = Level4::index(start);
        let ei = cmp::min(Level4::index(end), 255);
        for entry in self.entries[si..=ei].iter_mut().filter(|e| e.is_present()) {
            let raw_ptr = entry.virt_page_addr();
            let next_table = unsafe { &mut *(raw_ptr as *mut Table<Level3>) };
            next_table.free_user_pages(start, end);
            if next_table.is_empty() {
                kalloc::free(unsafe { &mut *(raw_ptr as *mut arch::Page) });
                entry.clear();
            }
        }
    }

    pub fn user_addr_to_kern_page(&self, va: usize) -> Result<&'static mut Page> {
        let entry = self.entry_for(va).ok_or("no mapping for user address")?;
        if !entry.is_present() || !entry.is_user() {
            return Err("bad user address");
        }
        Ok(unsafe { &mut *(entry.virt_page_addr() as *mut Page) })
    }

    pub fn copy_out(&self, mut data: &[u8], mut va: usize) -> Result<()> {
        let mut len = data.len();
        while len > 0 {
            let va0 = arch::page_round_down(va);
            let dst = self.user_addr_to_kern_page(va0)?.as_mut();
            let off = va - va0;
            let n = cmp::min(arch::PAGE_SIZE - off, len);
            dst[off..off + n].clone_from_slice(&data[..n]);
            va = va0 + arch::PAGE_SIZE;
            len -= n;
            data = &data[n..];
        }
        Ok(())
    }
}

impl fmt::Debug for PageTable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:x}", self as *const _ as usize)
    }
}

unsafe fn init_pat() {
    use arch::wrmsr;
    const PAT_BITS: u64 = 0x0007_0406_0007_0406;
    const MSR_PAT: u32 = 0x277;
    wrmsr(MSR_PAT, PAT_BITS);
}

pub unsafe fn init(kpage_table: &mut PageTable) {
    use PageFlags as PF;

    init_pat();

    let text_phys = kmem::addr_to_phys(kmem::text_addr());
    let etext_phys = kmem::addr_to_phys(kmem::etext_addr());
    let erodata_phys = kmem::addr_to_phys(kmem::erodata_addr());
    let end_phys = kmem::addr_to_phys(kmem::end_addr());

    let devflags = PF::WRITE | PF::WRTHRU | PF::NOCACHE | PF::NX;
    for entry in kmem::mem_map().iter() {
        let flags = match entry.typ {
            kmem::MemType::Memory => PF::WRITE | PF::NX,
            kmem::MemType::System => devflags,
            _ => continue,
        };
        if entry.end < end_phys {
            continue;
        }
        let start_phys = cmp::max(entry.start, end_phys);
        kpage_table
            .map_phys_range(start_phys, entry.end, flags)
            .expect("init map failed");
    }

    let custom_map = [
        // x86_64 low memory regions and the kernel are mapped
        // specially.
        //
        // Note that the first page is left unmapped to try and
        // catch null pointer dereferences in unsafe code: defense
        // in depth!
        //
        // While it may seem paradoxical that we map the SIPI
        // page non-executable, bear in mind that the APs
        // will execute the code on that page in real mode,
        // 32-bit protected mode using segmentation and then a
        // flat physical mapping, and finally long mode, but
        // on the initial boot memory map.  The APs will then
        // jump into Rust code at some normal link address,
        // at which point they will load this address space
        // and not touch the SIPI page again.
        //
        // That is, nothing will ever execute the code
        // written to that page via the address space
        // described here.  Indeed, to do so would be a
        // grievous error; hence, mapping non-execute.
        //
        // Low memory.
        (0x0_1000, 0x0_7000, PF::WRITE | PF::NX),
        // SIPI page.  Never freed to kalloc.
        (0x0_7000, 0x0_8000, PF::WRITE | PF::NX),
        // Conventional RAM
        (0x0_8000, 0x8_0000, PF::WRITE | PF::NX),
        // BIOS data
        (0x8_0000, 0xA_0000, PF::NX),
        // VGA buffer
        (0xA_0000, 0xC_0000, devflags),
        // Who knows?
        (0xC_0000, 0x10_0000, PF::NX),
        // Kernel text
        (text_phys, etext_phys, PF::empty()),
        // Kernel read-only data
        (etext_phys, erodata_phys, PF::NX),
        // Kernel BSS
        (erodata_phys, end_phys, PF::WRITE | PF::NX),
        // Kernel heap
        (end_phys, 64 * 1024 * 1024, PF::WRITE | PF::NX),
        // Device space (LAPIC, IOAPIC, etc)
        (kmem::DEVSPACE, kmem::GIG4, devflags),
    ];

    for (start, end, flags) in custom_map.iter() {
        kpage_table
            .map_phys_range(*start, *end, *flags)
            .expect("init mapping failed");
    }
}

pub struct PageTableGuard {
    pgtbl: *mut PageTable,
}

impl PageTableGuard {
    pub fn release(&mut self) -> &'static mut PageTable {
        let pgtbl = unsafe { self.pgtbl.as_mut().unwrap() };
        self.pgtbl = null_mut();
        pgtbl
    }
}

impl Drop for PageTableGuard {
    fn drop(&mut self) {
        if !self.pgtbl.is_null() {
            free(unsafe { self.pgtbl.as_mut().unwrap() });
        }
    }
}

impl Deref for PageTableGuard {
    type Target = PageTable;
    fn deref(&self) -> &Self::Target {
        unsafe { self.pgtbl.as_ref().unwrap() }
    }
}

impl DerefMut for PageTableGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.pgtbl.as_mut().unwrap() }
    }
}

pub fn new_pgtbl() -> Result<PageTableGuard> {
    let pgtbl = unsafe {
        crate::KPGTBL
            .dup_kern()
            .ok_or("exec: cannot allocate new page table")?
    };
    Ok(PageTableGuard { pgtbl })
}

pub unsafe fn switch(kpage_table: &PageTable) {
    arch::load_page_table(kmem::ref_to_phys(kpage_table));
}

pub fn free(pgtbl: &mut PageTable) {
    pgtbl.free_user_pages(0, !0);
    let raw_ptr = pgtbl as *mut PageTable as usize;
    kalloc::free(unsafe { &mut *(raw_ptr as *mut arch::Page) });
}
