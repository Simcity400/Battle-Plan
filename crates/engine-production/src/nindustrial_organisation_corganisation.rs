//! Military-industrial organisations held by each country's production status
//! in content-template order. Instances are created at game start or loading,
//! and production lines resolve their global full handles through the registry.
//! These offsets require a resolved organisation object.

use crate::{
    Memory, ccountry, cequipment_variant,
    cequipment_variant::TextSpan,
    cproduction_status,
    msvc_string::Descriptor,
    read_slice,
    walk::{Buffer, Outcome},
};
use engine_sites::Rva;

/// Organisation primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029e_4568);
/// Organisation reference-type discriminator.
pub const REFERENCE_TYPE_ID: u32 = 79;
/// Organisation full self-reference, two u32 words.
pub const REFERENCE: usize = 0x08;
/// Organisation u32 script-key token copied from its template and used to serialize its block.
pub const TOKEN: usize = 0x38;
/// Stored name-key override copied from the template and mutable through
/// script effects. Empty selects the script key; an available owner-tag-prefixed
/// localization key takes precedence. The stored string is serialized even empty.
pub const NAME: usize = 0x60;
/// Stored icon-sprite override copied from the template and mutable through
/// script effects. A nonempty override precedes tag-prefixed and plain script-key
/// sprite candidates, then the generic fallback.
pub const ICON: usize = 0x80;
/// Organisation production-line entry array.
pub const PRODUCTION_LINES: usize = 0xd8;
/// History robin-hood entries keyed by variant handle; serialized in slot order.
pub const HISTORY: usize = 0xf8;
/// History signed i32 element count; zero omits history serialization.
pub const HISTORY_COUNT: usize = 0x100;
/// History-table signed i32 bucket mask, used with the overflow extent.
pub const HISTORY_LAST: usize = 0x104;
/// History-table u8 overflow extent, set to the bucket exponent plus two.
pub const HISTORY_OVERFLOW: usize = 0x108;
/// Owner country link copied at construction and used for name/icon tag prefixes.
pub const OWNER: usize = 0x120;

pub mod details;

/// History slots copied per guarded read.
const HISTORY_BATCH: usize = 32;

/// One organisation, in its owner's array order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Full native CID: type 79 in the low word, id in the high word.
    pub reference: u64,
    /// Raw id of the country whose production status holds it.
    pub country: u32,
    /// Position in that array; the save writes this order.
    pub ordinal: u32,
    /// [`OWNER`]: the tag link the name and icon rules prefix.
    pub owner: u32,
    /// [`TOKEN`]: the script key.
    pub token: u32,
    /// [`NAME`] in the caller's arena; empty means the script key.
    pub name: TextSpan,
    /// [`ICON`] in the caller's arena; empty selects the script-key sprite fallback.
    pub icon: TextSpan,
}

/// One occupied history slot: the units the organisation's lines completed
/// of one variant.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HistoryRow {
    pub organisation: u64,
    /// The slot's key, an authenticated-type variant CID (type 70).
    pub variant: u64,
    /// `mio_history_entry::PRODUCTION_COUNT`.
    pub units: i32,
    /// An occupied slot may have production history without ever authoring a design.
    pub design_hour: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcomes {
    pub organisations: Outcome,
    pub history: Outcome,
    /// Arena bytes the organisation rows use.
    pub bytes_len: usize,
    /// Exact when complete; when truncated, at least the bytes of the first
    /// organisation that did not fit.
    pub required_bytes: usize,
}

impl Outcomes {
    fn unavailable(reason: &'static str) -> Self {
        Self {
            organisations: Outcome::Unavailable { reason },
            history: Outcome::Unavailable { reason },
            bytes_len: 0,
            required_bytes: 0,
        }
    }
}

/// Acquisition workers continue from the original organisation fields and
/// history headers when native storage grows after the last sizing poll.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    organisations: &mut Vec<Row>,
    history: &mut Vec<HistoryRow>,
    bytes: &mut Vec<u8>,
) -> Outcomes {
    walk_with(
        memory,
        base,
        game_state,
        organisations,
        history,
        bytes,
        &mut (),
    )
}

