use crate::bio;
use crate::fs;
use crate::param;
use crate::spinlock::SpinMutex as Mutex;
use core::intrinsics::volatile_copy_memory;
use static_assertions::const_assert;

/// Contents of the header block, used for both the
/// stored header block and keeping track of logged
/// block numbers in memory before commit.
#[repr(C)]
struct LogHeader {
    len: u64,
    blocks: [u64; param::LOGSIZE],
}

impl LogHeader {
    pub const fn new() -> LogHeader {
        LogHeader {
            len: 0,
            blocks: [0; param::LOGSIZE],
        }
    }
}
const_assert!(core::mem::size_of::<LogHeader>() <= fs::BSIZE);

/// Simple logging that allows concurrent FS system calls.
///
/// A log transaction contains the updates of multiple FS system
/// calls. The logging system only commits when there are
/// no FS system calls active. Thus there is never
/// any reasoning required about whether a commit might
/// write an uncommitted system call's updates to disk.
///
/// A system call should call begin_op()/end_op() to mark
/// its start and end. Usually begin_op() just increments
/// the count of in-progress FS system calls and returns.
/// But if it thinks the log is close to running out, it
/// sleeps until the last outstanding end_op() commits.
///
/// The log is a physical re-do log containing disk blocks.
/// The on-disk log format:
///   header block, containing block #s for block A, B, C, ...
///   block A
///   block B
///   block C
///   ...
/// Log appends are synchronous.
pub struct Log {
    start: u64,
    size: usize,
    outstanding: usize,
    committing: bool,
    dev: u32,
    header: LogHeader,
}

impl Log {
    pub const fn new() -> Log {
        Log {
            start: 0,
            size: 0,
            outstanding: 0,
            committing: false,
            dev: 0,
            header: LogHeader::new(),
        }
    }
}

impl Log {
    pub fn read_header(&self) {}
    pub fn write_header() {}
    pub fn install_transaction(&self) {
        for tail in 0..self.header.len {
            bio::with_block(self.dev, self.start + tail + 1, |log_buf| {
                bio::with_block(self.dev, self.header.blocks[tail as usize], |sd_buf| {
                    let dst = sd_buf.data_mut().as_mut_ptr();
                    let src = log_buf.data_ref().as_ptr();
                    unsafe {
                        volatile_copy_memory(dst, src, fs::BSIZE);
                    }
                    sd_buf.write();
                })
                .unwrap();
            })
            .unwrap();
        }
    }
    pub fn recover(&self) {}
}

static LOG: Mutex<Log> = Mutex::new("log", Log::new());

pub unsafe fn init(dev: u32, sb: &fs::Superblock) {
    let mut log = LOG.lock();
    log.start = sb.log_start;
    log.size = sb.nlog as usize;
    log.dev = dev;
    log.recover();
}

pub fn write(bp: &bio::Buf) {
    assert!(bp.is_locked());
}

pub struct Op {}
impl Op {
    pub fn begin() {}
    pub fn end() {}
}

pub fn with_op<U, F: FnMut() -> U>(mut thunk: F) -> U {
    Op::begin();
    let r = thunk();
    Op::end();
    r
}
