//! CNationalFocusProgress state. Preserve complete signed i64 progress.

use crate::{
    Memory, ccontinuous_national_focus, ccountry, cnational_focus, cnational_focus_tree,
    cpdx_array::Array, msvc_string, read, read_slice, walk::Outcome,
};
use engine_sites::Rva;

/// CNationalFocusProgress primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x027b_7ad8);
/// Focus progress's owning country pointer; the constructor stores its RDX
/// argument here.
pub const COUNTRY: usize = 0x08;
/// Focus progress's current focus pointer, dereferenced by country focus acquisition.
pub const CURRENT: usize = 0x10;
/// CNationalFocusProgress current CContinuousNationalFocus pointer.
pub const CONTINUOUS: usize = 0x18;
/// Active/banked focus progress, signed i64 fixed5; accumulates even while idle.
pub const PROGRESS: usize = 0x38;
/// CNationalFocusProgress completed-focus pointer-array data.
pub const COMPLETED_DATA: usize = 0x40;
/// CNationalFocusProgress completed-focus signed i32 capacity.
#[cfg(test)]
pub const COMPLETED_CAPACITY: usize = 0x48;
/// CNationalFocusProgress completed-focus signed i32 count.
pub const COMPLETED_COUNT: usize = 0x4c;
/// Completed-focus pointer stride for eight-byte elements.
pub const COMPLETED_STRIDE: usize = 8;
/// CNationalFocusProgress paused byte.
pub const PAUSED: usize = 0xb0;

/// Focus daily update follows country/diplomacy joins and completes before hourly return.
pub const DAILY_UPDATE_RVA: Rva = Rva(0x002e_8bd0);

/// National-focus completion entry; no event hook is installed.
pub const COMPLETED_RVA: Rva = Rva(0x002e_7d00);
/// Exact national-focus completion entry bytes.
pub const COMPLETED_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x53, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TokenSpan {
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub tree: Option<TokenSpan>,
    pub current: Option<TokenSpan>,
    pub continuous: Option<TokenSpan>,
    /// Native signed fixed5 accumulator, retained even while no focus is selected.
    pub progress: i64,
    /// Native byte, without collapsing a nonzero value to a Rust boolean.
    pub paused: u8,
    pub completed_count: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CompletedRow {
    pub country: u32,
    /// Position in the native completed-focus vector, preserving its order.
    pub ordinal: u32,
    pub token: TokenSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusWalk {
    pub outcome: Outcome,
    pub completed_outcome: Outcome,
    pub bytes_len: usize,
    /// Observed native token bytes, including the first row omitted because the
    /// arena was full. This is a lower bound when the walk stops at capacity;
    /// grow storage off-thread without inspecting the omitted suffix here.
    pub required_bytes: usize,
}

#[derive(Clone, Copy)]
struct Token {
    address: usize,
    len: usize,
}

struct Tokens<'a> {
    bytes: crate::walk::Buffer<'a, u8>,
    len: usize,
    required: usize,
}

impl Tokens<'_> {
    fn account(&mut self, count: usize) -> Result<(), &'static str> {
        self.required = self
            .required
            .checked_add(count)
            .ok_or("focus token byte count overflow")?;
        self.bytes.ensure(self.required)?;
        Ok(())
    }

    fn fits(&self, count: usize) -> bool {
        count <= self.bytes.len() - self.len
    }

    fn copy(&mut self, memory: &impl Memory, token: Token) -> Result<TokenSpan, &'static str> {
        let span = TokenSpan {
            start: self.len,
            len: token.len,
        };
        if !read_slice(
            memory,
            token.address,
            &mut self.bytes[self.len..self.len + token.len],
        ) {
            return Err("focus token bytes unreadable");
        }
        self.len += token.len;
        Ok(span)
    }
}

// The daily focus update and completed-focus query use these exact existing
// fields, kept in one root record.
const ROOT_FIELDS: [crate::memory::RecordField; 6] = crate::memory::packed_fields([
    (0, 8),
    (CURRENT, 16),
    (PROGRESS, 8),
    (PAUSED, 1),
    (COMPLETED_COUNT, 4),
    (COMPLETED_DATA, 16),
]);
const ROOT_LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&ROOT_FIELDS, 53).unwrap();
const STRING_FIELDS: [crate::memory::RecordField; 1] = crate::memory::packed_fields([(0, 32)]);
const STRING_LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&STRING_FIELDS, 32).unwrap();
const COMPLETED_FIELDS: [crate::memory::RecordField; 1] =
    crate::memory::packed_fields([(cnational_focus::TOKEN, 32)]);