/// Additional operands read only while the existing census holds authenticated
/// objects. A visitor keeps its own independent outcome; it cannot add a census.
pub trait Visitor {
    fn country(&mut self, _country: &ccountry::Country, _status: usize) {}
    fn organisation(&mut self, _pointer: usize, _row: &Row) {}
}
impl Visitor for () {}

#[allow(clippy::too_many_arguments)]
pub fn walk_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    organisations: &mut Vec<Row>,
    history: &mut Vec<HistoryRow>,
    bytes: &mut Vec<u8>,
    visitor: &mut impl Visitor,
) -> Outcomes {
    walk_inner(
        memory,
        base,
        game_state,
        Buffer::Growing(organisations),
        Buffer::Growing(history),
        Buffer::Growing(bytes),
        visitor,
    )
    .unwrap_or_else(Outcomes::unavailable)
}

/// New-game roster includes eligible content templates; full strings and live history counts size storage.
pub fn demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<(usize, usize, usize), &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let vtable =
        engine_sites::at(base, VTABLE_RVA).ok_or("organisation module address overflow")? as u64;
    let (mut rows, mut histories, mut bytes) = (0usize, 0usize, 0usize);
    let mut found = false;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        found = true;
        let status = cproduction_status::manager(memory, base, country)?;
        let array = status
            .checked_add(cproduction_status::ORGANISATIONS)
            .and_then(|address| crate::cpdx_array::Array::read(memory, address, 8))
            .ok_or("organisation array unreadable or invalid")?;
        crate::cpdx_array::pointers(
            memory,
            array,
            "organisation pointers unreadable",
            |_, pointer| {
                if pointer == 0 {
                    return Err("organisation pointer null");
                }
                let fields = read_fields(memory, vtable, pointer)?;
                rows = rows.checked_add(1).ok_or("organisation demand overflow")?;
                histories = histories
                    .checked_add(fields.history_count as usize)
                    .ok_or("organisation history demand overflow")?;
                bytes = bytes
                    .checked_add(fields.name.len)
                    .and_then(|len| len.checked_add(fields.icon.len))
                    .ok_or("organisation byte demand overflow")?;
                Ok(())
            },
        )?;
    }
    if !found {
        return Err("country table has no readable countries");
    }
    Ok((rows, histories, bytes))
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut organisations: Buffer<'_, Row>,
    mut history: Buffer<'_, HistoryRow>,
    mut bytes: Buffer<'_, u8>,
    visitor: &mut impl Visitor,
) -> Result<Outcomes, &'static str> {
    let vtable =
        engine_sites::at(base, VTABLE_RVA).ok_or("organisation module address overflow")? as u64;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut len = 0;
    let mut unread = 0_u32;
    let mut history_len = 0usize;
    let mut history_unread = 0_u32;
    let mut history_failure = None;
    let mut found_country = false;
    let mut bytes_len = 0usize;
    let mut required_bytes = 0;
    // Set once a row or its strings do not fit: later organisations are
    // counted from their array headers only.
    let mut full = false;
    let mut pointers = [0_u8; 32 * 8];
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        found_country = true;
        let status = cproduction_status::manager(memory, base, country)?;
        visitor.country(&country, status);
        let array = status
            .checked_add(cproduction_status::ORGANISATIONS)
            .and_then(|address| crate::cpdx_array::Array::read(memory, address, 8))
            .ok_or("organisation array unreadable or invalid")?;
        'batches: for first in (0..array.count).step_by(32) {
            let count = ((array.count - first) as usize).min(32);
            if full || !organisations.ensure(len + 1)? {
                full = true;
                unread = unread
                    .checked_add(array.count - first)
                    .ok_or("organisation unread count overflow")?;
                break;
            }
            let address = array
                .element(first, 8)
                .ok_or("organisation array address overflow")?;
            if !read_slice(memory, address, &mut pointers[..count * 8]) {
                return Err("organisation pointers unreadable");
            }
            for (index, pointer) in pointers[..count * 8].chunks_exact(8).enumerate() {
                let ordinal = first + index as u32;
                if !organisations.ensure(len + 1)? {
                    unread = unread
                        .checked_add(array.count - ordinal)
                        .ok_or("organisation unread count overflow")?;
                    break 'batches;
                }
                let pointer = usize::try_from(u64::from_le_bytes(pointer.try_into().unwrap()))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or("organisation pointer null")?;
                let fields = read_fields(memory, vtable, pointer)?;
                let end = bytes_len
                    .checked_add(fields.name.len)
                    .and_then(|len| len.checked_add(fields.icon.len))
                    .ok_or("organisation byte arena overflow")?;
                if !bytes.is_growing() && end > bytes.len() {
                    full = true;
                    required_bytes = end;
                    unread = unread
                        .checked_add(array.count - ordinal)
                        .ok_or("organisation unread count overflow")?;
                    break 'batches;
                }
                let mut span = |text: &Descriptor| -> Result<TextSpan, &'static str> {
                    let start = bytes_len;
                    text.copy_buffer(memory, &mut bytes, start)?;
                    bytes_len += text.len;
                    Ok(TextSpan {
                        start,
                        len: text.len,
                    })
                };
                organisations[len] = Row {
                    country: country.id,
                    ordinal,
                    name: span(&fields.name)
                        .map_err(|_| "organisation name unreadable or invalid")?,
                    icon: span(&fields.icon)
                        .map_err(|_| "organisation icon unreadable or invalid")?,
                    ..fields.row
                };
                visitor.organisation(pointer, &organisations[len]);
                len += 1;
                if history_failure.is_some() || fields.history_count == 0 {
                    continue;
                }
                let history_end = history_len
                    .checked_add(fields.history_count as usize)
                    .ok_or("organisation history arena overflow")?;
                history.ensure(history_end)?;
                if history_unread != 0 || history_len == history.len() {
                    history_unread = history_unread
                        .checked_add(fields.history_count)
                        .ok_or("organisation history unread count overflow")?;
                    continue;
                }
                match read_history(memory, &fields, &mut history[history_len..]) {
                    Ok((read, unread)) => {
                        history_len += read;
                        history_unread = unread;
                    }
                    Err(reason) => history_failure = Some(reason),
                }
            }
        }
    }
    if !found_country {
        return Err("country table has no readable countries");
    }
    let outcome = |len, unread_rows| {
        if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        }
    };
    // Organisations left unread for capacity have unread histories of unknown
    // size, so the history is never complete while they are.
    if unread != 0 {
        history_unread = history_unread.max(1);
    }
    Ok(Outcomes {
        organisations: outcome(len, unread),
        history: match history_failure {
            Some(reason) => Outcome::Unavailable { reason },
            None => outcome(history_len, history_unread),
        },
        bytes_len,
        required_bytes: required_bytes.max(bytes_len),
    })
}

