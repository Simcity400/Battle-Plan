
use super::Failure;

pub(super) enum Stop {
    Fatal(String),
    Undefined(String),
    Device(String),
    OverBudget(String),
}

fn fatal<T>(why: impl Into<String>) -> Result<T, Stop> {
    Err(Stop::Fatal(why.into()))
}

const NATURAL: [usize; 80] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
    63, 63, 63, 63, 63, 63,
];
const NATURAL2: [usize; 20] = [
    0, 1, 8, 9, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
];
const NATURAL3: [usize; 25] = [
    0, 1, 8, 16, 9, 2, 10, 17, 18, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
];
const NATURAL4: [usize; 32] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 25, 18, 11, 19, 26, 27, 63, 63, 63, 63, 63, 63, 63, 63, 63,
    63, 63, 63, 63, 63, 63, 63,
];
const NATURAL5: [usize; 41] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 12, 19, 26, 33, 34, 27, 20, 28, 35, 36,
    63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
];
const NATURAL6: [usize; 52] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 41, 34, 27, 20, 13,
    21, 28, 35, 42, 43, 36, 29, 37, 44, 45, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
    63, 63,
];
const NATURAL7: [usize; 65] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 14, 21, 28, 35, 42, 49, 50, 43, 36, 29, 22, 30, 37, 44, 51, 52, 45, 38, 46, 53, 54, 63,
    63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
];

const ARITAB: [u32; 114] = [
    0x5A1D0181, 0x2586020E, 0x11140310, 0x080B0412, 0x03D80514, 0x01DA0617, 0x00E50719, 0x006F081C,
    0x0036091E, 0x001A0A21, 0x000D0B23, 0x00060C09, 0x00030D0A, 0x00010D0C, 0x5A7F0F8F, 0x3F251024,
    0x2CF21126, 0x207C1227, 0x17B91328, 0x1182142A, 0x0CEF152B, 0x09A1162D, 0x072F172E, 0x055C1830,
    0x04061931, 0x03031A33, 0x02401B34, 0x01B11C36, 0x01441D38, 0x00F51E39, 0x00B71F3B, 0x008A203C,
    0x0068213E, 0x004E223F, 0x003B2320, 0x002C0921, 0x5AE125A5, 0x484C2640, 0x3A0D2741, 0x2EF12843,
    0x261F2944, 0x1F332A45, 0x19A82B46, 0x15182C48, 0x11772D49, 0x0E742E4A, 0x0BFB2F4B, 0x09F8304D,
    0x0861314E, 0x0706324F, 0x05CD3330, 0x04DE3432, 0x040F3532, 0x03633633, 0x02D43734, 0x025C3835,
    0x01F83936, 0x01A43A37, 0x01603B38, 0x01253C39, 0x00F63D3A, 0x00CB3E3B, 0x00AB3F3D, 0x008F203D,
    0x5B1241C1, 0x4D044250, 0x412C4351, 0x37D84452, 0x2FE84553, 0x293C4654, 0x23794756, 0x1EDF4857,
    0x1AA94957, 0x174E4A48, 0x14244B48, 0x119C4C4A, 0x0F6B4D4A, 0x0D514E4B, 0x0BB64F4D, 0x0A40304D,
    0x583251D0, 0x4D1C5258, 0x438E5359, 0x3BDD545A, 0x34EE555B, 0x2EAE565C, 0x299A575D, 0x25164756,
    0x557059D8, 0x4CA95A5F, 0x44D95B60, 0x3E225C61, 0x38245D63, 0x32B45E63, 0x2E17565D, 0x56A860DF,
    0x4F466165, 0x47E56266, 0x41CF6367, 0x3C3D6468, 0x375E5D63, 0x52316669, 0x4C0F676A, 0x4639686B,
    0x415E6367, 0x56276AE9, 0x50E76B6C, 0x4B85676D, 0x55976D6E, 0x504F6B6F, 0x5A106FEE, 0x55226D70,
    0x59EB6FF0, 0x5A1D7171,
];

const AANSCALES: [i32; 64] = [
    16384, 22725, 21407, 19266, 16384, 12873, 8867, 4520, 22725, 31521, 29692, 26722, 22725, 17855,
    12299, 6270, 21407, 29692, 27969, 25172, 21407, 16819, 11585, 5906, 19266, 26722, 25172, 22654,
    19266, 15137, 10426, 5315, 16384, 22725, 21407, 19266, 16384, 12873, 8867, 4520, 12873, 17855,
    16819, 15137, 12873, 10114, 6967, 3552, 8867, 12299, 11585, 10426, 8867, 6967, 4799, 2446,
    4520, 6270, 5906, 5315, 4520, 3552, 2446, 1247,
];

const BMASK: [i32; 16] = [
    0, 0x1, 0x3, 0x7, 0xF, 0x1F, 0x3F, 0x7F, 0xFF, 0x1FF, 0x3FF, 0x7FF, 0xFFF, 0x1FFF, 0x3FFF,
    0x7FFF,
];

fn extend(x: i32, s: i32) -> i32 {
    if x <= BMASK[(s - 1) as usize] {
        x - BMASK[s as usize]
    } else {
        x
    }
}

#[derive(Clone)]
struct HuffTable {
    bits: [u8; 17],
    values: [u8; 256],
}

type Tables = Vec<Option<Derived>>;

struct Derived {
    maxcode: [i32; 18],
    valoffset: [i32; 17],
    look_nbits: [u8; 256],
    look_sym: [u8; 256],
    values: [u8; 256],
}

fn derive(table: Option<&HuffTable>, is_dc: bool, number: usize) -> Result<Derived, Stop> {
    let Some(table) = table else {
        return fatal(format!("Huffman table 0x{number:02x} was not defined"));
    };
    let mut size = [0_u8; 257];
    let mut p = 0;
    for l in 1..=16 {
        let count = usize::from(table.bits[l]);
        if p + count > 256 {
            return fatal("Bogus Huffman table definition");
        }
        for _ in 0..count {
            size[p] = l as u8;
            p += 1;
        }
    }
    size[p] = 0;
    let symbols = p;
    let mut code = [0_u32; 257];
    let mut next = 0_u32;
    let mut si = u32::from(size[0]);
    p = 0;
    while size[p] != 0 {
        while u32::from(size[p]) == si {
            code[p] = next;
            p += 1;
            next += 1;
        }
        if i64::from(next) >= 1_i64 << si {
            return fatal("Bogus Huffman table definition");
        }
        next <<= 1;
        si += 1;
    }
    let mut derived = Derived {
        maxcode: [0; 18],
        valoffset: [0; 17],
        look_nbits: [0; 256],
        look_sym: [0; 256],
        values: table.values,
    };
    p = 0;
    for l in 1..=16 {
        if table.bits[l] != 0 {
            derived.valoffset[l] = p as i32 - code[p] as i32;
            p += usize::from(table.bits[l]);
            derived.maxcode[l] = code[p - 1] as i32;
        } else {
            derived.maxcode[l] = -1;
        }
    }
    derived.maxcode[17] = 0xFFFFF;
    p = 0;
    for l in 1..=8 {
        for _ in 0..table.bits[l] {
            let first = (code[p] << (8 - l)) as usize;
            for look in first..first + (1 << (8 - l)) {
                derived.look_nbits[look] = l as u8;
                derived.look_sym[look] = table.values[p];
            }
            p += 1;
        }
    }
    if is_dc && table.values[..symbols].iter().any(|symbol| *symbol > 15) {
        return fatal("Bogus Huffman table definition");
    }
    Ok(derived)
}

#[derive(Clone, Default)]
struct Component {
    id: i32,
    h: usize,
    v: usize,
    quant_number: usize,
    dc_table: usize,
    ac_table: usize,
    width_in_blocks: usize,
    height_in_blocks: usize,
    stride: usize,
    quant: Option<[u16; 64]>,
    coefficients: Vec<[i16; 64]>,
    mcu_width: usize,
    mcu_height: usize,
}

