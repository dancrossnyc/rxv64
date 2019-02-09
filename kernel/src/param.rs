#![allow(dead_code)]

pub const KERNBASE: usize = 0xFFFF_8000_0000_0000;
pub const USERSTACK: usize = 0x0000_7FFF_FFFF_C000;
pub const USEREND: usize = 0x0000_8000_0000_0000;
pub const NPROC: usize = 256;
pub const NPCICFGMAX: usize = 256;
pub const NCPUMAX: usize = 256;
pub const NOFILE: usize = 64;
pub const NFILE: usize = 1024;
pub const NINODE: usize = 1024;
pub const NDEV: usize = 128;
pub const ROOTDEV: u32 = 1;
pub const MAXARG: usize = 32;
pub const MAXOPBLOCKS: usize = 64;
pub const LOGSIZE: usize = MAXOPBLOCKS * (8 - 1);
pub const NBUF: usize = MAXOPBLOCKS * 8;
pub const FSSIZE: usize = 1024;