/// One organisation's fields and history header, from a single scatter.
struct Fields {
    row: Row,
    name: Descriptor,
    icon: Descriptor,
    history: usize,
    history_count: u32,
    history_slots: usize,
}

fn read_fields(memory: &impl Memory, vtable: u64, pointer: usize) -> Result<Fields, &'static str> {
    let at = |offset| pointer.checked_add(offset).unwrap_or(0);
    let mut identity = [0; 16];
    let mut token = [0; 4];
    let mut name = [0; 32];
    let mut icon = [0; 32];
    let mut table = [0; HISTORY_OVERFLOW + 1 - HISTORY];
    let mut owner = [0; 4];
    let copied = memory.copy_scattered(
        [
            pointer,
            at(TOKEN),
            at(NAME),
            at(ICON),
            at(HISTORY),
            at(OWNER),
        ],
        &mut [
            &mut identity,
            &mut token,
            &mut name,
            &mut icon,
            &mut table,
            &mut owner,
        ],
    );
    if copied == 0 {
        return Err("organisation identity unreadable");
    }
    let reference = u64::from_le_bytes(identity[REFERENCE..].try_into().unwrap());
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != vtable
        || reference as u32 != REFERENCE_TYPE_ID
        || reference >> 32 == 0
    {
        return Err("organisation vtable or reference mismatch");
    }
    if copied < 6 {
        return Err("organisation fields unreadable");
    }
    let text = |header: [u8; 32], offset| Descriptor::from_header(memory, at(offset), header);
    let name = text(name, NAME).map_err(|_| "organisation name unreadable or invalid")?;
    let icon = text(icon, ICON).map_err(|_| "organisation icon unreadable or invalid")?;
    let word =
        |offset: usize| u32::from_le_bytes(table[offset - HISTORY..][..4].try_into().unwrap());
    let overflow = table[HISTORY_OVERFLOW - HISTORY];
    // Bucket exponents 1..=30 select mask and overflow geometry; invalid geometry refuses only history.
    let history_slots = word(HISTORY_LAST)
        .checked_add(1)
        .filter(|buckets| {
            buckets.is_power_of_two()
                && (1..=30).contains(&buckets.trailing_zeros())
                && u32::from(overflow) == buckets.trailing_zeros() + 2
        })
        .map(|buckets| buckets as usize + usize::from(overflow))
        .unwrap_or(0);
    Ok(Fields {
        row: Row {
            reference,
            token: u32::from_le_bytes(token),
            owner: u32::from_le_bytes(owner),
            ..Row::default()
        },
        name,
        icon,
        history: usize::try_from(u64::from_le_bytes(table[..8].try_into().unwrap()))
            .map_err(|_| "organisation history pointer overflow")?,
        history_count: word(HISTORY_COUNT),
        // Slot extent follows the live mask and overflow count, without a fixed slot limit.
        history_slots,
    })
}