#[derive(Clone, Copy)]
enum Bin {
    Dc(usize, usize),
    Ac(usize, usize),
    Fixed,
}

struct Jpeg<'a> {
    data: &'a [u8],
    position: usize,
    unread_marker: u8,
    saw_soi: bool,
    saw_sof: bool,
    restart_interval: u32,
    arith_dc_l: [u8; 16],
    arith_dc_u: [u8; 16],
    arith_ac_k: [u8; 16],
    saw_jfif: bool,
    saw_adobe: bool,
    adobe_transform: u8,
    color_transform: bool,
    is_baseline: bool,
    progressive: bool,
    arith: bool,
    precision: u8,
    height: usize,
    width: usize,
    components: Vec<Component>,
    quant_tables: [Option<[u16; 64]>; 4],
    dc_tables: [Option<HuffTable>; 4],
    ac_tables: [Option<HuffTable>; 4],
    scan: Vec<usize>,
    ss: i32,
    se: i32,
    ah: i32,
    al: i32,
    max_h: usize,
    max_v: usize,
    lim_se: i32,
    block_size: usize,
    natural: &'static [usize],
    coef_bits: Vec<[i32; 64]>,
    get_buffer: u32,
    bits_left: i32,
    insufficient: bool,
    eobrun: u32,
    last_dc: [i32; 4],
    restarts_to_go: u32,
    next_restart: u8,
    c: i32,
    a: i32,
    ct: i32,
    dc_context: [usize; 4],
    dc_stats: [[u8; 64]; 16],
    ac_stats: [[u8; 256]; 16],
    fixed_bin: [u8; 4],
}

impl<'a> Jpeg<'a> {
    fn byte(&mut self) -> u8 {
        let at = self.position;
        self.position += 1;
        match self.data.get(at) {
            Some(byte) => *byte,
            None if (at - self.data.len()).is_multiple_of(2) => 0xFF,
            None => 0xD9,
        }
    }

    fn word(&mut self) -> i32 {
        let high = i32::from(self.byte());
        (high << 8) + i32::from(self.byte())
    }

    fn skip(&mut self, count: i32) {
        if count > 0 {
            self.position += count as usize;
        }
    }


    fn next_marker(&mut self) {
        loop {
            let mut c = self.byte();
            while c != 0xFF {
                c = self.byte();
            }
            loop {
                c = self.byte();
                if c != 0xFF {
                    break;
                }
            }
            if c != 0 {
                self.unread_marker = c;
                return;
            }
        }
    }

    fn get_sof(&mut self, baseline: bool, progressive: bool, arith: bool) -> Result<(), Stop> {
        self.is_baseline = baseline;
        self.progressive = progressive;
        self.arith = arith;
        let length = self.word();
        self.precision = self.byte();
        self.height = self.word() as usize;
        self.width = self.word() as usize;
        let count = usize::from(self.byte());
        if self.saw_sof {
            return fatal("Invalid JPEG file structure: two SOF markers");
        }
        if self.height == 0 || self.width == 0 || count == 0 {
            return fatal("Empty JPEG image (DNL not supported)");
        }
        if length - 8 != count as i32 * 3 {
            return fatal("Bogus marker length");
        }
        self.components = vec![Component::default(); count];
        for index in 0..count {
            let mut id = i32::from(self.byte());
            if self.components[..index]
                .iter()
                .any(|component| component.id == id)
            {
                id = self.components[0].id;
                for component in &self.components[1..index] {
                    id = id.max(component.id);
                }
                id += 1;
            }
            let sampling = self.byte();
            let quant = self.byte();
            let component = &mut self.components[index];
            component.id = id;
            component.h = usize::from(sampling >> 4);
            component.v = usize::from(sampling & 15);
            component.quant_number = usize::from(quant);
        }
        self.saw_sof = true;
        Ok(())
    }

    fn get_sos(&mut self) -> Result<(), Stop> {
        if !self.saw_sof {
            return fatal("Invalid JPEG file structure: SOS before SOF");
        }
        let length = self.word();
        let count = i32::from(self.byte());
        if length != count * 2 + 6 || count > 4 || (count == 0 && !self.progressive) {
            return fatal("Bogus marker length");
        }
        self.scan.clear();
        for i in 0..count as usize {
            let mut id = i32::from(self.byte());
            if self.scan[..i].iter().any(|&c| self.components[c].id == id) {
                id = self.components[self.scan[0]].id;
                for &c in &self.scan[1..i] {
                    id = id.max(self.components[c].id);
                }
                id += 1;
            }
            let Some(index) = self
                .components
                .iter()
                .position(|component| component.id == id)
            else {
                return fatal(format!("Invalid component ID {id} in SOS"));
            };
            self.scan.push(index);
            let tables = self.byte();
            self.components[index].dc_table = usize::from(tables >> 4);
            self.components[index].ac_table = usize::from(tables & 15);
        }
        self.ss = i32::from(self.byte());
        self.se = i32::from(self.byte());
        let approximation = self.byte();
        self.ah = i32::from(approximation >> 4);
        self.al = i32::from(approximation & 15);
        self.next_restart = 0;
        Ok(())
    }

    fn get_dht(&mut self) -> Result<(), Stop> {
        let mut length = self.word() - 2;
        while length > 16 {
            let index = self.byte();
            let mut bits = [0_u8; 17];
            let mut count = 0_i32;
            for bit in &mut bits[1..] {
                *bit = self.byte();
                count += i32::from(*bit);
            }
            length -= 17;
            if count > 256 || count > length {
                return fatal("Bogus Huffman table definition");
            }
            let mut values = [0_u8; 256];
            for value in &mut values[..count as usize] {
                *value = self.byte();
            }
            length -= count;
            let (slot, ac) = if index & 0x10 != 0 {
                (index - 0x10, true)
            } else {
                (index, false)
            };
            if slot >= 4 {
                return fatal(format!("Bogus DHT index {slot}"));
            }
            let table = Some(HuffTable { bits, values });
            if ac {
                self.ac_tables[usize::from(slot)] = table;
            } else {
                self.dc_tables[usize::from(slot)] = table;
            }
        }
        if length != 0 {
            return fatal("Bogus marker length");
        }
        Ok(())
    }

    fn get_dqt(&mut self) -> Result<(), Stop> {
        let mut length = self.word() - 2;
        while length > 0 {
            length -= 1;
            let byte = self.byte();
            let (precision, n) = (byte >> 4, usize::from(byte & 15));
            if n >= 4 {
                return fatal(format!("Bogus DQT index {n}"));
            }
            let table = self.quant_tables[n].get_or_insert([0; 64]);
            let count = if precision != 0 {
                if length < 128 {
                    *table = [1; 64];
                    length >> 1
                } else {
                    64
                }
            } else if length < 64 {
                *table = [1; 64];
                length
            } else {
                64
            };
            let order: &[usize] = match count {
                4 => &NATURAL2,
                9 => &NATURAL3,
                16 => &NATURAL4,
                25 => &NATURAL5,
                36 => &NATURAL6,
                49 => &NATURAL7,
                _ => &NATURAL,
            };
            for &position in &order[..count.max(0) as usize] {
                let value = if precision != 0 {
                    self.word() as u16
                } else {
                    u16::from(self.byte())
                };
                self.quant_tables[n].as_mut().unwrap()[position] = value;
            }
            length -= count;
            if precision != 0 {
                length -= count;
            }
        }
        if length != 0 {
            return fatal("Bogus marker length");
        }
        Ok(())
    }

    fn get_dac(&mut self) -> Result<(), Stop> {
        let mut length = self.word() - 2;
        while length > 0 {
            let index = usize::from(self.byte());
            let value = self.byte();
            length -= 2;
            if index >= 32 {
                return fatal(format!("Bogus DAC index {index}"));
            }
            if index >= 16 {
                self.arith_ac_k[index - 16] = value;
            } else {
                self.arith_dc_l[index] = value & 15;
                self.arith_dc_u[index] = value >> 4;
                if self.arith_dc_l[index] > self.arith_dc_u[index] {
                    return fatal(format!("Bogus DAC value 0x{value:x}"));
                }
            }
        }
        if length != 0 {
            return fatal("Bogus marker length");
        }
        Ok(())
    }

