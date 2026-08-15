//! Domain 68 (`session_header_live`) payload grammar — the single structural walk.
//!
//! # Why this crate exists
//!
//! D68's DATA payload grammar was transcribed at FOUR sites: the collector's
//! packer wrote it (`attach_log.rs::compact_session_header_data_payload`),
//! `recorder_host::world_fold::fold_session_header` folded it,
//! `recorder_host::domain_data_authority::decode_compact_session_header_data_payload`
//! gated it, and `wire_format::d68::{split_payload, parse_update}` ported it.
//! The three readers were line-for-line copies of each other, each with its own
//! spelling of the same eight presence flags, the same two block lengths, the
//! same 128-byte token ceiling and the same tag ceiling — twenty layout facts
//! independently restated (`runs/ko_domain_grammars/survey.md`).
//!
//! That is the shape that failed for d13 on 2026-08-14: lane JO added a tail at
//! the producer and one reader, two other readers never learned it, and one of
//! them refused 59 of 59 live records of the domain. Nothing in d68 has gone
//! stale yet — the cross-site value check found no disagreement
//! (`runs/ko_domain_grammars/value_disagreement.txt`) — and the point of
//! converting it before it does is that a NINTH presence field is now a compile
//! error at all four sites instead of a diff four people have to remember.
//!
//! # What is grammar here, and what is not
//!
//! Grammar (this crate): the header's width and field offsets, the scalar
//! block's width and each scalar's offset, which flag bit belongs to which
//! field, the token block's shape (three `u16`-prefixed byte runs, each present
//! in the payload whether or not its flag is set), the token length ceiling, and
//! exact consumption.
//!
//! Semantics (each site): UTF-8 validity, the "absent scalar must pack a zero"
//! rule, the player-tag range, the "flag set but empty" / "bytes without flag"
//! pair, the merge (last-value-wins into the carried header), the status gates,
//! and every identity witness. This crate reads no value it does not need in
//! order to find the next byte boundary, except through [`ScalarBlock`], which
//! is fixed offset arithmetic and validates nothing.
//!
//! # Traversal is LAZY, and that is a requirement
//!
//! The readers interleave their semantics with the walk, and they do NOT agree
//! about fatality: `fold_session_header` returns on the first violated rule,
//! while `decode_compact_session_header_data_payload` records a diagnostic for
//! each violated scalar rule and keeps going. A function that computed a whole
//! payload layout up front would discover a truncated third token before the
//! validator had recorded the first token's rule violation, changing which
//! diagnostics a malformed capture carries. So the token walk is a PULL cursor:
//! the caller asks for the next token, does its own work, and asks again.
//!
//! Everything this crate CAN refuse is fatal at every site — the bytes to find
//! the next boundary are genuinely absent — so unlike `d13_grammar` there is no
//! `Recoverable` step here. Every rule the two readers disagree about the
//! fatality of is a semantic rule, and all of them stayed with their readers.
//!
//! # Violations are data, never text
//!
//! The four sites spell the same violation four ways and always have: the fold
//! says `"session_header header: need 16 bytes, have 8"`, the validator says
//! `"offset 900: session_header payload is 8 byte(s), expected at least 16"`,
//! and the port says `"session_header payload 8 shorter than the 16-byte
//! header"`. [`Violation`] carries the operands and no message; each site owns
//! one exhaustive formatter. An `ErrorStyle` knob inside this crate would make a
//! supposedly neutral leaf know its consumers by name.
//!
//! # The status exit is EARLY, and this crate must not fight it
//!
//! `fold_session_header` runs exactly one check before its failure-status exit —
//! the 16-byte header minimum — and folds nothing on a failure record without
//! ever inspecting the presence word. [`read_header`] therefore checks the
//! header minimum and NOTHING else: not the presence word, not the scalar block.
//! A failure-status record whose presence word is garbage is a record the
//! recorder accepts today, and a leaf that refused it here would refuse a stream
//! the recorder reads happily.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod encode;

#[cfg(test)]
mod tests;

// --- Fixed widths ------------------------------------------------------------

/// The payload header: `elapsed_wall_ms` (u64), `presence` (u32),
/// `status_flags` (u32).
pub const HEADER_LEN: usize = 16;

/// The five `u32` scalars that always follow the header, present or not.
pub const SCALAR_BLOCK_LEN: usize = 20;

/// Where the three length-prefixed tokens begin.
pub const STRING_BLOCK_OFFSET: usize = HEADER_LEN + SCALAR_BLOCK_LEN;

