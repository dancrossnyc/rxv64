use crate::acpi;
use crate::kmem;

pub type Bus = u8;

#[derive(Clone, Copy, Debug)]
pub enum Device {
    D0 = 0,
    D1 = 1,
    D2 = 2,
    D3 = 3,
    D4 = 4,
    D5 = 5,
    D6 = 6,
    D7 = 7,
    D8 = 8,
    D9 = 9,
    D10 = 10,
    D11 = 11,
    D12 = 12,
    D13 = 13,
    D14 = 14,
    D15 = 15,
    D16 = 16,
    D17 = 17,
    D18 = 18,
    D19 = 19,
    D20 = 20,
    D21 = 21,
    D22 = 22,
    D23 = 23,
    D24 = 24,
    D25 = 25,
    D26 = 26,
    D27 = 27,
    D28 = 28,
    D29 = 29,
    D30 = 30,
    D31 = 31,
}

static DEVICES: [Device; 32] = [
    Device::D0,
    Device::D1,
    Device::D2,
    Device::D3,
    Device::D4,
    Device::D5,
    Device::D6,
    Device::D7,
    Device::D8,
    Device::D9,
    Device::D10,
    Device::D11,
    Device::D12,
    Device::D13,
    Device::D14,
    Device::D15,
    Device::D16,
    Device::D17,
    Device::D18,
    Device::D19,
    Device::D20,
    Device::D21,
    Device::D22,
    Device::D23,
    Device::D24,
    Device::D25,
    Device::D26,
    Device::D27,
    Device::D28,
    Device::D29,
    Device::D30,
    Device::D31,
];

static FUNCTIONS: [Function; 8] = [
    Function::F0,
    Function::F1,
    Function::F2,
    Function::F3,
    Function::F4,
    Function::F5,
    Function::F6,
    Function::F7,
];

#[derive(Clone, Copy, Debug)]
pub enum Function {
    F0 = 0,
    F1 = 1,
    F2 = 2,
    F3 = 3,
    F4 = 4,
    F5 = 5,
    F6 = 6,
    F7 = 7,
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    phys_addr: u64,
    segment_group: u16,
    start_bus: Bus,
    end_bus: Bus,
}

impl Config {
    pub const fn empty() -> Config {
        Config {
            phys_addr: 0,
            segment_group: 0,
            start_bus: 0,
            end_bus: 0,
        }
    }

    pub fn new(phys_addr: u64, segment_group: u16, start_bus: u8, end_bus: u8) -> Config {
        Config {
            phys_addr,
            segment_group,
            start_bus,
            end_bus,
        }
    }

    pub fn func_addr(&self, bus: Bus, device: Device, function: Function) -> u64 {
        (self.phys_addr + (u64::from(bus - self.start_bus) << 20))
            | (device as u64) << 15
            | (function as u64) << 12
    }
}

pub unsafe fn init() {
    let configs = acpi::pci_configs();
    for config in configs.iter() {
        crate::println!("searching config {:x?}", config);
        for bus in config.start_bus..config.end_bus {
            for dev in DEVICES.iter() {
                for func in FUNCTIONS.iter() {
                    let phys_addr = config.func_addr(bus, *dev, *func);
                    let addr = kmem::phys_to_ref::<u16>(phys_addr);
                    let vendor_id = *addr;
                    if vendor_id == 0xFFFF {
                        continue;
                    }
                    let addr = kmem::phys_to_ref::<u16>(phys_addr + 2);
                    let device_id = *addr;
                    let addr = kmem::phys_to_ref::<u32>(phys_addr + 8);
                    let stuff = *addr;
                    crate::println!(
                        "phys_addr for bus {}, {:?}, {:?} is {:x} ({:x}/{:x} - {:x})",
                        bus,
                        dev,
                        func,
                        phys_addr,
                        vendor_id,
                        device_id,
                        stuff,
                    );
                }
            }
        }
    }
}