    fn get_lse(&mut self) -> Result<(), Stop> {
        if !self.saw_sof {
            return fatal("Invalid JPEG file structure: LSE before SOF");
        }
        let bad = || fatal("Unsupported color conversion request");
        if self.components.len() < 3 {
            return bad();
        }
        if self.word() != 24 {
            return fatal("Bogus marker length");
        }
        let _ = self.byte();
        let checks: [(bool, i32); 13] = [
            (true, 255),
            (false, 3),
            (false, self.components[1].id),
            (false, self.components[0].id),
            (false, self.components[2].id),
            (false, 0x80),
            (true, 0),
            (true, 0),
            (false, 0),
            (true, 1),
            (true, 0),
            (false, 0),
            (true, 1),
        ];
        for (two, expected) in checks {
            let value = if two {
                self.word()
            } else {
                i32::from(self.byte())
            };
            if value != expected {
                return bad();
            }
        }
        if self.word() != 0 {
            return bad();
        }
        self.color_transform = true;
        Ok(())
    }

    fn get_app(&mut self, marker: u8) {
        let length = self.word() - 2;
        let data: Vec<u8> = (0..length.max(0)).map(|_| self.byte()).collect();
        if marker == 0xE0 && data.len() >= 14 && data[..5] == *b"JFIF\0" {
            self.saw_jfif = true;
        } else if marker == 0xEE && data.len() >= 12 && data[..5] == *b"Adobe" {
            self.saw_adobe = true;
            self.adobe_transform = data[11];
        }
    }

    fn skip_variable(&mut self) {
        let length = self.word() - 2;
        self.skip(length);
    }

    fn read_markers(&mut self) -> Result<bool, Stop> {
        loop {
            if self.unread_marker == 0 {
                if !self.saw_soi {
                    let (c, c2) = (self.byte(), self.byte());
                    if c != 0xFF || c2 != 0xD8 {
                        return fatal("Not a JPEG file");
                    }
                    self.unread_marker = c2;
                } else {
                    self.next_marker();
                }
            }
            match self.unread_marker {
                0xD8 => {
                    if self.saw_soi {
                        return fatal("Invalid JPEG file structure: two SOI markers");
                    }
                    self.arith_dc_l = [0; 16];
                    self.arith_dc_u = [1; 16];
                    self.arith_ac_k = [5; 16];
                    self.restart_interval = 0;
                    self.color_transform = false;
                    self.saw_jfif = false;
                    self.saw_adobe = false;
                    self.adobe_transform = 0;
                    self.saw_soi = true;
                }
                0xC0 => self.get_sof(true, false, false)?,
                0xC1 => self.get_sof(false, false, false)?,
                0xC2 => self.get_sof(false, true, false)?,
                0xC9 => self.get_sof(false, false, true)?,
                0xCA => self.get_sof(false, true, true)?,
                marker @ (0xC3 | 0xC5 | 0xC6 | 0xC7 | 0xC8 | 0xCB | 0xCD | 0xCE | 0xCF) => {
                    return fatal(format!("Unsupported JPEG process: SOF type 0x{marker:02x}"));
                }
                0xDA => {
                    self.get_sos()?;
                    self.unread_marker = 0;
                    return Ok(true);
                }
                0xD9 => {
                    self.unread_marker = 0;
                    return Ok(false);
                }
                0xCC => self.get_dac()?,
                0xC4 => self.get_dht()?,
                0xDB => self.get_dqt()?,
                0xDD => {
                    if self.word() != 4 {
                        return fatal("Bogus marker length");
                    }
                    self.restart_interval = self.word() as u32;
                }
                0xF8 => self.get_lse()?,
                marker @ (0xE0..=0xEF | 0xFE) => self.get_app(marker),
                0xD0..=0xD7 | 0x01 => {}
                0xDC => self.skip_variable(),
                _ => {}
            }
            self.unread_marker = 0;
        }
    }

    fn read_restart_marker(&mut self) {
        if self.unread_marker == 0 {
            self.next_marker();
        }
        let desired = self.next_restart;
        if self.unread_marker == 0xD0 + desired {
            self.unread_marker = 0;
        } else {
            loop {
                let marker = self.unread_marker;
                let action = if marker < 0xC0 {
                    2
                } else if !(0xD0..=0xD7).contains(&marker)
                    || marker == 0xD0 + ((desired + 1) & 7)
                    || marker == 0xD0 + ((desired + 2) & 7)
                {
                    3
                } else if marker == 0xD0 + (desired.wrapping_sub(1) & 7)
                    || marker == 0xD0 + (desired.wrapping_sub(2) & 7)
                {
                    2
                } else {
                    1
                };
                match action {
                    1 => {
                        self.unread_marker = 0;
                        break;
                    }
                    2 => self.next_marker(),
                    _ => break,
                }
            }
        }
        self.next_restart = (self.next_restart + 1) & 7;
    }


    fn initial_setup(&mut self) -> Result<(), Stop> {
        if self.height > 65500 || self.width > 65500 {
            return fatal("Maximum supported image dimension is 65500 pixels");
        }
        if !(8..=12).contains(&self.precision) {
            return fatal(format!(
                "Unsupported JPEG data precision {}",
                self.precision
            ));
        }
        if self.components.len() > 10 {
            return fatal("Too many color components");
        }
        self.max_h = 1;
        self.max_v = 1;
        for component in &self.components {
            if !(1..=4).contains(&component.h) || !(1..=4).contains(&component.v) {
                return fatal("Bogus sampling factors");
            }
            self.max_h = self.max_h.max(component.h);
            self.max_v = self.max_v.max(component.v);
        }
        if self.is_baseline || (self.progressive && !self.scan.is_empty()) {
            self.block_size = 8;
            self.natural = &NATURAL;
            self.lim_se = 63;
        } else {
            let Some(size) = (1..=16_usize).find(|size| (size * size - 1) as i32 == self.se) else {
                return fatal(format!(
                    "Invalid progressive parameters Ss={} Se={} Ah={} Al={}",
                    self.ss, self.se, self.ah, self.al
                ));
            };
            self.block_size = size;
            (self.natural, self.lim_se) = match size {
                1 => (&NATURAL[..], 0),
                2 => (&NATURAL2[..], 3),
                3 => (&NATURAL3[..], 8),
                4 => (&NATURAL4[..], 15),
                5 => (&NATURAL5[..], 24),
                6 => (&NATURAL6[..], 35),
                7 => (&NATURAL7[..], 48),
                _ => (&NATURAL[..], 63),
            };
        }
        let (max_h, max_v, block) = (self.max_h, self.max_v, self.block_size);
        let (width, height) = (self.width, self.height);
        for component in &mut self.components {
            component.width_in_blocks = (width * component.h).div_ceil(max_h * block);
            component.height_in_blocks = (height * component.v).div_ceil(max_v * block);
            component.stride = component.width_in_blocks.div_ceil(component.h) * component.h;
            let rows = component.height_in_blocks.div_ceil(component.v) * component.v;
            component.coefficients = vec![[0; 64]; component.stride * rows];
        }
        Ok(())
    }