const COMPLETED_LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&COMPLETED_FIELDS, 32).unwrap();

/// Reads one authenticated country's current focus state and completed roster
/// into caller-owned storage. Country enumeration/authentication belongs to the
/// caller. Exact-field batches use advisory worker storage; a failed batch
/// permits one original serial pass, without engine calls or event hooks. Summary and completion failures remain independent after the common
/// focus-status root is authenticated; arena exhaustion never shortens a token.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    country: usize,
    country_id: u32,
    rows: &mut [Row],
    completed: &mut [CompletedRow],
    tokens: &mut [u8],
) -> FocusWalk {
    walk_buffer(
        memory,
        base,
        country,
        country_id,
        crate::walk::Buffer::Fixed(rows),
        crate::walk::Buffer::Fixed(completed),
        crate::walk::Buffer::Fixed(tokens),
    )
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    country: usize,
    country_id: u32,
    rows: &mut Vec<Row>,
    completed: &mut Vec<CompletedRow>,
    tokens: &mut Vec<u8>,
) -> FocusWalk {
    walk_buffer(
        memory,
        base,
        country,
        country_id,
        crate::walk::Buffer::Growing(rows),
        crate::walk::Buffer::Growing(completed),
        crate::walk::Buffer::Growing(tokens),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    country: usize,
    country_id: u32,
    mut rows: crate::walk::Buffer<'_, Row>,
    mut completed: crate::walk::Buffer<'_, CompletedRow>,
    tokens: crate::walk::Buffer<'_, u8>,
) -> FocusWalk {
    let mut arena = Tokens {
        bytes: tokens,
        len: 0,
        required: 0,
    };
    let root = country
        .checked_add(ccountry::FOCUS_PROGRESS)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|value| usize::try_from(value).ok())
        .filter(|&value| value != 0);
    let roots = [root.unwrap_or(0)];
    let roots = &roots[..usize::from(root.is_some())];
    let root_batch = crate::governance_batch::Records::gather(
        memory,
        crate::batch_fallback::Batch::GovernanceFocusProgress,
        crate::memory::RecordSources::Pointers(roots),
        &ROOT_LAYOUT,
    );
    let batched = root_batch.view(memory);
    let memory = &batched;
    let result = match root {
        Some(root)
            if engine_sites::at(base, VTABLE_RVA)
                .is_some_and(|expected| read::<u64>(memory, root) == Some(expected as u64)) =>
        {
            let outcome = summary(memory, country, country_id, root, &mut rows, &mut arena)
                .unwrap_or_else(|reason| Outcome::Unavailable { reason });
            let saved_len = arena.len;
            let completed_outcome =
                completed_rows(memory, country_id, root, &mut completed, &mut arena)
                    .unwrap_or_else(|reason| Outcome::Unavailable { reason });
            if matches!(completed_outcome, Outcome::Unavailable { .. }) {
                arena.len = saved_len;
            }
            (outcome, completed_outcome)
        }
        Some(_) => {
            let outcome = Outcome::Unavailable {
                reason: "focus status vtable mismatch",
            };
            (outcome, outcome)
        }
        None => {
            let outcome = Outcome::Unavailable {
                reason: "focus status pointer unreadable or null",
            };
            (outcome, outcome)
        }
    };
    FocusWalk {
        outcome: result.0,
        completed_outcome: result.1,
        bytes_len: arena.len,
        required_bytes: arena.required,
    }
}

