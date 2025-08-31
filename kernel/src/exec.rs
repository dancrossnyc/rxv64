use crate::Result;
use crate::arch;
use crate::fs;
use crate::fslog;
use crate::param;
use crate::proc;
use crate::vm;
use core::cmp;
use core::mem;
use core::slice;

const NIDENT: usize = 16;

// The ELF header and Program Header are taken from the System V
// interface definition specification.  For details, see the
// references at:
//
// http://www.sco.com/developers/gabi/latest/ch4.eheader.html
// http://www.sco.com/developers/gabi/latest/ch5.pheader.html
//
// We use the 64-bit type definitions, details of which are
// specified here:
//
// https://www.sco.com/developers/gabi/latest/ch4.intro.html#data_representation
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ELFHeader {
    ident: [u8; NIDENT],
    object_file_type: u16,
    machine: u16,
    version: u32,
    entry_addr: u64,
    program_header_offset: u64,
    section_header_offset: u64,
    flags: u32,
    elf_header_size: u16,
    program_header_entry_size: u16,
    num_program_headers: u16,
    section_header_entry_size: u16,
    num_section_headers: u16,
    name_strings_section_index: u16,
}

impl ELFHeader {
    fn read(ip: &fs::Inode) -> Result<ELFHeader> {
        let mut header = [ELFHeader::default(); 1];
        if ip.readi(&mut header[..], 0)? != mem::size_of::<ELFHeader>() {
            return Err("exec: short ELF file");
        }
        Ok(header[0])
    }