    fn start_scan(&mut self) -> Result<(), Stop> {
        let blocks: usize = if self.scan.len() == 1 {
            let component = &mut self.components[self.scan[0]];
            component.mcu_width = 1;
            component.mcu_height = 1;
            1
        } else {
            let mut blocks = 0;
            for &index in &self.scan {
                let component = &mut self.components[index];
                component.mcu_width = component.h;
                component.mcu_height = component.v;
                blocks += component.h * component.v;
            }
            if blocks > 10 {
                return fatal("Sampling factors too large for interleaved scan");
            }
            blocks
        };
        let _ = blocks;
        for &index in &self.scan {
            let number = self.components[index].quant_number;
            if self.components[index].quant.is_none() {
                let Some(table) = self.quant_tables.get(number).and_then(|table| *table) else {
                    return fatal(format!("Quantization table 0x{number:02x} was not defined"));
                };
                self.components[index].quant = Some(table);
            }
        }
        if self.progressive {
            let valid = if self.ss == 0 {
                self.se == 0
            } else {
                self.se >= self.ss && self.se <= self.lim_se && self.scan.len() == 1
            } && (self.ah == 0 || self.ah - 1 == self.al)
                && self.al <= 13;
            if !valid {
                return fatal(format!(
                    "Invalid progressive parameters Ss={} Se={} Ah={} Al={}",
                    self.ss, self.se, self.ah, self.al
                ));
            }
            for &index in &self.scan {
                for coefficient in self.ss..=self.se {
                    self.coef_bits[index][coefficient as usize] = self.al;
                }
            }
        }
        if self.arith {
            for &index in &self.scan {
                let component = &self.components[index];
                if !self.progressive || (self.ss == 0 && self.ah == 0) {
                    if component.dc_table >= 16 {
                        return fatal("Arithmetic table 0x{:02x} was not defined");
                    }
                    self.dc_stats[component.dc_table] = [0; 64];
                }
                if (!self.progressive && self.lim_se != 0) || (self.progressive && self.ss != 0) {
                    if component.ac_table >= 16 {
                        return fatal("Arithmetic table was not defined");
                    }
                    self.ac_stats[component.ac_table] = [0; 256];
                }
            }
            self.last_dc = [0; 4];
            self.dc_context = [0; 4];
            self.c = 0;
            self.a = 0;
            self.ct = -16;
        } else {
            self.last_dc = [0; 4];
            self.eobrun = 0;
            self.get_buffer = 0;
            self.bits_left = 0;
            self.insufficient = false;
        }
        self.restarts_to_go = self.restart_interval;
        Ok(())
    }

    fn derive_tables(&self) -> Result<(Tables, Tables), Stop> {
        let mut dc: Vec<Option<Derived>> = (0..4).map(|_| None).collect();
        let mut ac: Vec<Option<Derived>> = (0..4).map(|_| None).collect();
        for &index in &self.scan {
            let component = &self.components[index];
            let wants_dc = !self.progressive || (self.ss == 0 && self.ah == 0);
            let wants_ac = if self.progressive {
                self.ss != 0
            } else {
                self.lim_se != 0
            };
            if wants_dc {
                let number = component.dc_table;
                if number >= 4 {
                    return fatal(format!("Huffman table 0x{number:02x} was not defined"));
                }
                dc[number] = Some(derive(self.dc_tables[number].as_ref(), true, number)?);
            }
            if wants_ac {
                let number = component.ac_table;
                if number >= 4 {
                    return fatal(format!("Huffman table 0x{number:02x} was not defined"));
                }
                ac[number] = Some(derive(self.ac_tables[number].as_ref(), false, number)?);
            }
        }
        Ok((dc, ac))
    }


    fn fill(&mut self, nbits: i32) {
        if self.unread_marker == 0 {
            while self.bits_left < 25 {
                let mut c = self.byte();
                if c == 0xFF {
                    loop {
                        c = self.byte();
                        if c != 0xFF {
                            break;
                        }
                    }
                    if c == 0 {
                        c = 0xFF;
                    } else {
                        self.unread_marker = c;
                        return self.no_more_bytes(nbits);
                    }
                }
                self.get_buffer = (self.get_buffer << 8) | u32::from(c);
                self.bits_left += 8;
            }
        } else {
            self.no_more_bytes(nbits);
        }
    }

    fn no_more_bytes(&mut self, nbits: i32) {
        if nbits > self.bits_left {
            self.insufficient = true;
            self.get_buffer <<= 25 - self.bits_left;
            self.bits_left = 25;
        }
    }

    fn get_bits(&mut self, n: i32) -> i32 {
        if self.bits_left < n {
            self.fill(n);
        }
        self.bits_left -= n;
        ((self.get_buffer >> self.bits_left) as i32) & BMASK[n as usize]
    }

    fn decode(&mut self, table: &Derived) -> i32 {
        let mut l = 9;
        if self.bits_left < 8 {
            self.fill(0);
            if self.bits_left < 8 {
                l = 1;
            }
        }
        if l == 9 {
            let look = ((self.get_buffer >> (self.bits_left - 8)) & 0xFF) as usize;
            let nb = table.look_nbits[look];
            if nb != 0 {
                self.bits_left -= i32::from(nb);
                return i32::from(table.look_sym[look]);
            }
        }
        let mut code = self.get_bits(l);
        while code > table.maxcode[l as usize] {
            code <<= 1;
            code |= self.get_bits(1);
            l += 1;
        }
        if l > 16 {
            return 0;
        }
        i32::from(table.values[((code + table.valoffset[l as usize]) & 0xFF) as usize])
    }

    fn restart(&mut self) {
        if !self.arith {
            self.bits_left = 0;
        }
        self.read_restart_marker();
        if self.arith {
            for &index in &self.scan {
                let component = &self.components[index];
                if !self.progressive || (self.ss == 0 && self.ah == 0) {
                    self.dc_stats[component.dc_table] = [0; 64];
                }
                if (!self.progressive && self.lim_se != 0) || (self.progressive && self.ss != 0) {
                    self.ac_stats[component.ac_table] = [0; 256];
                }
            }
            self.last_dc = [0; 4];
            self.dc_context = [0; 4];
            self.c = 0;
            self.a = 0;
            self.ct = -16;
        } else {
            self.last_dc = [0; 4];
            self.eobrun = 0;
            if self.unread_marker == 0 {
                self.insufficient = false;
            }
        }
        self.restarts_to_go = self.restart_interval;
    }

    fn decode_mcu_huffman(
        &mut self,
        blocks: &[(usize, usize)],
        dc: &[Option<Derived>],
        ac: &[Option<Derived>],
    ) {
        if self.restart_interval != 0 && self.restarts_to_go == 0 {
            self.restart();
        }
        let refine = self.ah != 0;
        if self.progressive && refine && self.ss == 0 {
            let p1 = 1 << self.al;
            for &(component, block) in blocks {
                if self.get_bits(1) != 0 {
                    let index = self.scan[component];
                    self.components[index].coefficients[block][0] |= p1 as i16;
                }
            }
        } else if !self.insufficient {
            if !self.progressive {
                for &(component, block) in blocks {
                    let index = self.scan[component];
                    let (dc_table, ac_table) = (
                        self.components[index].dc_table,
                        self.components[index].ac_table,
                    );
                    let mut s = self.decode(dc[dc_table].as_ref().unwrap());
                    if s != 0 {
                        let r = self.get_bits(s);
                        s = extend(r, s);
                    }
                    self.last_dc[component] = self.last_dc[component].wrapping_add(s);
                    let value = self.last_dc[component] as i16;
                    self.components[index].coefficients[block][0] = value;
                    let limit = self.lim_se as usize + 1;
                    let Some(ac) = ac[ac_table].as_ref().filter(|_| limit > 1) else {
                        continue;
                    };
                    let mut k = 1_usize;
                    while k < limit {
                        let symbol = self.decode(ac);
                        let (r, s) = ((symbol >> 4) as usize, symbol & 15);
                        if s != 0 {
                            k += r;
                            let bits = self.get_bits(s);
                            let position = self.natural[k];
                            self.components[index].coefficients[block][position] =
                                extend(bits, s) as i16;
                        } else {
                            if r != 15 {
                                break;
                            }
                            k += 15;
                        }
                        k += 1;
                    }
                }
            } else if self.ss == 0 {
                for &(component, block) in blocks {
                    let index = self.scan[component];
                    let table = self.components[index].dc_table;
                    let mut s = self.decode(dc[table].as_ref().unwrap());
                    if s != 0 {
                        let r = self.get_bits(s);
                        s = extend(r, s);
                    }
                    self.last_dc[component] = self.last_dc[component].wrapping_add(s);
                    let value = self.last_dc[component].wrapping_shl(self.al as u32) as i16;
                    self.components[index].coefficients[block][0] = value;
                }
            } else if !refine {
                let (component, block) = blocks[0];
                let index = self.scan[component];
                if self.eobrun > 0 {
                    self.eobrun -= 1;
                } else {
                    let table = ac[self.components[index].ac_table].as_ref().unwrap();
                    let mut k = self.ss as usize;
                    while k <= self.se as usize {
                        let symbol = self.decode(table);
                        let (mut r, s) = (symbol >> 4, symbol & 15);
                        if s != 0 {
                            k += r as usize;
                            let bits = self.get_bits(s);
                            let value = extend(bits, s).wrapping_shl(self.al as u32) as i16;
                            self.components[index].coefficients[block][self.natural[k]] = value;
                        } else {
                            if r != 15 {
                                if r != 0 {
                                    let mut eobrun = 1_u32 << r;
                                    r = self.get_bits(r);
                                    eobrun += r as u32;
                                    self.eobrun = eobrun - 1;
                                }
                                break;
                            }
                            k += 15;
                        }
                        k += 1;
                    }
                }
            } else {
                self.decode_ac_refine(blocks[0], ac);
            }
        }
        self.restarts_to_go = self.restarts_to_go.wrapping_sub(1);
    }

