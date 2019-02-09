use bitflags::bitflags;

pub const INTR: u32 = 1;

pub unsafe fn init() {
    use crate::ioapic;
    ioapic::enable(INTR, 0);
}

const STATUS_PORT: u16 = 0x64;
const DATA_PORT: u16 = 0x60;

bitflags! {
    pub struct Status: u8 {
        const DATA_AVAIL = 1;
    }
}

bitflags! {
    pub struct Modifiers: usize {
        const NORMAL = 0;
        const SHIFT = 1;
        const CTL = 1 << 1;
        const ALT = 1 << 2;
        const CAPSLOCK = 1 << 3;
        const NUMLOCK = 1 << 4;
        const SCROLLLOCK = 1 << 5;
        const E0ESC = 1 << 6;
    }
}

pub const NO: u8 = 0u8;
pub const HOME: u8 = 0xE0;
pub const END: u8 = 0xE1;
pub const UP: u8 = 0xE2;
pub const DOWN: u8 = 0xE3;
pub const LEFT: u8 = 0xE4;
pub const RIGHT: u8 = 0xE5;
pub const PGUP: u8 = 0xE6;
pub const PGDN: u8 = 0xE7;
pub const INS: u8 = 0xE8;
pub const DEL: u8 = 0xE9;

pub const fn c(b: char) -> u8 {
    b as u8 - b'@'
}

fn shift_code(b: u8) -> Modifiers {
    match b {
        0x1D | 0x9D => Modifiers::CTL,
        0x2A | 0x36 => Modifiers::SHIFT,
        0x38 | 0xB8 => Modifiers::ALT,
        _ => Modifiers::empty(),
    }
}

fn toggle_code(b: u8) -> Modifiers {
    match b {
        0x3A => Modifiers::CAPSLOCK,
        0x45 => Modifiers::NUMLOCK,
        0x46 => Modifiers::SCROLLLOCK,
        _ => Modifiers::empty(),
    }
}

#[rustfmt::skip]
const NORMAL_MAP: [u8; 256] = [
    NO,      0x1B,    b'1',    b'2',    b'3',    b'4',    b'5',    b'6', // 0x00
    b'7',    b'8',    b'9',    b'0',    b'-',    b'=',    b'\x07', b'\t',
    b'q',    b'w',    b'e',    b'r',    b't',    b'y',    b'u',    b'i', // 0x10
    b'o',    b'p',    b'[',    b']',    b'\n',   NO,      b'a',    b's',
    b'd',    b'f',    b'g',    b'h',    b'j',    b'k',    b'l',    b';', // 0x20
    b'\'',   b'`',    NO,      b'\\',   b'z',    b'x',    b'c',    b'v',
    b'b',    b'n',    b'm',    b',',    b'.',    b'/',    NO,      b'*', // 0x30
    NO,      b' ',    NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      b'7', // 0x40
    b'8',    b'9',    b'-',    b'4',    b'5',    b'6',    b'+',    b'1',
    b'2',    b'3',    b'0',    b'.',    NO,      NO,      NO,      NO,   // 0x50
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0x60
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0x70
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      b'/',    NO,      NO,   // 0x80
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0x90
    NO,      NO,      NO,      NO,      b'\n',   NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xa0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xb0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      HOME, // 0xc0
    UP,      PGUP,    NO,      LEFT,    NO,      RIGHT,   NO,      END,
    DOWN,    PGDN,    INS,     DEL,     NO,      NO,      NO,      NO,   // 0xd0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xe0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xf0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
];

