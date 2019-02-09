use crate::arch;
use crate::kmem;
use core::mem;
use core::slice;
use static_assertions::const_assert_eq;

pub fn checksum(table: &[u8]) -> u8 {
    table.iter().fold(0u8, |sum, x| sum.wrapping_add(*x))
}

struct Table {
    // These fields are the fixed and common headers.
    signature: [u8; 4],
    length: [u8; 4],
    revision: u8,
    _checksum: u8,
    oem_id: [u8; 6],
    _oem_table_id: [u8; 8],
    _oem_revision: [u8; 4],
    _creator_id: [u8; 4],
    _creator_revision: [u8; 4],
}
const_assert_eq!(mem::size_of::<Table>(), 36);

impl Table {
    unsafe fn new(phys_addr: u64) -> &'static Table {
        let tbl = kmem::phys_to_ref::<Table>(phys_addr);
        assert_eq!(checksum(tbl.as_bytes()), 0, "corrupt ACPI table");
        tbl
    }

    fn signature(&self) -> &[u8] {
        &self.signature
    }

    unsafe fn as_bytes(&self) -> &[u8] {
        let ptr = self as *const _ as *const u8;
        slice::from_raw_parts(ptr, self.len())
    }

    fn len(&self) -> usize {
        u32::from_le_bytes(self.length) as usize
    }

    unsafe fn data(&self) -> &[u8] {
        let ptr = self as *const Table as *const u8;
        let table_slice = slice::from_raw_parts(ptr, self.len());
        &table_slice[mem::size_of::<Table>()..]
    }
}

//#[derive(Clone, Copy)]
pub enum ACPIVersion {
    V1,
    V2,
}

pub unsafe fn init() {
    let sdt = RSDP::init();
    sdt.init();
}

enum RSDP {}

impl RSDP {
    pub fn init() -> SDT {
        const RSDP_RAW_LEN: usize = 20;
        const XSDP_RAW_LEN: usize = 36; // Length of an RSDP with an XSDT field.

        let acpi_region = Self::find(Self::firmware_region());
        let raw_rsdp = &acpi_region[..RSDP_RAW_LEN];
        let cksum = checksum(raw_rsdp);
        assert_eq!(cksum, 0, "corrupt RSDPv1");
        let acpi_version: ACPIVersion;
        let phys_sdt_addr: u64;
        if Self::is_acpi_v1(raw_rsdp) {
            acpi_version = ACPIVersion::V1;
            phys_sdt_addr = u64::from(arch::read_u32(&raw_rsdp[16..20]));
        } else {
            let raw_rsdp = &acpi_region[..XSDP_RAW_LEN];
            let len = arch::read_u32(&raw_rsdp[20..24]) as usize;
            assert_eq!(len, XSDP_RAW_LEN, "short RSDPv2");
            let cksum = checksum(raw_rsdp);
            assert_eq!(cksum, 0, "corrupt RSDPv2");
            acpi_version = ACPIVersion::V2;
            phys_sdt_addr = arch::read_u64(&raw_rsdp[24..32]);
        };
        unsafe { SDT::new(phys_sdt_addr, acpi_version) }
    }

    fn find(region: &[u8]) -> &[u8] {
        const RSDP_KEY: [u8; 8] = *b"RSD PTR "; // See ACPI v6.2A, sec. 5.2.5.3.
        let offset = region
            .windows(RSDP_KEY.len())
            .position(|win| win == RSDP_KEY)
            .expect("ACPI rquired");
        &region[offset..]
    }

    fn firmware_region() -> &'static [u8] {
        const ACPI_REGION_START: u64 = 0xE_0000;
        const ACPI_REGION_END: u64 = 0x000F_FFFF + 1;
        let region_start = kmem::phys_to_ptr::<u8>(ACPI_REGION_START);
        let region_length = ACPI_REGION_END - ACPI_REGION_START;
        unsafe { slice::from_raw_parts(region_start, region_length as usize) }
    }

    fn is_acpi_v1(raw: &[u8]) -> bool {
        const ACPI_REVISION_BYTE_INDEX: usize = 15;
        raw[ACPI_REVISION_BYTE_INDEX] == 0
    }
}

pub struct SDT {
    _oem_id: [u8; 6],
    _revision: u8,
    tables: *const [u8],
    acpi_version: ACPIVersion,
}

impl SDT {
    pub unsafe fn new(phys_addr: u64, acpi_version: ACPIVersion) -> SDT {
        let table = Table::new(phys_addr);
        SDT {
            _oem_id: table.oem_id,
            _revision: table.revision,
            tables: table.data() as *const _,
            acpi_version,
        }
    }

    fn ptr_size(&self) -> usize {
        match self.acpi_version {
            ACPIVersion::V1 => 4,
            ACPIVersion::V2 => 8,
        }
    }

    fn as_phys_addr(&self, raw: &[u8]) -> u64 {
        match self.acpi_version {
            ACPIVersion::V1 => u64::from(arch::read_u32(raw)),
            ACPIVersion::V2 => arch::read_u64(raw),
        }
    }