    fn decode_ac_refine(&mut self, (component, block): (usize, usize), ac: &[Option<Derived>]) {
        let index = self.scan[component];
        let table = ac[self.components[index].ac_table].as_ref().unwrap();
        let p1 = 1_i32 << self.al;
        let m1 = (-1_i32).wrapping_shl(self.al as u32);
        let se = self.se as usize;
        let mut k = self.ss as usize;
        let mut eobrun = self.eobrun;
        let refine_existing = |jpeg: &mut Self, position: usize| {
            let value = i32::from(jpeg.components[index].coefficients[block][position]);
            if jpeg.get_bits(1) != 0 && value & p1 == 0 {
                let updated = if value >= 0 { value + p1 } else { value + m1 };
                jpeg.components[index].coefficients[block][position] = updated as i16;
            }
        };
        if eobrun == 0 {
            loop {
                let symbol = self.decode(table);
                let (mut r, mut s) = (symbol >> 4, symbol & 15);
                if s != 0 {
                    s = if self.get_bits(1) != 0 { p1 } else { m1 };
                } else if r != 15 {
                    eobrun = 1 << r;
                    if r != 0 {
                        r = self.get_bits(r);
                        eobrun += r as u32;
                    }
                    break;
                }
                loop {
                    let position = self.natural[k];
                    if self.components[index].coefficients[block][position] != 0 {
                        refine_existing(self, position);
                    } else {
                        r -= 1;
                        if r < 0 {
                            break;
                        }
                    }
                    k += 1;
                    if k > se {
                        break;
                    }
                }
                if s != 0 {
                    let position = self.natural[k];
                    self.components[index].coefficients[block][position] = s as i16;
                }
                k += 1;
                if k > se {
                    break;
                }
            }
        }
        if eobrun != 0 {
            while k <= se {
                let position = self.natural[k];
                if self.components[index].coefficients[block][position] != 0 {
                    refine_existing(self, position);
                }
                k += 1;
            }
            eobrun -= 1;
        }
        self.eobrun = eobrun;
    }


    fn arith_byte(&mut self) -> i32 {
        i32::from(self.byte())
    }

    fn bin(&mut self, bin: Bin) -> &mut u8 {
        match bin {
            Bin::Dc(table, offset) => &mut self.dc_stats[table][offset],
            Bin::Ac(table, offset) => &mut self.ac_stats[table][offset],
            Bin::Fixed => &mut self.fixed_bin[0],
        }
    }

    fn arith_decode(&mut self, bin: Bin) -> i32 {
        while self.a < 0x8000 {
            self.ct -= 1;
            if self.ct < 0 {
                let data = if self.unread_marker != 0 {
                    0
                } else {
                    let mut data = self.arith_byte();
                    if data == 0xFF {
                        loop {
                            data = self.arith_byte();
                            if data != 0xFF {
                                break;
                            }
                        }
                        if data == 0 {
                            data = 0xFF;
                        } else {
                            self.unread_marker = data as u8;
                            data = 0;
                        }
                    }
                    data
                };
                self.c = self.c.wrapping_shl(8) | data;
                self.ct += 8;
                if self.ct < 0 {
                    self.ct += 1;
                    if self.ct == 0 {
                        self.a = 0x8000;
                    }
                }
            }
            self.a = self.a.wrapping_shl(1);
        }
        let sv = i32::from(*self.bin(bin));
        let mut qe = ARITAB[(sv & 0x7F) as usize] as i32;
        let nl = (qe & 0xFF) as u8;
        qe >>= 8;
        let nm = (qe & 0xFF) as u8;
        qe >>= 8;
        let mut temp = self.a - qe;
        self.a = temp;
        temp = temp.wrapping_shl(self.ct as u32);
        let mut sv = sv;
        let state = (sv & 0x80) as u8;
        if self.c >= temp {
            self.c -= temp;
            if self.a < qe {
                self.a = qe;
                *self.bin(bin) = state ^ nm;
            } else {
                self.a = qe;
                *self.bin(bin) = state ^ nl;
                sv ^= 0x80;
            }
        } else if self.a < 0x8000 {
            if self.a < qe {
                *self.bin(bin) = state ^ nl;
                sv ^= 0x80;
            } else {
                *self.bin(bin) = state ^ nm;
            }
        }
        sv >> 7
    }

    fn arith_dc(&mut self, ci: usize, table: usize) -> Option<i32> {
        let context = self.dc_context[ci];
        if self.arith_decode(Bin::Dc(table, context)) == 0 {
            self.dc_context[ci] = 0;
            return Some(0);
        }
        let sign = self.arith_decode(Bin::Dc(table, context + 1));
        let mut st = context + 2 + sign as usize;
        let mut m = self.arith_decode(Bin::Dc(table, st));
        if m != 0 {
            st = 20;
            while self.arith_decode(Bin::Dc(table, st)) != 0 {
                m <<= 1;
                if m == 0x8000 {
                    self.ct = -1;
                    return None;
                }
                st += 1;
            }
        }
        self.dc_context[ci] = if m < ((1_i64 << self.arith_dc_l[table]) >> 1) as i32 {
            0
        } else if m > ((1_i64 << self.arith_dc_u[table]) >> 1) as i32 {
            12 + (sign as usize * 4)
        } else {
            4 + (sign as usize * 4)
        };
        let mut v = m;
        st += 14;
        while {
            m >>= 1;
            m != 0
        } {
            if self.arith_decode(Bin::Dc(table, st)) != 0 {
                v |= m;
            }
        }
        v += 1;
        Some(if sign != 0 { -v } else { v })
    }

    fn arith_ac_value(&mut self, table: usize, k: i32, mut st: usize) -> Option<i32> {
        let sign = self.arith_decode(Bin::Fixed);
        st += 2;
        let mut m = self.arith_decode(Bin::Ac(table, st));
        if m != 0 && self.arith_decode(Bin::Ac(table, st)) != 0 {
            m <<= 1;
            st = if k <= i32::from(self.arith_ac_k[table]) {
                189
            } else {
                217
            };
            while self.arith_decode(Bin::Ac(table, st)) != 0 {
                m <<= 1;
                if m == 0x8000 {
                    self.ct = -1;
                    return None;
                }
                st += 1;
            }
        }
        let mut v = m;
        st += 14;
        while {
            m >>= 1;
            m != 0
        } {
            if self.arith_decode(Bin::Ac(table, st)) != 0 {
                v |= m;
            }
        }
        v += 1;
        Some(if sign != 0 { -v } else { v })
    }