/// Reads the occupied slots of one history table in slot order, as Serialize
/// visits them: a zero distance byte is empty; the -1 sentinel may only end
/// the table. The occupied slots must number the element count. Returns the
/// rows written and the rows left unread for capacity.
fn read_history(
    memory: &impl Memory,
    fields: &Fields,
    output: &mut [HistoryRow],
) -> Result<(usize, u32), &'static str> {
    use crate::mio_history_entry::{
        ACTIVE, DESIGN_HOUR, DISTANCE, ITEM_REFERENCE, PRODUCTION_COUNT, STRIDE,
    };
    if fields.history == 0 {
        return Err("organisation history table invalid");
    }
    if fields.history_slots == 0 {
        return Err("organisation history table geometry invalid");
    }
    fields
        .history_slots
        .checked_mul(STRIDE)
        .and_then(|bytes| fields.history.checked_add(bytes))
        .ok_or("organisation history address overflow")?;
    let mut bytes = [0_u8; HISTORY_BATCH * STRIDE];
    let mut len = 0;
    let mut occupied = 0_u32;
    for first in (0..fields.history_slots).step_by(HISTORY_BATCH) {
        let count = (fields.history_slots - first).min(HISTORY_BATCH);
        let address = fields.history + first * STRIDE;
        if !read_slice(memory, address, &mut bytes[..count * STRIDE]) {
            return Err("organisation history unreadable");
        }
        for entry in bytes[..count * STRIDE].chunks_exact(STRIDE) {
            match entry[DISTANCE] as i8 {
                0 => continue,
                -1 => return Err("organisation history sentinel inside its table"),
                _ => {}
            }
            let variant = u64::from_le_bytes(entry[ITEM_REFERENCE..][..8].try_into().unwrap());
            if variant as u32 != cequipment_variant::REFERENCE_TYPE {
                return Err("organisation history key is not a variant");
            }
            occupied += 1;
            if occupied > fields.history_count {
                return Err("organisation history count mismatch");
            }
            if len < output.len() {
                output[len] = HistoryRow {
                    organisation: fields.row.reference,
                    variant,
                    units: i32::from_le_bytes(entry[PRODUCTION_COUNT..][..4].try_into().unwrap()),
                    design_hour: (entry[ACTIVE] != 0)
                        .then(|| i32::from_le_bytes(entry[DESIGN_HOUR..][..4].try_into().unwrap())),
                };
                len += 1;
            }
        }
    }
    if occupied != fields.history_count {
        return Err("organisation history count mismatch");
    }
    Ok((len, occupied - len as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cgame_state, memory::SliceMemory, mio_history_entry as entry};

    const BASE: usize = 0x1000;
    const COUNTRY: usize = 0x3000;
    const STATUS: usize = 0x5000;
    const ARRAY: usize = 0x6000;
    const FIRST: usize = 0x6100;
    const SECOND: usize = 0x6300;
    const TABLE: usize = 0x6800;
    /// One past the fixture's readable memory.
    const END: usize = BASE + 0x8000;
    const CID_FIRST: u64 = (25 << 32) | REFERENCE_TYPE_ID as u64;
    const CID_SECOND: u64 = (27 << 32) | REFERENCE_TYPE_ID as u64;
    const VARIANT_A: u64 = (1354 << 32) | cequipment_variant::REFERENCE_TYPE as u64;
    const VARIANT_B: u64 = (7 << 32) | cequipment_variant::REFERENCE_TYPE as u64;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
        put(bytes, header, &(data as u64).to_le_bytes());
        put(bytes, header + 8, &count.to_le_bytes());
        put(bytes, header + 12, &count.to_le_bytes());
    }

    fn string(bytes: &mut [u8], header: usize, text: &[u8]) {
        put(bytes, header, text);
        put(bytes, header + 16, &(text.len() as u64).to_le_bytes());
        put(bytes, header + 24, &15_u64.to_le_bytes());
    }

    /// One country (slot 2 aliases slot 1) holding two organisations; the
    /// first has eight history buckets and a five-slot overflow tail, with
    /// occupied slots 0 and 2.
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x8000];
        array(&mut bytes, BASE + cgame_state::COUNTRIES, 0x2000, 3);
        array(&mut bytes, BASE + cgame_state::COUNTRY_LINKS, 0x2100, 3);
        for slot in [1, 2] {
            put(
                &mut bytes,
                0x2000 + slot * 8,
                &(COUNTRY as u64).to_le_bytes(),
            );
            put(&mut bytes, 0x2100 + slot * 4, &1_u32.to_le_bytes());
        }
        for (address, value) in [
            (COUNTRY, ccountry::VTABLE_RVA.0 as u64),
            (COUNTRY + ccountry::PRODUCTION_STATUS, STATUS as u64),
            (STATUS, cproduction_status::VTABLE_RVA.0 as u64),
            (STATUS + cproduction_status::COUNTRY, COUNTRY as u64),
            (ARRAY, FIRST as u64),
            (ARRAY + 8, SECOND as u64),
        ] {
            put(&mut bytes, address, &value.to_le_bytes());
        }
        put(&mut bytes, COUNTRY + ccountry::ID, &2_u32.to_le_bytes());
        array(
            &mut bytes,
            STATUS + cproduction_status::ORGANISATIONS,
            ARRAY,
            2,
        );
        for (pointer, cid, token, owner) in [
            (FIRST, CID_FIRST, 0x8000_1234_u32, 2_u32),
            (SECOND, CID_SECOND, 0x8000_1235, 9),
        ] {
            put(&mut bytes, pointer, &(VTABLE_RVA.0 as u64).to_le_bytes());
            put(&mut bytes, pointer + REFERENCE, &cid.to_le_bytes());
            put(&mut bytes, pointer + TOKEN, &token.to_le_bytes());
            put(&mut bytes, pointer + OWNER, &owner.to_le_bytes());
            string(&mut bytes, pointer + NAME, b"");
            string(&mut bytes, pointer + ICON, b"");
        }
        string(&mut bytes, FIRST + ICON, b"GFX_idea_x");
        string(&mut bytes, SECOND + NAME, b"renamed_key");
        put(&mut bytes, FIRST + HISTORY, &(TABLE as u64).to_le_bytes());
        put(&mut bytes, FIRST + HISTORY_COUNT, &2_u32.to_le_bytes());
        put(&mut bytes, FIRST + HISTORY_LAST, &7_i32.to_le_bytes());
        put(&mut bytes, FIRST + HISTORY_OVERFLOW, &[5]);
        for (slot, variant, units) in [(0, VARIANT_A, 2116_i32), (2, VARIANT_B, -3)] {
            let at = TABLE + slot * entry::STRIDE;
            put(&mut bytes, at + entry::DISTANCE, &[1]);
            put(
                &mut bytes,
                at + entry::ITEM_REFERENCE,
                &variant.to_le_bytes(),
            );
            put(
                &mut bytes,
                at + entry::PRODUCTION_COUNT,
                &units.to_le_bytes(),
            );
        }
        // The end sentinel after the last slot is never read.
        put(
            &mut bytes,
            TABLE + 13 * entry::STRIDE + entry::DISTANCE,
            &[0xff],
        );
        bytes
    }

    fn run(
        bytes: &[u8],
        organisations: usize,
        history: usize,
    ) -> (Outcomes, Vec<Row>, Vec<HistoryRow>, Vec<u8>) {
        let mut rows = vec![Row::default(); organisations];
        let mut entries = vec![HistoryRow::default(); history];
        let mut arena = vec![0; 64];
        let outcomes = walk(
            &SliceMemory { base: BASE, bytes },
            0,
            BASE,
            &mut rows,
            &mut entries,
            &mut arena,
        );
        (outcomes, rows, entries, arena)
    }

    #[test]
    fn organisations_keep_array_order_raw_keys_and_occupied_history_slots() {
        let (outcomes, rows, entries, arena) = run(&fixture(), 4, 4);
        assert_eq!(outcomes.organisations, Outcome::Rows { len: 2 });
        assert_eq!(outcomes.history, Outcome::Rows { len: 2 });
        assert_eq!((outcomes.bytes_len, outcomes.required_bytes), (21, 21));
        let text = |span: TextSpan| &arena[span.start..span.start + span.len];
        assert_eq!(
            (
                rows[0].reference,
                rows[0].country,
                rows[0].ordinal,
                rows[0].owner,
                rows[0].token
            ),
            (CID_FIRST, 2, 0, 2, 0x8000_1234)
        );
        assert_eq!(
            (text(rows[0].name), text(rows[0].icon)),
            (&b""[..], &b"GFX_idea_x"[..])
        );
        assert_eq!(
            (rows[1].reference, rows[1].ordinal, rows[1].owner),
            (CID_SECOND, 1, 9)
        );
        assert_eq!(text(rows[1].name), b"renamed_key");
        assert_eq!(
            entries[..2],
            [
                HistoryRow {
                    organisation: CID_FIRST,
                    variant: VARIANT_A,
                    units: 2116,
                    design_hour: None,
                },
                HistoryRow {
                    organisation: CID_FIRST,
                    variant: VARIANT_B,
                    units: -3,
                    design_hour: None,
                }
            ]
        );
    }

    #[test]
    fn history_keeps_design_dates_separately_from_production_counts() {
        let mut bytes = fixture();
        put(&mut bytes, TABLE + entry::ACTIVE, &[1]);
        put(
            &mut bytes,
            TABLE + entry::DESIGN_HOUR,
            &(-123_i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            TABLE + 2 * entry::STRIDE + entry::DESIGN_HOUR,
            &999_i32.to_le_bytes(),
        );
        let (outcome, _, rows, _) = run(&bytes, 4, 4);
        assert_eq!(outcome.history, Outcome::Rows { len: 2 });
        assert_eq!(rows[0].design_hour, Some(-123));
        assert_eq!(rows[1].design_hour, None);
        assert_eq!(rows[0].units, 2116);
    }

    #[test]
    fn history_extent_comes_from_the_live_header_past_65536_slots() {
        let buckets = 1_usize << 16;
        let overflow = buckets.trailing_zeros() as u8 + 2;
        let slots = buckets + usize::from(overflow);
        let mut bytes = fixture();
        bytes.resize(TABLE - BASE + (slots + 1) * entry::STRIDE, 0);
        put(
            &mut bytes,
            FIRST + HISTORY_LAST,
            &(buckets as i32 - 1).to_le_bytes(),
        );
        put(&mut bytes, FIRST + HISTORY_OVERFLOW, &[overflow]);
        put(&mut bytes, FIRST + HISTORY_COUNT, &3_u32.to_le_bytes());
        put(
            &mut bytes,
            TABLE + 13 * entry::STRIDE + entry::DISTANCE,
            &[0],
        );
        let last = TABLE + (buckets + 1) * entry::STRIDE;
        let variant = (8_u64 << 32) | cequipment_variant::REFERENCE_TYPE as u64;
        put(&mut bytes, last + entry::DISTANCE, &[3]);
        put(
            &mut bytes,
            last + entry::ITEM_REFERENCE,
            &variant.to_le_bytes(),
        );
        put(
            &mut bytes,
            last + entry::PRODUCTION_COUNT,
            &4242_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            TABLE + slots * entry::STRIDE + entry::DISTANCE,
            &[0xff],
        );

        let (outcomes, _, entries, _) = run(&bytes, 4, 4);
        assert_eq!(outcomes.organisations, Outcome::Rows { len: 2 });
        assert_eq!(outcomes.history, Outcome::Rows { len: 3 });
        assert_eq!(
            entries[2],
            HistoryRow {
                organisation: CID_FIRST,
                variant,
                units: 4242,
                design_hour: None,
            }
        );

        // A fault beyond the former cap refuses the entire history.
        put(&mut bytes, last + entry::DISTANCE, &[0xff]);
        assert_eq!(
            run(&bytes, 4, 4).0.history,
            Outcome::Unavailable {
                reason: "organisation history sentinel inside its table",
            }
        );
    }

    #[test]
    fn history_live_extent_address_overflow_is_refused() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut fields = read_fields(&memory, VTABLE_RVA.0 as u64, FIRST).unwrap();
        for (history, slots) in [
            (usize::MAX - entry::STRIDE, fields.history_slots),
            (TABLE, usize::MAX / entry::STRIDE + 1),
        ] {
            fields.history = history;
            fields.history_slots = slots;
            assert_eq!(
                read_history(&memory, &fields, &mut [HistoryRow::default(); 4]),
                Err("organisation history address overflow")
            );
        }
    }

    #[test]
    fn live_organisation_demand_and_same_walk_growth_keep_mod_roster_and_long_names() {
        let count = 13_000usize;
        let mut bytes = fixture();
        let organisation = bytes[FIRST - BASE..FIRST - BASE + 0x218].to_vec();
        let pointers = 0x10000;
        let objects = pointers + count * 8;
        let heap = objects + count * organisation.len();
        let name = vec![b'x'; 300];
        bytes.resize(heap - BASE + name.len() + 1, 0);
        put(&mut bytes, heap, &name);
        array(
            &mut bytes,
            STATUS + cproduction_status::ORGANISATIONS,
            pointers,
            count as u32,
        );
        for index in 0..count {
            let at = objects + index * organisation.len();
            put(&mut bytes, at, &organisation);
            put(&mut bytes, pointers + index * 8, &(at as u64).to_le_bytes());
            put(
                &mut bytes,
                at + REFERENCE,
                &(((index as u64 + 1) << 32) | REFERENCE_TYPE_ID as u64).to_le_bytes(),
            );
            put(&mut bytes, at + NAME, &(heap as u64).to_le_bytes());
            put(
                &mut bytes,
                at + NAME + 16,
                &(name.len() as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                at + NAME + 24,
                &(name.len() as u64).to_le_bytes(),
            );
        }
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(
            demand(&memory, 0, BASE).unwrap(),
            (count, count * 2, count * 310)
        );
        let mut organisations = Vec::new();
        let mut history = Vec::new();
        let mut arena = Vec::new();
        let walked = walk(
            &memory,
            0,
            BASE,
            &mut organisations,
            &mut history,
            &mut arena,
        );
        assert_eq!(walked.organisations, Outcome::Rows { len: count });
        assert_eq!(walked.history, Outcome::Rows { len: count * 2 });
        assert_eq!(walked.bytes_len, count * 310);
        assert_eq!(organisations[count - 1].name.len, 300);
    }

    #[test]
    fn history_header_geometry_must_match_native_growth() {
        for (mask, overflow) in [(2_i32, 5_u8), (7, 4), (i32::MAX, 33), (-1, 5), (0, 2)] {
            let mut bytes = fixture();
            put(&mut bytes, FIRST + HISTORY_LAST, &mask.to_le_bytes());
            put(&mut bytes, FIRST + HISTORY_OVERFLOW, &[overflow]);
            let (outcomes, ..) = run(&bytes, 4, 4);
            assert_eq!(outcomes.organisations, Outcome::Rows { len: 2 });
            assert_eq!(
                outcomes.history,
                Outcome::Unavailable {
                    reason: "organisation history table geometry invalid",
                }
            );
        }
        let mut bytes = fixture();
        put(
            &mut bytes,
            FIRST + HISTORY_LAST,
            &((1_u32 << 30) - 1).to_le_bytes(),
        );
        put(&mut bytes, FIRST + HISTORY_OVERFLOW, &[32]);
        let fields = read_fields(
            &SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            VTABLE_RVA.0 as u64,
            FIRST,
        )
        .unwrap();
        assert_eq!(fields.history_slots, (1_usize << 30) + 32);
    }

    #[test]
    fn excess_occupied_history_count_refuses_before_later_batches() {
        let mut bytes = fixture();
        put(&mut bytes, FIRST + HISTORY_LAST, &63_i32.to_le_bytes());
        put(&mut bytes, FIRST + HISTORY_OVERFLOW, &[8]);
        put(&mut bytes, FIRST + HISTORY_COUNT, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            TABLE + 13 * entry::STRIDE + entry::DISTANCE,
            &[0],
        );
        // The next batch is unreadable; the excess second entry must win.
        bytes.truncate(TABLE - BASE + HISTORY_BATCH * entry::STRIDE);
        let (outcomes, ..) = run(&bytes, 4, 4);
        assert_eq!(outcomes.organisations, Outcome::Rows { len: 2 });
        assert_eq!(
            outcomes.history,
            Outcome::Unavailable {
                reason: "organisation history count mismatch",
            }
        );
    }

    #[test]
    fn an_unreadable_or_foreign_organisation_withholds_both_tables() {
        for (address, value, reason) in [
            (ARRAY + 8, 0_u64, "organisation pointer null"),
            (SECOND, 0x1234, "organisation vtable or reference mismatch"),
            (
                SECOND + REFERENCE,
                (27 << 32) | 70,
                "organisation vtable or reference mismatch",
            ),
            (
                SECOND + REFERENCE,
                REFERENCE_TYPE_ID as u64,
                "organisation vtable or reference mismatch",
            ),
            // An authentic identity at the end of memory, its fields beyond.
            (ARRAY + 8, END as u64 - 16, "organisation fields unreadable"),
            (
                SECOND + NAME + 16,
                16,
                "organisation name unreadable or invalid",
            ),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, END - 16, &(VTABLE_RVA.0 as u64).to_le_bytes());
            put(&mut bytes, END - 8, &CID_SECOND.to_le_bytes());
            put(&mut bytes, address, &value.to_le_bytes());
            let (outcomes, ..) = run(&bytes, 4, 4);
            let expected = Outcome::Unavailable { reason };
            assert_eq!(
                (outcomes.organisations, outcomes.history),
                (expected, expected),
                "{address:#x}"
            );
        }
        let mut bytes = fixture();
        array(
            &mut bytes,
            STATUS + cproduction_status::ORGANISATIONS,
            END,
            2,
        );
        assert_eq!(
            run(&bytes, 4, 4).0.organisations,
            Outcome::Unavailable {
                reason: "organisation pointers unreadable"
            }
        );
    }

    #[test]
    fn a_broken_history_withholds_only_the_history() {
        let sentinel = TABLE + entry::STRIDE + entry::DISTANCE;
        for (address, value, reason) in [
            (
                sentinel,
                vec![0xff],
                "organisation history sentinel inside its table",
            ),
            (
                FIRST + HISTORY_COUNT,
                3_u32.to_le_bytes().to_vec(),
                "organisation history count mismatch",
            ),
            (
                TABLE + entry::ITEM_REFERENCE,
                79_u32.to_le_bytes().to_vec(),
                "organisation history key is not a variant",
            ),
            (
                FIRST + HISTORY,
                (END as u64 - 0x40).to_le_bytes().to_vec(),
                "organisation history unreadable",
            ),
            (
                FIRST + HISTORY,
                0_u64.to_le_bytes().to_vec(),
                "organisation history table invalid",
            ),
            (
                FIRST + HISTORY_LAST,
                (-1_i32).to_le_bytes().to_vec(),
                "organisation history table geometry invalid",
            ),
            (
                FIRST + HISTORY_LAST,
                (-2_i32).to_le_bytes().to_vec(),
                "organisation history table geometry invalid",
            ),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &value);
            let (outcomes, ..) = run(&bytes, 4, 4);
            assert_eq!(outcomes.organisations, Outcome::Rows { len: 2 }, "{reason}");
            assert_eq!(outcomes.history, Outcome::Unavailable { reason });
        }
        // An empty history is not read at all, even with a broken table.
        let mut bytes = fixture();
        put(&mut bytes, FIRST + HISTORY_COUNT, &0_u32.to_le_bytes());
        put(&mut bytes, FIRST + HISTORY, &0_u64.to_le_bytes());
        assert_eq!(run(&bytes, 4, 4).0.history, Outcome::Rows { len: 0 });
    }
}
