#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod encode;

#[cfg(test)]
mod tests;

pub const HEADER_LEN: usize = 16;

pub const SCALAR_BLOCK_LEN: usize = 20;

pub const STRING_BLOCK_OFFSET: usize = HEADER_LEN + SCALAR_BLOCK_LEN;

pub const MAX_TOKEN_LEN: usize = 128;

/// token order, `KNOWN_FLAGS`, the walk and the encoder are all generated from
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Field {
    Player,
    Dlcs,
    StartDate,
    SaveVersion,
    Session,
    Ideology,
    CosmeticTag,
    Version,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Slot {
    Scalar { offset: usize },
    Token { order: usize },
}

impl Field {
    /// Every field, in wire order. The whole crate is generated from this.
    pub const ALL: [Field; 8] = [
        Field::Player,
        Field::Dlcs,
        Field::StartDate,
        Field::SaveVersion,
        Field::Session,
        Field::Ideology,
        Field::CosmeticTag,
        Field::Version,
    ];

    pub const fn flag(self) -> u32 {
        match self {
            Field::Player => 1 << 0,
            Field::Dlcs => 1 << 1,
            Field::StartDate => 1 << 2,
            Field::SaveVersion => 1 << 3,
            Field::Session => 1 << 4,
            Field::Ideology => 1 << 5,
            Field::CosmeticTag => 1 << 6,
            Field::Version => 1 << 7,
        }
    }

    pub const fn slot(self) -> Slot {
        match self {
            Field::Player => Slot::Scalar { offset: 0 },
            Field::Dlcs => Slot::Scalar { offset: 4 },
            Field::StartDate => Slot::Scalar { offset: 8 },
            Field::SaveVersion => Slot::Scalar { offset: 12 },
            Field::Session => Slot::Scalar { offset: 16 },
            Field::Ideology => Slot::Token { order: 0 },
            Field::CosmeticTag => Slot::Token { order: 1 },
            Field::Version => Slot::Token { order: 2 },
        }
    }

    pub const fn diagnostic_name(self) -> &'static str {
        match self {
            Field::Player => "player_tag_db_id",
            Field::Dlcs => "dlcs",
            Field::StartDate => "start_date",
            Field::SaveVersion => "save_version",
            Field::Session => "session",
            Field::Ideology => "ideology",
            Field::CosmeticTag => "cosmetic_tag",
            Field::Version => "version",
        }
    }

    pub const fn is_present(self, presence: u32) -> bool {
        presence & self.flag() != 0
    }
}

pub const SCALAR_FIELDS: [Field; 5] = [
    Field::Player,
    Field::Dlcs,
    Field::StartDate,
    Field::SaveVersion,
    Field::Session,
];

pub const TOKEN_FIELDS: [Field; 3] = [Field::Ideology, Field::CosmeticTag, Field::Version];

pub const SCALAR_OFFSETS: [usize; SCALAR_FIELDS.len()] = scalar_offsets();

const fn scalar_offsets() -> [usize; SCALAR_FIELDS.len()] {
    let mut out = [0usize; SCALAR_FIELDS.len()];
    let mut i = 0;
    while i < SCALAR_FIELDS.len() {
        out[i] = match SCALAR_FIELDS[i].slot() {
            Slot::Scalar { offset } => offset,
            Slot::Token { .. } => panic!("d68_grammar: SCALAR_FIELDS holds a token field"),
        };
        i += 1;
    }
    out
}

const _: () = {
    let mut i = 0;
    while i < TOKEN_FIELDS.len() {
        match TOKEN_FIELDS[i].slot() {
            Slot::Token { order } => {
                if order != i {
                    panic!("d68_grammar: TOKEN_FIELDS is not in Slot::Token order");
                }
            }
            Slot::Scalar { .. } => panic!("d68_grammar: TOKEN_FIELDS holds a scalar field"),
        }
        i += 1;
    }
    if SCALAR_FIELDS.len() + TOKEN_FIELDS.len() != Field::ALL.len() {
        panic!("d68_grammar: SCALAR_FIELDS + TOKEN_FIELDS do not cover Field::ALL");
    }
};

pub const KNOWN_FLAGS: u32 = known_flags();