    fn decode_mcu_arith(&mut self, blocks: &[(usize, usize)]) {
        if self.restart_interval != 0 {
            if self.restarts_to_go == 0 {
                self.restart();
            }
            self.restarts_to_go = self.restarts_to_go.wrapping_sub(1);
        }
        if self.progressive && self.ah != 0 && self.ss == 0 {
            let p1 = 1_i16 << self.al;
            for &(component, block) in blocks {
                if self.arith_decode(Bin::Fixed) != 0 {
                    let index = self.scan[component];
                    self.components[index].coefficients[block][0] |= p1;
                }
            }
            return;
        }
        if self.ct == -1 {
            return;
        }
        if !self.progressive || self.ss == 0 {
            for &(component, block) in blocks {
                let index = self.scan[component];
                let table = self.components[index].dc_table;
                let Some(diff) = self.arith_dc(component, table) else {
                    return;
                };
                self.last_dc[component] = self.last_dc[component].wrapping_add(diff);
                let shift = if self.progressive { self.al as u32 } else { 0 };
                self.components[index].coefficients[block][0] =
                    self.last_dc[component].wrapping_shl(shift) as i16;
                if self.progressive || self.lim_se == 0 {
                    continue;
                }
                let table = self.components[index].ac_table;
                let mut k = 0;
                loop {
                    let mut st = 3 * k as usize;
                    if self.arith_decode(Bin::Ac(table, st)) != 0 {
                        break;
                    }
                    loop {
                        k += 1;
                        if self.arith_decode(Bin::Ac(table, st + 1)) != 0 {
                            break;
                        }
                        st += 3;
                        if k >= self.lim_se {
                            self.ct = -1;
                            return;
                        }
                    }
                    let Some(v) = self.arith_ac_value(table, k, st) else {
                        return;
                    };
                    self.components[index].coefficients[block][self.natural[k as usize]] = v as i16;
                    if k >= self.lim_se {
                        break;
                    }
                }
            }
        } else if self.ah == 0 {
            let (component, block) = blocks[0];
            let index = self.scan[component];
            let table = self.components[index].ac_table;
            let mut k = self.ss - 1;
            loop {
                let mut st = (3 * k) as usize;
                if self.arith_decode(Bin::Ac(table, st)) != 0 {
                    break;
                }
                loop {
                    k += 1;
                    if self.arith_decode(Bin::Ac(table, st + 1)) != 0 {
                        break;
                    }
                    st += 3;
                    if k >= self.se {
                        self.ct = -1;
                        return;
                    }
                }
                let Some(v) = self.arith_ac_value(table, k, st) else {
                    return;
                };
                self.components[index].coefficients[block][self.natural[k as usize]] =
                    v.wrapping_shl(self.al as u32) as i16;
                if k >= self.se {
                    break;
                }
            }
        } else {
            let (component, block) = blocks[0];
            let index = self.scan[component];
            let table = self.components[index].ac_table;
            let p1 = 1_i32 << self.al;
            let m1 = (-1_i32).wrapping_shl(self.al as u32);
            let mut kex = self.se;
            loop {
                if self.components[index].coefficients[block][self.natural[kex as usize]] != 0 {
                    break;
                }
                kex -= 1;
                if kex == 0 {
                    break;
                }
            }
            let mut k = self.ss - 1;
            loop {
                let mut st = (3 * k) as usize;
                if k >= kex && self.arith_decode(Bin::Ac(table, st)) != 0 {
                    break;
                }
                loop {
                    k += 1;
                    let position = self.natural[k as usize];
                    let value = i32::from(self.components[index].coefficients[block][position]);
                    if value != 0 {
                        if self.arith_decode(Bin::Ac(table, st + 2)) != 0 {
                            let updated = if value < 0 { value + m1 } else { value + p1 };
                            self.components[index].coefficients[block][position] = updated as i16;
                        }
                        break;
                    }
                    if self.arith_decode(Bin::Ac(table, st + 1)) != 0 {
                        let set = if self.arith_decode(Bin::Fixed) != 0 {
                            m1
                        } else {
                            p1
                        };
                        self.components[index].coefficients[block][position] = set as i16;
                        break;
                    }
                    st += 3;
                    if k >= self.se {
                        self.ct = -1;
                        return;
                    }
                }
                if k >= self.se {
                    break;
                }
            }
        }
    }

    fn decode_scan(&mut self) -> Result<(), Stop> {
        let tables = if self.arith {
            None
        } else {
            Some(self.derive_tables()?)
        };
        let (rows, columns) = if self.scan.len() == 1 {
            let component = &self.components[self.scan[0]];
            (component.height_in_blocks, component.width_in_blocks)
        } else {
            let block = self.block_size;
            (
                self.height.div_ceil(self.max_v * block),
                self.width.div_ceil(self.max_h * block),
            )
        };
        let mut blocks = Vec::with_capacity(10);
        for row in 0..rows {
            for column in 0..columns {
                blocks.clear();
                for (ci, &index) in self.scan.iter().enumerate() {
                    let component = &self.components[index];
                    for y in 0..component.mcu_height {
                        for x in 0..component.mcu_width {
                            let block_row = row * component.mcu_height + y;
                            let block_column = column * component.mcu_width + x;
                            blocks.push((ci, block_row * component.stride + block_column));
                        }
                    }
                }
                match &tables {
                    Some((dc, ac)) => self.decode_mcu_huffman(&blocks, dc, ac),
                    None => self.decode_mcu_arith(&blocks),
                }
            }
        }
        if !self.arith {
            self.bits_left = 0;
        }
        Ok(())
    }
}

pub(super) struct Decoded {
    pub width: usize,
    pub height: usize,
    pub components: usize,
    pub cmyk: bool,
    pub samples: Vec<u8>,
}

pub(super) fn idct_limit(x: i32) -> u8 {
    let v = x & 1023;
    if v < 128 {
        (v + 128) as u8
    } else if v < 512 {
        255
    } else if v < 896 {
        0
    } else {
        (v - 896) as u8
    }
}

fn sample_limit(x: i32) -> u8 {
    x.clamp(0, 255) as u8
}

