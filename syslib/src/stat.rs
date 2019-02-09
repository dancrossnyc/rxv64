#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileType {
    Unused = 0,
    Dir = 1,
    File = 2,
    Dev = 3,
}

pub struct Stat {
    pub typ: FileType,
    pub dev: u32,
    pub ino: u64,
    pub nlink: u32,
    pub size: u64,
}