/// The longest token this grammar can express, in bytes.
///
/// This is not a taste judgement: the producer reads all three tokens with a
/// 128-byte ceiling of its own
/// (`compact_contract::build_profile::DOMAIN68_MAX_TOKEN_LEN` /
/// `DOMAIN68_MAX_VERSION_LEN`), so a longer token is impossible writer output
/// and every reader folds closed on one. [`encode::encode_payload`] enforces the
/// same ceiling, which is what makes that an invariant of the grammar instead of
/// a coincidence between two crates.
pub const MAX_TOKEN_LEN: usize = 128;

// --- The field table ---------------------------------------------------------

/// One presence-gated field of the session header.
///
/// **This enum is the drift boundary.** The flag mask, the scalar offsets, the
/// token order, `KNOWN_FLAGS`, the walk and the encoder are all generated from
/// [`Field::ALL`] by exhaustive `match`es with no wildcard arm, and all four
/// consuming sites also match on `Field` with no wildcard. Adding a ninth field
/// is a compile error in this crate, in `fold_session_header`, in
/// `decode_compact_session_header_data_payload`, in `wire_format::d68` and in
/// the collector's packer until each has decided what it means.
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

/// Where a field's bytes live: a fixed word in the scalar block, or one of the
/// length-prefixed tokens behind it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Slot {
    /// A `u32` at `STRING_BLOCK_OFFSET - SCALAR_BLOCK_LEN + offset`.
    Scalar { offset: usize },
    /// The `order`-th `u16`-prefixed token, counting from zero.
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

    /// The presence bit that claims this field.
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

    /// Where this field's bytes are.
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

    /// The name every site already spells this field in its diagnostics.
    ///
    /// The three readers' messages are byte-identical strings built from these
    /// names (`"session_header ideology bytes truncated"`), so the names belong
    /// with the table that decides which fields exist.
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

    /// Whether `presence` claims this field.
    pub const fn is_present(self, presence: u32) -> bool {
        presence & self.flag() != 0
    }
}

/// The scalar fields, in wire order.
pub const SCALAR_FIELDS: [Field; 5] = [
    Field::Player,
    Field::Dlcs,
    Field::StartDate,
    Field::SaveVersion,
    Field::Session,
];

/// The token fields, in wire order.
pub const TOKEN_FIELDS: [Field; 3] = [Field::Ideology, Field::CosmeticTag, Field::Version];

/// Each scalar field's byte offset within the scalar block, in wire order.
///
/// Derived from [`Field::slot`], not restated. The `panic!` below runs in a
/// CONST context, so a table that puts a token field in [`SCALAR_FIELDS`] fails
/// the build; there is no runtime path through it. That is deliberate: a leaf
/// under a recorder that must fail closed does not get to panic at runtime.
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

/// [`TOKEN_FIELDS`] is in `Slot::Token { order }` order, checked at build time
/// for the same reason [`SCALAR_OFFSETS`] is.
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
    // Every field is in exactly one of the two lists, so a ninth field that is
    // added to `Field::ALL` and to neither list fails the build here.
    if SCALAR_FIELDS.len() + TOKEN_FIELDS.len() != Field::ALL.len() {
        panic!("d68_grammar: SCALAR_FIELDS + TOKEN_FIELDS do not cover Field::ALL");
    }
};

/// Every assigned presence bit. Any other bit means the payload is not this
/// grammar, so no bit in it can be trusted — a rule each reader still owns,
/// because the three of them disagree about when it is checked and whether it
/// is fatal.
pub const KNOWN_FLAGS: u32 = known_flags();

const fn known_flags() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    // `while` rather than `for`: `for` is not allowed in a `const fn`.
    while i < Field::ALL.len() {
        mask |= Field::ALL[i].flag();
        i += 1;
    }
    mask
}

// --- Violations --------------------------------------------------------------

/// Which part of the walk ran out of bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    /// The 16-byte header.
    Header,
    /// The 20-byte scalar block behind it.
    ScalarBlock,
    /// A token's two-byte length prefix.
    TokenPrefix(Field),
    /// A token's declared bytes.
    TokenBytes(Field),
}