fn idct_ifast(block: &[i16; 64], table: &[i32; 64], out: &mut [u8], stride: usize) {
    const FIX_1_082392200: i32 = 277;
    const FIX_1_414213562: i32 = 362;
    const FIX_1_847759065: i32 = 473;
    const FIX_2_613125930: i32 = 669;
    let multiply = |var: i32, c: i32| var.wrapping_mul(c) >> 8;
    let mut ws = [0_i32; 64];
    for col in 0..8 {
        let at = |row: usize| i32::from(block[row * 8 + col]);
        let q = |row: usize| table[row * 8 + col];
        if (1..8).all(|row| at(row) == 0) {
            let dc = at(0).wrapping_mul(q(0));
            for row in 0..8 {
                ws[row * 8 + col] = dc;
            }
            continue;
        }
        let tmp0 = at(0).wrapping_mul(q(0));
        let tmp1 = at(2).wrapping_mul(q(2));
        let tmp2 = at(4).wrapping_mul(q(4));
        let tmp3 = at(6).wrapping_mul(q(6));
        let tmp10 = tmp0.wrapping_add(tmp2);
        let tmp11 = tmp0.wrapping_sub(tmp2);
        let tmp13 = tmp1.wrapping_add(tmp3);
        let tmp12 = multiply(tmp1.wrapping_sub(tmp3), FIX_1_414213562).wrapping_sub(tmp13);
        let tmp0 = tmp10.wrapping_add(tmp13);
        let tmp3 = tmp10.wrapping_sub(tmp13);
        let tmp1 = tmp11.wrapping_add(tmp12);
        let tmp2 = tmp11.wrapping_sub(tmp12);
        let tmp4 = at(1).wrapping_mul(q(1));
        let tmp5 = at(3).wrapping_mul(q(3));
        let tmp6 = at(5).wrapping_mul(q(5));
        let tmp7 = at(7).wrapping_mul(q(7));
        let z13 = tmp6.wrapping_add(tmp5);
        let z10 = tmp6.wrapping_sub(tmp5);
        let z11 = tmp4.wrapping_add(tmp7);
        let z12 = tmp4.wrapping_sub(tmp7);
        let tmp7 = z11.wrapping_add(z13);
        let tmp11 = multiply(z11.wrapping_sub(z13), FIX_1_414213562);
        let z5 = multiply(z10.wrapping_add(z12), FIX_1_847759065);
        let tmp10 = multiply(z12, FIX_1_082392200).wrapping_sub(z5);
        let tmp12 = multiply(z10, -FIX_2_613125930).wrapping_add(z5);
        let tmp6 = tmp12.wrapping_sub(tmp7);
        let tmp5 = tmp11.wrapping_sub(tmp6);
        let tmp4 = tmp10.wrapping_add(tmp5);
        ws[col] = tmp0.wrapping_add(tmp7);
        ws[56 + col] = tmp0.wrapping_sub(tmp7);
        ws[8 + col] = tmp1.wrapping_add(tmp6);
        ws[48 + col] = tmp1.wrapping_sub(tmp6);
        ws[16 + col] = tmp2.wrapping_add(tmp5);
        ws[40 + col] = tmp2.wrapping_sub(tmp5);
        ws[32 + col] = tmp3.wrapping_add(tmp4);
        ws[24 + col] = tmp3.wrapping_sub(tmp4);
    }
    for row in 0..8 {
        let w = &ws[row * 8..row * 8 + 8];
        let line = &mut out[row * stride..row * stride + 8];
        if w[1..].iter().all(|value| *value == 0) {
            line.fill(idct_limit(w[0] >> 5));
            continue;
        }
        let tmp10 = w[0].wrapping_add(w[4]);
        let tmp11 = w[0].wrapping_sub(w[4]);
        let tmp13 = w[2].wrapping_add(w[6]);
        let tmp12 = multiply(w[2].wrapping_sub(w[6]), FIX_1_414213562).wrapping_sub(tmp13);
        let tmp0 = tmp10.wrapping_add(tmp13);
        let tmp3 = tmp10.wrapping_sub(tmp13);
        let tmp1 = tmp11.wrapping_add(tmp12);
        let tmp2 = tmp11.wrapping_sub(tmp12);
        let z13 = w[5].wrapping_add(w[3]);
        let z10 = w[5].wrapping_sub(w[3]);
        let z11 = w[1].wrapping_add(w[7]);
        let z12 = w[1].wrapping_sub(w[7]);
        let tmp7 = z11.wrapping_add(z13);
        let tmp11 = multiply(z11.wrapping_sub(z13), FIX_1_414213562);
        let z5 = multiply(z10.wrapping_add(z12), FIX_1_847759065);
        let tmp10 = multiply(z12, FIX_1_082392200).wrapping_sub(z5);
        let tmp12 = multiply(z10, -FIX_2_613125930).wrapping_add(z5);
        let tmp6 = tmp12.wrapping_sub(tmp7);
        let tmp5 = tmp11.wrapping_sub(tmp6);
        let tmp4 = tmp10.wrapping_add(tmp5);
        line[0] = idct_limit(tmp0.wrapping_add(tmp7) >> 5);
        line[7] = idct_limit(tmp0.wrapping_sub(tmp7) >> 5);
        line[1] = idct_limit(tmp1.wrapping_add(tmp6) >> 5);
        line[6] = idct_limit(tmp1.wrapping_sub(tmp6) >> 5);
        line[2] = idct_limit(tmp2.wrapping_add(tmp5) >> 5);
        line[5] = idct_limit(tmp2.wrapping_sub(tmp5) >> 5);
        line[4] = idct_limit(tmp3.wrapping_add(tmp4) >> 5);
        line[3] = idct_limit(tmp3.wrapping_sub(tmp4) >> 5);
    }
}

fn smoothed(component: &Component, bits: &[i32; 64], row: usize, column: usize) -> [i16; 64] {
    let quant = component.quant.unwrap();
    let block = |r: usize, c: usize| component.coefficients[r * component.stride + c];
    let mut workspace = block(row, column);
    let last_row = component.height_in_blocks - 1;
    let last_column = component.width_in_blocks - 1;
    let prev = if row == 0 { row } else { row - 1 };
    let next = if row == last_row { row } else { row + 1 };
    let dc = |r: usize, c: usize| i32::from(block(r, c)[0]);
    let left = if column == 0 { column } else { column - 1 };
    let right = if column == last_column {
        column
    } else {
        column + 1
    };
    let (dc1, dc2, dc3) = (dc(prev, left), dc(prev, column), dc(prev, right));
    let (dc4, dc5, dc6) = (dc(row, left), dc(row, column), dc(row, right));
    let (dc7, dc8, dc9) = (dc(next, left), dc(next, column), dc(next, right));
    let q = |position: usize| i32::from(quant[position]);
    let q00 = q(0);
    let predict = |al: i32, num: i32, quant: i32| -> i16 {
        let divisor = quant.wrapping_shl(8);
        let pred = if num >= 0 {
            let pred = (quant.wrapping_shl(7).wrapping_add(num)) / divisor;
            if al > 0 && pred >= (1 << al) {
                (1 << al) - 1
            } else {
                pred
            }
        } else {
            let pred = (quant.wrapping_shl(7).wrapping_sub(num)) / divisor;
            let pred = if al > 0 && pred >= (1 << al) {
                (1 << al) - 1
            } else {
                pred
            };
            -pred
        };
        pred as i16
    };
    let terms: [(usize, usize, i32, usize); 5] = [
        (1, 1, 36_i32.wrapping_mul(q00).wrapping_mul(dc4 - dc6), 1),
        (2, 8, 36_i32.wrapping_mul(q00).wrapping_mul(dc2 - dc8), 8),
        (
            3,
            16,
            9_i32.wrapping_mul(q00).wrapping_mul(dc2 + dc8 - 2 * dc5),
            16,
        ),
        (
            4,
            9,
            5_i32.wrapping_mul(q00).wrapping_mul(dc1 - dc3 - dc7 + dc9),
            9,
        ),
        (
            5,
            2,
            9_i32.wrapping_mul(q00).wrapping_mul(dc4 + dc6 - 2 * dc5),
            2,
        ),
    ];
    for (coefficient, position, num, quant_position) in terms {
        let al = bits[coefficient];
        if al != 0 && workspace[position] == 0 {
            workspace[position] = predict(al, num, q(quant_position));
        }
    }
    workspace
}