    fn validate(&self) -> Result<()> {
        if &self.ident[..4] != b"\x7FELF" {
            return Err("Bad magic ELF value");
        }
        const CLASS_64_BIT: u8 = 2;
        if self.ident[4] != CLASS_64_BIT {
            return Err("Not a 64-bit object file");
        }
        const OBJECT_FILE_TYPE_EXEC: u16 = 2;
        if self.object_file_type != OBJECT_FILE_TYPE_EXEC {
            return Err("Not an executable ELF file");
        }
        const MACHINE_X86_64: u16 = 62;
        if self.machine != MACHINE_X86_64 {
            return Err("Wrong ELF executable architecture");
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProgramHeader {
    prog_type: u32,
    flags: u32,
    offset: u64,
    virt_addr: u64,
    _phys_addr: u64,
    file_size: u64,
    mem_size: u64,
    align: u64,
}
const PH_SIZE: usize = mem::size_of::<ProgramHeader>();

impl ProgramHeader {
    fn read(ip: &fs::Inode, off: u64) -> Result<ProgramHeader> {
        let mut header = [ProgramHeader::default(); 1];
        if ip.readi(&mut header[..], off)? != PH_SIZE {
            return Err("exec: short program header read");
        }
        Ok(header[0])
    }

    fn validate(&self) -> Result<()> {
        if self.mem_size < self.file_size {
            return Err("exec: file and memory size mismatch");
        }
        if !self.virt_addr.is_multiple_of(arch::PAGE_SIZE as u64) {
            return Err("exec: misaligned section load address");
        }
        if self.virt_addr.wrapping_add(self.mem_size) < self.virt_addr {
            return Err("exec: program section too big");
        }
        Ok(())
    }

    fn is_loadable(&self) -> bool {
        const PROG_TYPE_LOAD: u32 = 1;
        self.prog_type == PROG_TYPE_LOAD
    }

    fn page_flags(&self) -> vm::PageFlags {
        const PF_X: u32 = 1;
        const PF_W: u32 = 1 << 1;
        const _PF_R: u32 = 1 << 2;
        let mut flags = vm::PageFlags::USER | vm::PageFlags::NX;
        if self.flags & PF_W == PF_W {
            flags.insert(vm::PageFlags::WRITE);
        }
        if self.flags & PF_X == PF_X {
            flags.remove(vm::PageFlags::NX);
        }
        flags
    }

    fn page_alloc_user(&self, pgtbl: &mut vm::PageTable, size: usize) -> Result<usize> {
        pgtbl.alloc_user(
            size,
            (self.virt_addr + self.mem_size) as usize,
            self.page_flags(),
        )
    }

    fn load_section(&self, pgtbl: &mut vm::PageTable, ip: &fs::Inode) -> Result<()> {
        let va = self.virt_addr as usize;
        assert_eq!(va % arch::PAGE_SIZE, 0);
        let file_size = self.file_size as usize;
        for kp in (0..file_size).step_by(arch::PAGE_SIZE) {
            let page = pgtbl.user_addr_to_kern_page(va + kp)?;
            let n = cmp::min(file_size - kp, arch::PAGE_SIZE);
            if ip.readi(&mut page.as_mut()[..n], self.offset + kp as u64)? != n {
                return Err("loaduvm: short read from file");
            }
        }
        Ok(())
    }
}

pub fn exec(proc: &proc::Proc, path: &[u8], args: &[&[u8]]) -> Result<()> {
    if args.len() > param::MAXARG {
        return Err("exec: too many arguments");
    }

    let mut pgtbl = vm::new_pgtbl()?;
    let mut size = 0;

    // Load the program into memory.
    let entry_addr = fslog::with_op(|| {
        let ip = fs::namei(path)?;
        ip.with_putlock(|ip| {
            let elf = ELFHeader::read(ip)?;
            elf.validate()?;
            let mut off = elf.program_header_offset;
            for _ in 0..elf.num_program_headers {
                let ph = ProgramHeader::read(ip, off)?;
                off += PH_SIZE as u64;
                if !ph.is_loadable() {
                    continue;
                }
                ph.validate()?;
                size = ph.page_alloc_user(&mut pgtbl, size)?;
                ph.load_section(&mut pgtbl, ip)?;
            }
            Ok(elf.entry_addr)
        })
    })?;

    // Allocate the stack at the top of the user portion of the
    // virtual address space.
    pgtbl.alloc_user(
        param::USERSTACK,
        param::USEREND,
        vm::PageFlags::WRITE | vm::PageFlags::NX,
    )?;

    // Copy arguments onto stack.
    let mut uargs = [0usize; param::MAXARG + 1];
    let uargs = &mut uargs[..args.len()];
    let mut sp = param::USEREND;
    for (k, &arg) in args.iter().enumerate() {
        sp -= arg.len() + 1;
        sp &= !0b111;
        uargs[k] = sp;
        pgtbl.copy_out(arg, sp)?;
        if sp < param::USERSTACK {
            return Err("exec: arg stack overflow");
        }
    }

    // Copy in the argument pointer vector.
    let bytes = slice_as_bytes(uargs);
    sp -= bytes.len();
    pgtbl.copy_out(bytes, sp)?;
    let argc = args.len();
    let argv = sp;

    // Align the stack and push a dummy frame pointer.
    if sp & 0b1111 == 0 {
        let bytes = 0usize.to_ne_bytes();
        sp -= bytes.len();
        pgtbl.copy_out(&bytes, sp)?;
    }
    let bytes = (!0usize).to_ne_bytes();
    sp -= bytes.len();
    pgtbl.copy_out(&bytes, sp)?;

    // XXX Copy in the name

    // Commit to the new page table.
    let previous = unsafe { proc.switch_pgtbl(pgtbl) };
    proc.set_size(size);
    drop(previous);

    // Set up for return to userspace.
    unsafe {
        let uctx = proc.user_context_mut();
        uctx.set_return(core::mem::transmute::<u64, extern "C" fn() -> u32>(
            entry_addr,
        ));
        uctx.set_rdi(argc as u64);
        uctx.set_rsi(argv as u64);
        uctx.set_stack(sp as u64);
    }

    Ok(())
}

fn slice_as_bytes<T>(s: &[T]) -> &[u8] {
    let len = core::mem::size_of_val(s);
    let ptr = s.as_ptr() as *const u8;
    unsafe { slice::from_raw_parts(ptr, len) }
}