/// A structural refusal: the bytes needed to find the next boundary are absent,
/// or a declared length is one this grammar cannot express.
///
/// Every variant is fatal at every site. Rules the readers disagree about the
/// fatality of are semantic and stayed with their readers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViolationKind {
    /// The payload is shorter than `needed` bytes.
    Need {
        section: Section,
        needed: usize,
        available: usize,
    },
    /// A token declares more bytes than this grammar can express.
    TokenTooLong {
        field: Field,
        len: usize,
        max: usize,
    },
    /// An offset plus a length does not fit a `usize`.
    ///
    /// Unreachable on any platform that can hold the slice: a token length is
    /// at most `u16::MAX` before the ceiling check and the offsets are payload
    /// offsets. It is a variant rather than a `saturating_add` because a
    /// saturating walk reports a wrong boundary as a truncation, and a reader
    /// that must fail closed should say what actually happened.
    OffsetOverflow {
        section: Section,
        at: usize,
        len: usize,
    },
    /// The tokens did not account for every remaining byte.
    TrailingBytes { consumed: usize, available: usize },
}

/// A violation and where in the payload it was found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Violation {
    /// The absolute payload offset the walk had reached.
    pub at: usize,
    pub kind: ViolationKind,
}

impl Violation {
    const fn new(at: usize, kind: ViolationKind) -> Self {
        Violation { at, kind }
    }
}

// --- Byte readers ------------------------------------------------------------
//
// Every one of these is reached only after a length check, so the slices are in
// bounds -- and they PANIC on a short slice, which is why they are crate-private
// and every public path into this crate bounds-checks before indexing.

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

// --- The walk ----------------------------------------------------------------

/// The payload header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub elapsed_wall_ms: u64,
    pub presence: u32,
    pub status_flags: u32,
}

/// Read the 16-byte header, and check NOTHING else.
///
/// See the module docs: the fold's failure-status exit happens after exactly
/// this check, so this function must not enforce the presence-word rule or the
/// scalar block's length. Both belong to the callers, after their status gates.
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

/// The five scalar words, read as bytes and interpreted as nothing.
///
/// A `u32` here is the raw word at its offset whether or not its presence bit is
/// set: the "an absent scalar must pack a zero" rule is a SEMANTIC rule and each
/// reader still owns it, which is why this type hands back the word rather than
/// an `Option`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScalarBlock {
    words: [u32; SCALAR_FIELDS.len()],
}

impl ScalarBlock {
    /// The raw word this field occupies, or `None` for a token field.
    ///
    /// `None` rather than a panic: this crate never panics at runtime.
    pub fn word(&self, field: Field) -> Option<u32> {
        match field.slot() {
            Slot::Scalar { offset } => Some(self.words[offset / 4]),
            Slot::Token { .. } => None,
        }
    }

    /// Every scalar word in wire order.
    ///
    /// Callers destructure this — `let [player, dlcs, start_date, save_version,
    /// session] = block.words();` — so a sixth scalar field is a compile error
    /// at every reader, which is the point of the whole crate.
    pub fn words(&self) -> [u32; SCALAR_FIELDS.len()] {
        self.words
    }
}

/// Read the scalar block that follows the header.
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

/// One step of the token walk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenStep<'a> {
    /// A delimited token. `bytes` is the declared run — possibly empty, and NOT
    /// validated as UTF-8: that is the caller's, because the three readers do it
    /// at three different points relative to their own rules.
    Token { field: Field, bytes: &'a [u8] },
    /// Every token has been delimited. The payload offset the walk finished at,
    /// which is what [`exact_consumption`] takes.
    End(usize),
}

/// The pull cursor over the three length-prefixed tokens.
///
/// `End` carries the finishing offset rather than a separate `finish()` because
/// a `finish()` would need a misuse state for "called before End was observed",
/// and the only honest ways to report that are a panic (unacceptable under a
/// recorder that must fail closed) or a `Violation` describing a caller defect
/// rather than a payload defect. Carrying the offset on `End` removes the state.
#[derive(Clone, Copy, Debug)]
pub struct TokenCursor {
    at: usize,
    next_token: usize,
}

impl TokenCursor {
    /// Start at the first token, immediately behind the scalar block.
    pub const fn new() -> Self {
        TokenCursor {
            at: STRING_BLOCK_OFFSET,
            next_token: 0,
        }
    }

    /// The payload offset the cursor has reached.
    pub const fn offset(&self) -> usize {
        self.at
    }

    /// Delimit the next token.
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
        // The length ceiling is checked BEFORE the bytes bound, because that is
        // the order all three readers check it in and the two produce different
        // messages for a payload that violates both.
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

/// The tokens must account for every remaining byte.
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