pub(super) fn decode(data: &[u8]) -> Result<Decoded, Stop> {
    let mut jpeg = Jpeg {
        data,
        position: 0,
        unread_marker: 0,
        saw_soi: false,
        saw_sof: false,
        restart_interval: 0,
        arith_dc_l: [0; 16],
        arith_dc_u: [1; 16],
        arith_ac_k: [5; 16],
        saw_jfif: false,
        saw_adobe: false,
        adobe_transform: 0,
        color_transform: false,
        is_baseline: false,
        progressive: false,
        arith: false,
        precision: 0,
        height: 0,
        width: 0,
        components: Vec::new(),
        quant_tables: [None; 4],
        dc_tables: Default::default(),
        ac_tables: Default::default(),
        scan: Vec::new(),
        ss: 0,
        se: 0,
        ah: 0,
        al: 0,
        max_h: 1,
        max_v: 1,
        lim_se: 63,
        block_size: 8,
        natural: &NATURAL,
        coef_bits: Vec::new(),
        get_buffer: 0,
        bits_left: 0,
        insufficient: false,
        eobrun: 0,
        last_dc: [0; 4],
        restarts_to_go: 0,
        next_restart: 0,
        c: 0,
        a: 0,
        ct: 0,
        dc_context: [0; 4],
        dc_stats: [[0; 64]; 16],
        ac_stats: [[0; 256]; 16],
        fixed_bin: [113, 0, 0, 0],
    };
    let mut in_headers = 1;
    loop {
        if !jpeg.read_markers()? {
            return if jpeg.saw_sof {
                fatal("Invalid JPEG file structure: missing SOS marker")
            } else {
                fatal("JPEG datastream contains no image")
            };
        }
        if in_headers == 1 {
            jpeg.initial_setup()?;
        }
        if jpeg.scan.is_empty() {
            in_headers = 2;
            continue;
        }
        break;
    }
    let count = jpeg.components.len();
    let multiple_scans = jpeg.scan.len() < count || jpeg.progressive;
    enum Space {
        Gray,
        YCbCr,
        BgYcc,
        Rgb,
        Cmyk,
        Ycck,
        Unknown,
    }
    let ids: Vec<i32> = jpeg
        .components
        .iter()
        .map(|component| component.id)
        .collect();
    let space = match count {
        1 => Space::Gray,
        3 => match (ids[0], ids[1], ids[2]) {
            (1, 2, 3) => Space::YCbCr,
            (1, 0x22, 0x23) => Space::BgYcc,
            (0x52, 0x47, 0x42) => Space::Rgb,
            (0x72, 0x67, 0x62) => return fatal("Unsupported color conversion request"),
            _ if jpeg.saw_jfif => Space::YCbCr,
            _ if jpeg.saw_adobe && jpeg.adobe_transform == 0 => Space::Rgb,
            _ => Space::YCbCr,
        },
        4 if jpeg.saw_adobe && jpeg.adobe_transform != 0 => Space::Ycck,
        4 => Space::Cmyk,
        _ => Space::Unknown,
    };
    if jpeg.precision != 8 {
        return fatal(format!(
            "Unsupported JPEG data precision {}",
            jpeg.precision
        ));
    }
    if jpeg.color_transform && !matches!(space, Space::Rgb) {
        return fatal("Unsupported color conversion request");
    }
    super::oversized(jpeg.width as u32, jpeg.height as u32).map_err(|failure| match failure {
        Failure::Undefined(why) | Failure::Undecodable(why) | Failure::Unsupported(why) => Stop::Undefined(why),
        Failure::Device(why) => Stop::Device(why),
        Failure::OverBudget(why) => Stop::OverBudget(why),
    })?;
    let size = jpeg.block_size;
    let scaled = |samp: usize, max: usize| {
        let mut ssize = 1;
        while size * ssize <= 4 && max.is_multiple_of(samp * ssize * 2) {
            ssize *= 2;
        }
        size * ssize
    };
    let dct_sizes: Vec<(usize, usize)> = jpeg
        .components
        .iter()
        .map(|component| {
            let (mut h, mut v) = (
                scaled(component.h, jpeg.max_h),
                scaled(component.v, jpeg.max_v),
            );
            if h > v * 2 {
                h = v * 2;
            } else if v > h * 2 {
                v = h * 2;
            }
            (h, v)
        })
        .collect();
    for (component, &(h_size, v_size)) in jpeg.components.iter().zip(&dct_sizes) {
        let (h_in, v_in) = (component.h * h_size / size, component.v * v_size / size);
        if !jpeg.max_h.is_multiple_of(h_in) || !jpeg.max_v.is_multiple_of(v_in) {
            return fatal("Fractional sampling not implemented yet");
        }
    }
    if jpeg.progressive {
        jpeg.coef_bits = vec![[-1; 64]; count];
    }
    jpeg.start_scan()?;
    jpeg.decode_scan()?;
    loop {
        if !jpeg.read_markers()? {
            break;
        }
        if !multiple_scans {
            return fatal("Didn't expect more than one scan");
        }
        if jpeg.scan.is_empty() {
            continue;
        }
        jpeg.start_scan()?;
        jpeg.decode_scan()?;
    }
    let smoothing = jpeg.progressive
        && jpeg.components.iter().enumerate().all(|(ci, component)| {
            component
                .quant
                .is_some_and(|q| [0, 1, 8, 16, 9, 2].iter().all(|&p| q[p] != 0))
                && jpeg.coef_bits[ci][0] >= 0
        })
        && jpeg
            .coef_bits
            .iter()
            .any(|bits| bits[1..=5].iter().any(|b| *b != 0));
    let mut planes = Vec::with_capacity(count);
    for (ci, component) in jpeg.components.iter().enumerate() {
        let (h_size, v_size) = dct_sizes[ci];
        let multipliers: [i32; 64] = match component.quant {
            Some(quant) if (h_size, v_size) == (8, 8) => {
                std::array::from_fn(|i| (i32::from(quant[i]) * AANSCALES[i] + (1 << 11)) >> 12)
            }
            Some(quant) => std::array::from_fn(|i| i32::from(quant[i])),
            None => [0; 64],
        };
        let stride = component.width_in_blocks * h_size;
        let mut plane = vec![0_u8; stride * component.height_in_blocks * v_size];
        for row in 0..component.height_in_blocks {
            for column in 0..component.width_in_blocks {
                let block = if smoothing {
                    smoothed(component, &jpeg.coef_bits[ci], row, column)
                } else {
                    component.coefficients[row * component.stride + column]
                };
                let at = row * v_size * stride + column * h_size;
                if (h_size, v_size) == (8, 8) {
                    idct_ifast(&block, &multipliers, &mut plane[at..], stride);
                } else if super::libjpeg_idct::idct(
                    h_size,
                    v_size,
                    &block,
                    &multipliers,
                    &mut plane[at..],
                    stride,
                )
                .is_none()
                {
                    return fatal(format!(
                        "IDCT output block size {h_size}x{v_size} not supported"
                    ));
                }
            }
        }
        planes.push((plane, stride));
    }
    let (width, height) = (jpeg.width, jpeg.height);
    let sample = |ci: usize, x: usize, y: usize| {
        let component = &jpeg.components[ci];
        let (plane, stride) = &planes[ci];
        let (h_size, v_size) = dct_sizes[ci];
        let sx = x * (component.h * h_size / size) / jpeg.max_h;
        let sy = y * (component.v * v_size / size) / jpeg.max_v;
        i32::from(plane[sy * stride + sx])
    };
    let fix = |x: f64| (x * 65536.0 + 0.5) as i32;
    let tables = |bg: bool| {
        let (r, b, rg, bgc) = if bg {
            (fix(2.804), fix(3.544), fix(1.428272572), fix(0.688272572))
        } else {
            (fix(1.402), fix(1.772), fix(0.714136286), fix(0.344136286))
        };
        move |y: i32, cb: i32, cr: i32| {
            let (x_cb, x_cr) = (cb - 128, cr - 128);
            let red = (r * x_cr + 32768) >> 16;
            let blue = (b * x_cb + 32768) >> 16;
            let green = ((-bgc) * x_cb + 32768 + (-rg) * x_cr) >> 16;
            (y + red, y + green, y + blue)
        }
    };
    let components_out = match space {
        Space::Gray => 1,
        Space::Cmyk | Space::Ycck => 4,
        Space::Unknown => count,
        _ => 3,
    };
    let mut samples = Vec::with_capacity(width * height * components_out);
    for y in 0..height {
        for x in 0..width {
            match space {
                Space::Gray => samples.push(sample(0, x, y) as u8),
                Space::YCbCr | Space::BgYcc => {
                    let bg = matches!(space, Space::BgYcc);
                    let (r, g, b) = tables(bg)(sample(0, x, y), sample(1, x, y), sample(2, x, y));
                    samples.extend([r, g, b].map(sample_limit));
                }
                Space::Rgb => {
                    let (r, g, b) = (sample(0, x, y), sample(1, x, y), sample(2, x, y));
                    if jpeg.color_transform {
                        samples.extend([
                            ((r + g - 128) & 255) as u8,
                            g as u8,
                            ((b + g - 128) & 255) as u8,
                        ]);
                    } else {
                        samples.extend([r as u8, g as u8, b as u8]);
                    }
                }
                Space::Ycck => {
                    let (r, g, b) =
                        tables(false)(sample(0, x, y), sample(1, x, y), sample(2, x, y));
                    samples.extend([255 - r, 255 - g, 255 - b].map(sample_limit));
                    samples.push(sample(3, x, y) as u8);
                }
                Space::Cmyk | Space::Unknown => {
                    for ci in 0..count {
                        samples.push(sample(ci, x, y) as u8);
                    }
                }
            }
        }
    }
    Ok(Decoded {
        width,
        height,
        components: components_out,
        cmyk: matches!(space, Space::Cmyk | Space::Ycck),
        samples,
    })
}