fn summary(
    memory: &impl Memory,
    country: usize,
    country_id: u32,
    root: usize,
    output: &mut crate::walk::Buffer<'_, Row>,
    arena: &mut Tokens<'_>,
) -> Result<Outcome, &'static str> {
    let tree = read::<u64>(memory, at(country, ccountry::FOCUS_TREE)?)
        .ok_or("focus tree pointer unreadable")?;
    let mut selections = [0_u8; 16];
    let mut progress = [0_u8; 8];
    let mut paused = [0_u8; 1];
    let mut count = [0_u8; 4];
    if memory.copy_scattered(
        [
            at(root, CURRENT)?,
            at(root, PROGRESS)?,
            at(root, PAUSED)?,
            at(root, COMPLETED_COUNT)?,
        ],
        &mut [&mut selections, &mut progress, &mut paused, &mut count],
    ) != 4
    {
        return Err("focus summary fields unreadable");
    }
    let current = u64::from_le_bytes(selections[..8].try_into().unwrap());
    let continuous = u64::from_le_bytes(selections[8..].try_into().unwrap());
    let completed_count = i32::from_le_bytes(count);
    if completed_count < 0 {
        return Err("completed focus count is negative");
    }
    let mut strings = [0; 3];
    let mut string_count = 0;
    for (pointer, offset) in [
        (tree, cnational_focus_tree::TOKEN),
        (current, cnational_focus::TOKEN),
        (continuous, ccontinuous_national_focus::TOKEN),
    ] {
        if let Ok(pointer) = usize::try_from(pointer)
            && pointer != 0
            && let Some(string) = pointer.checked_add(offset)
        {
            strings[string_count] = string;
            string_count += 1;
        }
    }
    let batch = crate::governance_batch::Records::gather(
        memory,
        crate::batch_fallback::Batch::GovernanceFocusDefinitions,
        crate::memory::RecordSources::Pointers(&strings[..string_count]),
        &STRING_LAYOUT,
    );
    let batched = batch.view(memory);
    let memory = &batched;
    let definitions = [
        optional_token(memory, tree, cnational_focus_tree::TOKEN)?,
        optional_token(memory, current, cnational_focus::TOKEN)?,
        optional_token(memory, continuous, ccontinuous_national_focus::TOKEN)?,
    ];
    let required = definitions
        .iter()
        .flatten()
        .try_fold(0_usize, |sum, token| {
            sum.checked_add(token.len)
                .ok_or("focus summary token byte count overflow")
        })?;
    arena.account(required)?;
    output.ensure(1)?;
    if output.is_empty() || !arena.fits(required) {
        return Ok(Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        });
    }
    let saved_len = arena.len;
    let mut spans = [None; 3];
    for (destination, source) in spans.iter_mut().zip(definitions) {
        if let Some(source) = source {
            match arena.copy(memory, source) {
                Ok(span) => *destination = Some(span),
                Err(reason) => {
                    arena.len = saved_len;
                    return Err(reason);
                }
            }
        }
    }
    output[0] = Row {
        country: country_id,
        tree: spans[0],
        current: spans[1],
        continuous: spans[2],
        progress: i64::from_le_bytes(progress),
        paused: paused[0],
        completed_count,
    };
    Ok(Outcome::Rows { len: 1 })
}

fn completed_rows(
    memory: &impl Memory,
    country_id: u32,
    root: usize,
    output: &mut crate::walk::Buffer<'_, CompletedRow>,
    arena: &mut Tokens<'_>,
) -> Result<Outcome, &'static str> {
    let roster = Array::read(memory, at(root, COMPLETED_DATA)?, COMPLETED_STRIDE)
        .ok_or("completed focus roster unreadable or invalid")?;
    output.ensure(roster.count as usize)?;
    let admitted = output.len().min(roster.count as usize);
    let pointer_batch = crate::governance_batch::Records::gather(
        memory,
        crate::batch_fallback::Batch::GovernanceFocusPointers,
        crate::memory::RecordSources::Strided {
            base: roster.data,
            stride: COMPLETED_STRIDE,
            count: admitted,
        },
        &crate::governance_batch::POINTER_LAYOUT,
    );
    let targets = pointer_batch.targets(0);
    let definition_batch = crate::governance_batch::Records::gather(
        memory,
        crate::batch_fallback::Batch::GovernanceFocusDefinitions,
        crate::memory::RecordSources::Pointers(&targets),
        &COMPLETED_LAYOUT,
    );
    let pointers = pointer_batch.view(memory);
    let definitions = definition_batch.view(&pointers);
    let memory = &definitions;
    let mut len = 0;
    let mut unread_rows = 0;
    for ordinal in 0..roster.count {
        if len == output.len() {
            unread_rows = roster.count - ordinal;
            break;
        }
        let pointer = read::<u64>(memory, roster.element(ordinal, COMPLETED_STRIDE).unwrap())
            .ok_or("completed focus pointer unreadable")?;
        let token = optional_token(memory, pointer, cnational_focus::TOKEN)?
            .ok_or("completed focus pointer is null")?;
        arena.account(token.len)?;
        if !arena.fits(token.len) {
            unread_rows = roster.count - ordinal;
            break;
        }
        output[len] = CompletedRow {
            country: country_id,
            ordinal,
            token: arena.copy(memory, token)?,
        };
        len += 1;
    }
    Ok(if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    })
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer.checked_add(offset).ok_or("focus address overflow")
}

