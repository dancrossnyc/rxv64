use core::ptr;

use crate::arch::{Page, PAGE_SIZE};
use crate::spinlock::SpinMutex as Mutex;

static FREE_LIST: Mutex<FreeList> = Mutex::new("kmem", FreeList { next: None });

#[repr(align(4096))]
struct FreeList {
    next: Option<ptr::NonNull<FreeList>>,
}
unsafe impl Send for FreeList {}

impl FreeList {
    pub fn put(&mut self, page: &mut Page) {
        let ptr = page as *mut _ as usize;
        assert_eq!(ptr % PAGE_SIZE, 0, "freeing unaligned page");
        page.scribble();
        let f = page as *mut Page as *mut FreeList;
        unsafe {
            ptr::write(f, FreeList { next: self.next });
        }
        self.next = ptr::NonNull::new(f);
    }

    pub fn get(&mut self) -> Option<&'static mut Page> {
        let mut next = self.next?;
        let next = unsafe { next.as_mut() };
        self.next = next.next;
        let pg = unsafe { &mut *(next as *mut FreeList as *mut Page) };
        Some(pg)
    }
}

pub unsafe fn early_init(pages: &mut [Page]) {
    free_pages(pages);
}

pub fn free_pages(pages: &mut [Page]) {
    let mut fl = FREE_LIST.lock();
    for page in pages.iter_mut() {
        fl.put(page);
    }
}

pub fn free(page: &mut Page) {
    FREE_LIST.lock().put(page);
}

pub fn alloc() -> Option<&'static mut Page> {
    FREE_LIST.lock().get()
}
