use crate::proc::myproc;
use crate::Result;
use core::mem;
use syslib::stat::Stat;

pub fn stat(fd: usize, addr: usize) -> Result<()> {
    let curproc = myproc();
    let file = curproc.get_fd(fd).ok_or("bad file")?;
    let sb = file.stat()?;
    let user_sb_sl = curproc
        .fetch_slice_mut(addr, mem::size_of::<Stat>())
        .ok_or("bad pointer")?;
    let user_sb = unsafe { &mut *(user_sb_sl.as_mut_ptr() as usize as *mut Stat) };
    *user_sb = sb;
    Ok(())
}

pub fn write(fd: usize, addr: usize, len: usize) -> Result<usize> {
    let curproc = myproc();
    let file = curproc.get_fd(fd).ok_or("bad file")?;
    let buf = curproc.fetch_slice(addr, len).ok_or("bad pointer")?;
    file.write(buf)
}

pub fn read(fd: usize, addr: usize, len: usize) -> Result<usize> {
    let curproc = myproc();
    let file = curproc.get_fd(fd).ok_or("bad file")?;
    let buf = curproc.fetch_slice_mut(addr, len).ok_or("bad pointer")?;
    file.read(buf)
}