#[rustfmt::skip]
const SHIFT_MAP: [u8; 256] = [
    NO,      0o33,    b'!',    b'@',    b'#',    b'$',    b'%',    b'^',  // 0x00
    b'&',    b'*',    b'(',    b')',    b'_',    b'+',    b'\x07', b'\t',
    b'Q',    b'W',    b'E',    b'R',    b'T',    b'Y',    b'U',    b'I',  // 0x10
    b'O',    b'P',    b'{',    b'}',    b'\n',    NO,     b'A',    b'S',
    b'D',    b'F',    b'G',    b'H',    b'J',    b'K',    b'L',    b':',  // 0x20
    b'"',    b'~',     NO,     b'|',    b'Z',    b'X',    b'C',    b'V',
    b'B',    b'N',    b'M',    b'<',    b'>',    b'?',    NO,      b'*',  // 0x30
    NO,      b' ',    NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      b'7',  // 0x40
    b'8',    b'9',    b'-',    b'4',    b'5',    b'6',    b'+',    b'1',
    b'2',    b'3',    b'0',    b'.',    NO,      NO,      NO,      NO,   // 0x50
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0x60
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0x70
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      b'/',    NO,      NO,   // 0x80
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO, // 0x90
    NO,      NO,      NO,      NO,      b'\n',   NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xa0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xb0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      HOME, // 0xc0
    UP,      PGUP,    NO,      LEFT,    NO,      RIGHT,   NO,      END,
    DOWN,    PGDN,    INS,     DEL,     NO,      NO,      NO,      NO,   // 0xd0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xe0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xf0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
];

#[rustfmt::skip]
const CTL_MAP: [u8; 256] = [

    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO, // 0x00
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    c('Q'),  c('W'),  c('E'),  c('R'),  c('T'),  c('Y'),  c('U'),  c('I'), // 0x10
    c('O'),  c('P'),  NO,      NO,      b'\r',   NO,      c('A'),  c('S'),
    c('D'),  c('F'),  c('G'),  c('H'),  c('J'),  c('K'),  c('L'),  NO, // 0x20
    NO,      NO,      NO,      c('\\'), c('Z'),  c('X'),  c('C'),  c('V'),
    c('B'),  c('N'),  c('M'),  NO,      NO,      b'/',    NO,      NO, // 0x30
    NO,      b' ',    NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      b'7', // 0x40
    b'8',    b'9',    b'-',    b'4',    b'5',    b'6',    b'+',    b'1',
    b'2',    b'3',    b'0',    b'.',    NO,      NO,      NO,      NO,   // 0x50
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0x60
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0x70
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      b'/',    NO,      NO,   // 0x80
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0x90
    NO,      NO,      NO,      NO,      b'\r',   NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xa0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xb0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      HOME, // 0xc0
    UP,      PGUP,    NO,      LEFT,    NO,      RIGHT,   NO,      END,
    DOWN,    PGDN,    INS,     DEL,     NO,      NO,      NO,      NO,   // 0xd0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xe0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,   // 0xf0
    NO,      NO,      NO,      NO,      NO,      NO,      NO,      NO,
];

pub unsafe fn getb() -> Option<u8> {
    static mut MODKEYS: Modifiers = Modifiers::NORMAL;
    use crate::x86_64::inb;
    let status = Status::from_bits_truncate(inb(STATUS_PORT));
    if !status.contains(Status::DATA_AVAIL) {
        return None;
    }
    let mut data = inb(DATA_PORT);
    if data == 0xE0 {
        // ESC key
        MODKEYS.insert(Modifiers::E0ESC);
        return None;
    } else if (data & 0b1000_0000) != 0 {
        // Key up event
        data = if MODKEYS.contains(Modifiers::E0ESC) {
            data
        } else {
            data & 0b0111_1111
        };
        MODKEYS.remove(Modifiers::E0ESC | shift_code(data));
        return None;
    } else if MODKEYS.contains(Modifiers::E0ESC) {
        data |= 0b1000_0000;
        MODKEYS.remove(Modifiers::E0ESC);
    }
    MODKEYS.insert(shift_code(data));
    MODKEYS.toggle(toggle_code(data));
    let map = if MODKEYS.contains(Modifiers::CTL) {
        &CTL_MAP
    } else if MODKEYS.contains(Modifiers::SHIFT) {
        &SHIFT_MAP
    } else {
        &NORMAL_MAP
    };
    let mut b = map[data as usize];
    if MODKEYS.contains(Modifiers::CAPSLOCK) {
        if b.is_ascii_lowercase() {
            b.make_ascii_uppercase();
        } else if b.is_ascii_uppercase() {
            b.make_ascii_lowercase();
        }
    }
    if b == 0 {
        return None;
    }
    Some(b)
}
