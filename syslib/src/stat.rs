#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileType {
    Unused = 0,
    Dir = 1,
    File = 2,
    Dev = 3,
}

pub struct Stat {
    typ: FileType,
    dev: u32,
    ino: u64,
    nlink: u32,
    size: u64,
}