fn optional_token(
    memory: &impl Memory,
    pointer: u64,
    offset: usize,
) -> Result<Option<Token>, &'static str> {
    if pointer == 0 {
        return Ok(None);
    }
    let pointer = usize::try_from(pointer).map_err(|_| "focus pointer overflow")?;
    let string = at(pointer, offset)?;
    let header = read::<[u8; 32]>(memory, string).ok_or("focus token header unreadable")?;
    let length = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if length > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("focus token header invalid");
    }
    let len = usize::try_from(length).map_err(|_| "focus token length overflow")?;
    let address = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .map_err(|_| "focus token pointer overflow")?
    };
    if address == 0 || read::<u8>(memory, at(address, len)?) != Some(0) {
        return Err("focus token terminator unreadable or invalid");
    }
    Ok(Some(Token { address, len }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const BASE: usize = 0x1_4000_0000;
    const COUNTRY: usize = 0x1000;
    const ROOT: usize = 0x3000;

    struct Fixture(Vec<u8>);

    impl Fixture {
        fn new() -> Self {
            let mut fixture = Self(vec![0; 0x5000]);
            fixture.u64(COUNTRY + ccountry::FOCUS_PROGRESS, ROOT as u64);
            fixture.u64(COUNTRY + ccountry::FOCUS_TREE, 0x4000);
            fixture.u64(ROOT, (BASE + VTABLE_RVA.0) as u64);
            fixture.u64(ROOT + CURRENT, 0x4100);
            fixture.u64(ROOT + CONTINUOUS, 0x4200);
            fixture.u64(ROOT + PROGRESS, (i64::MAX - 17) as u64);
            fixture.0[ROOT + PAUSED - COUNTRY] = 3;
            fixture.u64(ROOT + COMPLETED_DATA, 0x3800);
            fixture.u32(ROOT + COMPLETED_CAPACITY, 2);
            fixture.u32(ROOT + COMPLETED_COUNT, 2);
            fixture.u64(0x3800, 0x4300);
            fixture.u64(0x3808, 0x4400);
            fixture.text(0x4000 + cnational_focus_tree::TOKEN, b"tree");
            fixture.text(0x4100 + cnational_focus::TOKEN, b"active");
            fixture.text(0x4200 + ccontinuous_national_focus::TOKEN, b"continuous");
            fixture.text(0x4300 + cnational_focus::TOKEN, b"completed_a");
            fixture.text(0x4400 + cnational_focus::TOKEN, b"completed_b");
            fixture
        }

        fn u64(&mut self, address: usize, value: u64) {
            let at = address - COUNTRY;
            self.0[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }

        fn u32(&mut self, address: usize, value: u32) {
            let at = address - COUNTRY;
            self.0[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }

        fn text(&mut self, address: usize, value: &[u8]) {
            assert!(value.len() <= 15);
            let at = address - COUNTRY;
            self.0[at..at + 16].fill(0);
            self.0[at..at + value.len()].copy_from_slice(value);
            self.u64(address + msvc_string::LENGTH, value.len() as u64);
            self.u64(address + msvc_string::CAPACITY, 15);
        }

        fn memory(&self) -> SliceMemory<'_> {
            SliceMemory {
                base: COUNTRY,
                bytes: &self.0,
            }
        }
    }

    #[test]
    fn worker_grows_completed_focuses_and_tokens_from_original_mod_sized_rosters() {
        let mut fixture = Fixture::new();
        let mut rows = Vec::new();
        let mut completed = Vec::new();
        let mut tokens = Vec::new();
        let first = walk_growing(
            &fixture.memory(),
            BASE,
            COUNTRY,
            37,
            &mut rows,
            &mut completed,
            &mut tokens,
        );
        assert_eq!(first.outcome, Outcome::Rows { len: 1 });
        assert_eq!(first.completed_outcome, Outcome::Rows { len: 2 });
        assert_eq!((first.bytes_len, first.required_bytes), (42, 42));
        let data = 0x8000;
        fixture.0.resize(data + 5_000 * 8 - COUNTRY, 0);
        fixture.u64(ROOT + COMPLETED_DATA, data as u64);
        fixture.u32(ROOT + COMPLETED_CAPACITY, 5_000);
        fixture.u32(ROOT + COMPLETED_COUNT, 5_000);
        for ordinal in 0..5_000 {
            fixture.u64(data + ordinal * 8, [0x4300, 0x4400][ordinal % 2]);
        }
        let fixed = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            37,
            &mut rows[..1],
            &mut completed[..2],
            &mut tokens[..42],
        );
        assert_eq!(
            fixed.completed_outcome,
            Outcome::Truncated {
                len: 2,
                unread_rows: 4_998
            }
        );
        let later = walk_growing(
            &fixture.memory(),
            BASE,
            COUNTRY,
            37,
            &mut rows,
            &mut completed,
            &mut tokens,
        );
        assert_eq!(later.outcome, Outcome::Rows { len: 1 });
        assert_eq!(later.completed_outcome, Outcome::Rows { len: 5_000 });
        assert_eq!(rows[0].completed_count, 5_000);
        assert_eq!((later.bytes_len, later.required_bytes), (55_020, 55_020));
        for (ordinal, row) in completed[..5_000].iter().enumerate() {
            assert_eq!((row.country, row.ordinal), (37, ordinal as u32));
            assert_eq!(
                &tokens[row.token.start..row.token.start + row.token.len],
                [b"completed_a", b"completed_b"][ordinal % 2],
            );
        }
    }

    #[test]
    fn native_summary_and_completed_order_preserve_exact_values() {
        let fixture = Fixture::new();
        let mut rows = [Row::default(); 1];
        let mut completed = [CompletedRow::default(); 2];
        let mut tokens = [0; 64];
        let result = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            37,
            &mut rows,
            &mut completed,
            &mut tokens,
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(result.completed_outcome, Outcome::Rows { len: 2 });
        assert_eq!(rows[0].country, 37);
        assert_eq!(rows[0].progress, i64::MAX - 17);
        assert_eq!(rows[0].paused, 3);
        assert_eq!(rows[0].completed_count, 2);
        assert!(rows[0].current.is_some() && rows[0].continuous.is_some());
        let span = rows[0].tree.unwrap();
        assert_eq!(&tokens[span.start..span.start + span.len], b"tree");
        for (ordinal, row) in completed.iter().enumerate() {
            assert_eq!(row.country, 37);
            assert_eq!(row.ordinal, ordinal as u32);
            let expected: &[u8] = if ordinal == 0 {
                b"completed_a"
            } else {
                b"completed_b"
            };
            assert_eq!(
                &tokens[row.token.start..row.token.start + row.token.len],
                expected
            );
        }
        assert_eq!(result.required_bytes, 42);
        assert_eq!(result.bytes_len, 42);
    }

    #[test]
    fn idle_focus_preserves_signed_accumulator_and_proves_empty_roster() {
        let mut fixture = Fixture::new();
        fixture.u64(COUNTRY + ccountry::FOCUS_TREE, 0);
        fixture.u64(ROOT + CURRENT, 0);
        fixture.u64(ROOT + CONTINUOUS, 0);
        fixture.u64(ROOT + PROGRESS, (-123_456_789_i64) as u64);
        fixture.u64(ROOT + COMPLETED_DATA, 0);
        fixture.u32(ROOT + COMPLETED_COUNT, 0);
        let mut rows = [Row::default(); 1];
        let result = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            1,
            &mut rows,
            &mut [],
            &mut [],
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(result.completed_outcome, Outcome::Rows { len: 0 });
        assert_eq!(rows[0].progress, -123_456_789);
        assert_eq!(rows[0].tree, None);
        assert_eq!(rows[0].current, None);
        assert_eq!(rows[0].continuous, None);
        assert_eq!(result.bytes_len, 0);
    }

    #[test]
    fn row_and_arena_capacity_stop_without_inspecting_the_omitted_suffix() {
        let mut fixture = Fixture::new();
        fixture.u64(0x3808, u64::MAX);
        let mut rows = [Row::default(); 1];
        let mut completed = [CompletedRow::default(); 1];
        let mut tokens = [0xaa; 41];
        let result = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            1,
            &mut rows,
            &mut completed,
            &mut tokens,
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            result.completed_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(result.required_bytes, 31);
        assert_eq!(result.bytes_len, 31);
        assert!(tokens[31..].iter().all(|&byte| byte == 0xaa));

        let mut tiny = [0xaa; 10];
        let result = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            1,
            &mut rows,
            &mut completed,
            &mut tiny,
        );
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(
            result.completed_outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        assert_eq!(result.required_bytes, 31);
        assert_eq!(result.bytes_len, 0);
        assert_eq!(tiny, [0xaa; 10]);
    }

    #[test]
    fn summary_and_completion_read_failures_remain_independent() {
        let mut fixture = Fixture::new();
        fixture.u64(ROOT + CURRENT, u64::MAX);
        let mut rows = [Row::default(); 1];
        let mut completed = [CompletedRow::default(); 2];
        let mut tokens = [0; 64];
        let result = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            1,
            &mut rows,
            &mut completed,
            &mut tokens,
        );
        assert!(matches!(result.outcome, Outcome::Unavailable { .. }));
        assert_eq!(result.completed_outcome, Outcome::Rows { len: 2 });

        let mut fixture = Fixture::new();
        fixture.u64(0x3808, 0);
        let result = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            1,
            &mut rows,
            &mut completed,
            &mut tokens,
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            result.completed_outcome,
            Outcome::Unavailable {
                reason: "completed focus pointer is null"
            }
        );
        assert_eq!(result.bytes_len, 20);
    }

    #[test]
    fn token_headers_and_root_identity_fail_closed_without_lossy_conversion() {
        let mut fixture = Fixture::new();
        fixture.text(0x4100 + cnational_focus::TOKEN, &[0xff, 0, b'A']);
        let mut rows = [Row::default(); 1];
        let mut completed = [CompletedRow::default(); 2];
        let mut tokens = [0; 64];
        let result = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            1,
            &mut rows,
            &mut completed,
            &mut tokens,
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        let span = rows[0].current.unwrap();
        assert_eq!(&tokens[span.start..span.start + span.len], &[0xff, 0, b'A']);

        fixture.u64(0x4100 + cnational_focus::TOKEN + msvc_string::LENGTH, 16);
        let result = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            1,
            &mut rows,
            &mut completed,
            &mut tokens,
        );
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "focus token header invalid"
            }
        );
        fixture.u64(ROOT, 0);
        let result = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            1,
            &mut rows,
            &mut completed,
            &mut tokens,
        );
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "focus status vtable mismatch"
            }
        );
        assert_eq!(result.completed_outcome, result.outcome);
    }

    #[test]
    fn heap_token_is_exact_and_terminator_fault_does_not_become_absence() {
        let mut fixture = Fixture::new();
        let token = b"long_national_focus_name_01";
        let string = 0x4100 + cnational_focus::TOKEN;
        fixture.u64(string, 0x4800);
        fixture.u64(string + msvc_string::LENGTH, token.len() as u64);
        fixture.u64(string + msvc_string::CAPACITY, 31);
        fixture.0[0x4800 - COUNTRY..0x4800 - COUNTRY + token.len()].copy_from_slice(token);
        let mut rows = [Row::default(); 1];
        let mut completed = [CompletedRow::default(); 2];
        let mut tokens = [0; 96];
        let result = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            1,
            &mut rows,
            &mut completed,
            &mut tokens,
        );
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        let span = rows[0].current.unwrap();
        assert_eq!(&tokens[span.start..span.start + span.len], token);
        fixture.0[0x4800 - COUNTRY + token.len()] = b'x';
        let result = walk(
            &fixture.memory(),
            BASE,
            COUNTRY,
            1,
            &mut rows,
            &mut completed,
            &mut tokens,
        );
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "focus token terminator unreadable or invalid"
            }
        );
        assert_eq!(result.completed_outcome, Outcome::Rows { len: 2 });
    }
}

#[cfg(test)]
#[path = "cnational_focus_progress/serial.rs"]
mod serial;

#[cfg(test)]
#[path = "cnational_focus_progress/batch_tests.rs"]
mod batch_tests;