const fn known_flags() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < Field::ALL.len() {
        mask |= Field::ALL[i].flag();
        i += 1;
    }
    mask
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    Header,
    ScalarBlock,
    TokenPrefix(Field),
    TokenBytes(Field),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViolationKind {
    Need {
        section: Section,
        needed: usize,
        available: usize,
    },
    TokenTooLong {
        field: Field,
        len: usize,
        max: usize,
    },
    OffsetOverflow {
        section: Section,
        at: usize,
        len: usize,
    },
    TrailingBytes {
        consumed: usize,
        available: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Violation {
    pub at: usize,
    pub kind: ViolationKind,
}

impl Violation {
    const fn new(at: usize, kind: ViolationKind) -> Self {
        Violation { at, kind }
    }
}

pub(crate) fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

pub(crate) fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub(crate) fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
        bytes[at + 4],
        bytes[at + 5],
        bytes[at + 6],
        bytes[at + 7],
    ])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub elapsed_wall_ms: u64,
    pub presence: u32,
    pub status_flags: u32,
}

pub fn read_header(payload: &[u8]) -> Result<Header, Violation> {
    if payload.len() < HEADER_LEN {
        return Err(Violation::new(
            0,
            ViolationKind::Need {
                section: Section::Header,
                needed: HEADER_LEN,
                available: payload.len(),
            },
        ));
    }
    Ok(Header {
        elapsed_wall_ms: u64_at(payload, 0),
        presence: u32_at(payload, 8),
        status_flags: u32_at(payload, 12),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScalarBlock {
    words: [u32; SCALAR_FIELDS.len()],
}

impl ScalarBlock {
    pub fn word(&self, field: Field) -> Option<u32> {
        match field.slot() {
            Slot::Scalar { offset } => Some(self.words[offset / 4]),
            Slot::Token { .. } => None,
        }
    }

    pub fn words(&self) -> [u32; SCALAR_FIELDS.len()] {
        self.words
    }
}

pub fn read_scalar_block(payload: &[u8]) -> Result<ScalarBlock, Violation> {
    if payload.len() < STRING_BLOCK_OFFSET {
        return Err(Violation::new(
            HEADER_LEN,
            ViolationKind::Need {
                section: Section::ScalarBlock,
                needed: STRING_BLOCK_OFFSET,
                available: payload.len(),
            },
        ));
    }
    let mut words = [0u32; SCALAR_FIELDS.len()];
    let mut i = 0;
    while i < SCALAR_FIELDS.len() {
        words[i] = u32_at(payload, HEADER_LEN + SCALAR_OFFSETS[i]);
        i += 1;
    }
    Ok(ScalarBlock { words })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenStep<'a> {
    Token { field: Field, bytes: &'a [u8] },
    End(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct TokenCursor {
    at: usize,
    next_token: usize,
}

impl TokenCursor {
    pub const fn new() -> Self {
        TokenCursor {
            at: STRING_BLOCK_OFFSET,
            next_token: 0,
        }
    }

    pub const fn offset(&self) -> usize {
        self.at
    }

    pub fn next<'a>(&mut self, payload: &'a [u8]) -> Result<TokenStep<'a>, Violation> {
        if self.next_token >= TOKEN_FIELDS.len() {
            return Ok(TokenStep::End(self.at));
        }
        let field = TOKEN_FIELDS[self.next_token];
        let prefix_end = self.at.checked_add(2).ok_or(Violation::new(
            self.at,
            ViolationKind::OffsetOverflow {
                section: Section::TokenPrefix(field),
                at: self.at,
                len: 2,
            },
        ))?;
        if prefix_end > payload.len() {
            return Err(Violation::new(
                self.at,
                ViolationKind::Need {
                    section: Section::TokenPrefix(field),
                    needed: prefix_end,
                    available: payload.len(),
                },
            ));
        }
        let len = u16_at(payload, self.at) as usize;
        if len > MAX_TOKEN_LEN {
            return Err(Violation::new(
                prefix_end,
                ViolationKind::TokenTooLong {
                    field,
                    len,
                    max: MAX_TOKEN_LEN,
                },
            ));
        }
        let end = prefix_end.checked_add(len).ok_or(Violation::new(
            prefix_end,
            ViolationKind::OffsetOverflow {
                section: Section::TokenBytes(field),
                at: prefix_end,
                len,
            },
        ))?;
        if end > payload.len() {
            return Err(Violation::new(
                prefix_end,
                ViolationKind::Need {
                    section: Section::TokenBytes(field),
                    needed: end,
                    available: payload.len(),
                },
            ));
        }
        self.at = end;
        self.next_token += 1;
        Ok(TokenStep::Token {
            field,
            bytes: &payload[prefix_end..end],
        })
    }
}

impl Default for TokenCursor {
    fn default() -> Self {
        TokenCursor::new()
    }
}

pub fn exact_consumption(consumed: usize, payload_len: usize) -> Result<(), Violation> {
    if consumed != payload_len {
        return Err(Violation::new(
            consumed,
            ViolationKind::TrailingBytes {
                consumed,
                available: payload_len,
            },
        ));
    }
    Ok(())
}