    fn table_ptrs(&self) -> &[u8] {
        unsafe { &*self.tables }
    }

    pub unsafe fn init(&self) {
        let phys_ptrs = self.table_ptrs();
        for raw in phys_ptrs.chunks(self.ptr_size()) {
            let table = kmem::phys_to_ref::<Table>(self.as_phys_addr(raw));
            #[allow(clippy::single_match)]
            match table.signature() {
                b"APIC" => madt::init(table.data()),
                b"MCFG" => mcfg::init(table.data()),
                _ => {}
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct IOAPICT {
    id: u32,
    pub global_intr_base: u32,
    phys_addr: u64,
}

impl IOAPICT {
    pub const fn new(id: u32, global_intr_base: u32, phys_addr: u64) -> IOAPICT {
        IOAPICT {
            id,
            global_intr_base,
            phys_addr,
        }
    }

    pub const fn empty() -> IOAPICT {
        IOAPICT::new(0, 0, 0)
    }

    pub fn phys_addr(&self) -> u64 {
        self.phys_addr
    }
}

mod madt {
    use super::IOAPICT;
    use crate::arch;
    use crate::param;
    use bitflags::bitflags;

    static mut CPUS: [u32; param::NCPUMAX] = [0; param::NCPUMAX];
    static mut NCPUS: usize = 0;

    static mut IOAPICS: [IOAPICT; param::NCPUMAX] = [IOAPICT::empty(); param::NCPUMAX];
    static mut NIOAPICS: usize = 0;

    pub unsafe fn cpus<'a>() -> &'a [u32] {
        &CPUS[..NCPUS]
    }

    pub unsafe fn ioapics<'a>() -> &'a [IOAPICT] {
        &IOAPICS[..NIOAPICS]
    }

    bitflags! {
        pub struct APICFlags: u32 {
            const ENABLED = 1;
        }
    }

    pub fn init(raw_tbl: &[u8]) {
        let mut start = 8;
        while start < raw_tbl.len() {
            assert!(start + 1 < raw_tbl.len());
            let typ = raw_tbl[start];
            let len = raw_tbl[start + 1] as usize;
            assert!(start + len <= raw_tbl.len());
            let data = &raw_tbl[start..start + len];
            start += len;
            match typ {
                0x0 => init_lapic(data),
                0x1 => init_ioapic(data),
                0x7 => init_lsapic(data),
                _ => {}
            }
        }
    }

    fn init_lapic(data: &[u8]) {
        assert_eq!(data[0], 0);
        assert_eq!(data[1] as usize, data.len());
        let apic_id = u32::from(data[3]);
        let flags = APICFlags::from_bits_truncate(arch::read_u32(&data[4..8]));
        if flags.contains(APICFlags::ENABLED) {
            unsafe {
                if !cpus().iter().any(|id| apic_id == *id) {
                    CPUS[NCPUS] = apic_id;
                    NCPUS += 1;
                }
            }
        }
    }

    fn init_lsapic(data: &[u8]) {
        assert_eq!(data[0], 7);
        assert_eq!(data[1] as usize, data.len());
        let apic_id = u32::from(data[3]);
        let flags = APICFlags::from_bits_truncate(arch::read_u32(&data[4..8]));
        if flags.contains(APICFlags::ENABLED) {
            unsafe {
                if !cpus().iter().any(|id| apic_id == *id) {
                    CPUS[NCPUS] = apic_id;
                    NCPUS += 1;
                }
            }
        }
    }

    fn init_ioapic(data: &[u8]) {
        assert_eq!(data[0], 1);
        assert_eq!(data[1] as usize, data.len());
        let id = u32::from(data[2]);
        let phys_addr = u64::from(arch::read_u32(&data[4..8]));
        let global_intr_base = arch::read_u32(&data[8..12]);
        unsafe {
            IOAPICS[NIOAPICS] = IOAPICT::new(id, global_intr_base, phys_addr);
            NIOAPICS += 1;
        }
    }
}

mod mcfg {
    use crate::arch;
    use crate::param;
    use crate::pci;

    static mut CONFIGS: [pci::Config; param::NPCICFGMAX] =
        [pci::Config::empty(); param::NPCICFGMAX];
    static mut NCONFIGS: usize = 0;

    pub fn init(raw_tbl: &[u8]) {
        let mut start = 8;
        while start < raw_tbl.len() {
            assert!(start + 16 <= raw_tbl.len());
            let data = &raw_tbl[start..start + 16];
            start += 16;
            let base_addr = arch::read_u64(&data[0..8]);
            let segment_group = arch::read_u16(&data[8..10]);
            let start_bus = data[10];
            let end_bus = data[11];
            unsafe {
                CONFIGS[NCONFIGS] = pci::Config::new(base_addr, segment_group, start_bus, end_bus);
                NCONFIGS += 1;
            }
        }
    }

    pub fn configs() -> &'static [pci::Config] {
        unsafe { &CONFIGS[..NCONFIGS] }
    }
}

pub use madt::{cpus, ioapics};
pub use mcfg::configs as pci_configs;
