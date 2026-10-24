//! CCharacter state and role links. Serial writer paths are characterized;
//! live consistency and capture acceptance are unverified.
//!
//! `ccharacter_template` owns template identity fields and their writers.
//! `FLAGS` is this character's flag manager, read unchanged by `cflag_manager`.
//! Synthetic fixtures cover template reading, identity reuse, truncation and a
//! template the loader never read; fresh-save agreement remains unverified.

use crate::{
    Memory, ccharacter_manager, ccharacter_template, msvc_string, read, read_slice, walk::Outcome,
};

/// Character vtable.
pub const VTABLE_RVA: usize = 0x0297_ea60;
/// Reference type value, u32.
pub const REFERENCE_TYPE: u32 = 73;
/// Reference-type field, u32, in the primary object header.
pub const TYPE: usize = 0x08;
/// Instance id, u32.
pub const ID: usize = 0x0c;
/// Signed lexer token; serialization resolves it and the lossy name index excludes negatives.
pub const NAME_TOKEN_ID: usize = 0x18;
/// Optional template pointer, initially null. Definition building and save
/// loading select it; serialization retains the selected template tag.
pub const TEMPLATE: usize = 0x20;
/// Unsaved string member.
pub const NAME: usize = 0x28;
/// Saved character name string.
pub const TOKEN: usize = 0x48;
/// Raw country handle, u32, requiring country-link resolution.
pub const COUNTRY: usize = 0x68;
/// Stored gender enum; pronouns select feminine only for value two.
pub const GENDER: usize = 0x70;
/// Embedded portrait container.
pub const PORTRAITS: usize = 0x78;
/// Portrait-entry data pointer.
pub const PORTRAIT_DATA: usize = 0x80;
/// Portrait-entry count, u32.
#[cfg(test)]
pub const PORTRAIT_COUNT: usize = 0x8c;
/// Ideology-to-CCountryLeader map head.
pub const LEADER_MAP_HEAD: usize = 0x98;
/// Ideology-to-CCountryLeader map size, u64.
pub const LEADER_MAP_SIZE: usize = 0xa0;
/// Nullable CUnitLeader role pointer.
pub const UNIT_LEADER: usize = 0xa8;
/// String-to-CAdvisor map head.
pub const ADVISOR_MAP_HEAD: usize = 0xb0;
/// String-to-CAdvisor map size, u64.
pub const ADVISOR_MAP_SIZE: usize = 0xb8;
/// CScientist role pointer.
pub const SCIENTIST: usize = 0xc0;
/// Unit-leader full reference. A direct leader pointer wins; otherwise
/// resolve this reference, with both-zero meaning absence. Construction
/// zeroes it, operative creation copies the full identity, and loading
/// restores it. Serialization includes it when either word is nonzero.
pub const OPERATIVE: usize = 0xc8;
/// Embedded script-variable storage, constructed with the character table
/// and serialized when nonempty.
pub const VARIABLES: usize = 0xd8;
/// Embedded flag storage used by character flag conditions and mutations;
/// serialized when its count is positive.
pub const FLAGS: usize = 0x110;
/// Extent of the fields one `View` can hold, ending after the advisor map
/// size. It bounds `View`'s copied-range lookup only; it is not a class size,
/// and no span is read merely because it lies inside this extent.
pub const VIEW_EXTENT: usize = ADVISOR_MAP_SIZE + 8;
/// The name token id span a `View` copies: from `NAME_TOKEN_ID` up to `NAME`,
/// so the character's `TEMPLATE` pointer rides in the same guarded span and
/// is checked every hour without another read.
pub const NAME_TOKEN_SPAN: usize = NAME - NAME_TOKEN_ID;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    /// Exact native country-reference bits. Unrecruited characters may have zero.
    pub country: u32,
    pub name_token_id: i32,
    pub gender: u32,
    /// Serialized manager vector: zero historical, one dynamic.
    pub roster: u8,
    pub ordinal: u32,
    /// Exact unsaved `NAME` string. This legacy field label is not the saved name.
    pub name_start: usize,
    pub name_len: usize,
    /// Exact `TOKEN` string serialized as name; the script token is NAME_TOKEN_ID.
    pub token_start: usize,
    pub token_len: usize,
    /// The `TEMPLATE` pointer, zero without a template. It guards a reuse of
    /// the identity below; it is not published.
    pub template: usize,
    /// The template's exact fields (`ccharacter_template`); zero and empty
    /// without a template.
    pub template_identity: ccharacter_template::Identity,
    pub template_file_start: usize,
    pub template_file_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    /// Exact when complete; a lower bound including the first oversized row
    /// when truncated. Omitted rows are not read merely to size the pool.
    pub required_bytes: usize,
}

/// One authenticated roster occurrence: pointer, native id and its position
/// in the serialized manager vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occurrence {
    pub pointer: usize,
    pub id: u32,
    pub roster: u8,
    pub ordinal: u32,
}

/// Exact field bytes at `COUNTRY`, `LEADER_MAP_HEAD`, `UNIT_LEADER` and
/// `ADVISOR_MAP_HEAD`, each copied in the current callback.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AcquiredFields {
    pub country: Option<[u8; 4]>,
    pub leaders: Option<[u8; 16]>,
    pub unit: Option<[u8; 8]>,
    pub advisors: Option<[u8; 16]>,
}

/// Exact current-pass field spans. Unreadable names or role headers cannot hide
/// another domain; only fully copied spans are exposed to its native walker.
/// Pointer targets still use guarded memory, and every pass creates a new view.
pub struct View<'a, M: Memory> {
    memory: &'a M,
    pointer: usize,
    roster: u8,
    ordinal: u32,
    identity: [u8; 16],
    country: Option<[u8; 4]>,
    name_token_id: Option<[u8; NAME_TOKEN_SPAN]>,
    names: Option<[u8; 64]>,
    gender: Option<[u8; 4]>,
    leaders: Option<[u8; 16]>,
    unit: Option<[u8; 8]>,
    advisors: Option<[u8; 16]>,
}

impl<'a, M: Memory> View<'a, M> {
    pub(crate) fn read(
        memory: &'a M,
        base: usize,
        pointer: usize,
        roster: u8,
        ordinal: u32,
        required: [bool; 4],
    ) -> Result<Self, &'static str> {
        let mut identity = [0; 16];
        let mut country = [0; 4];
        let mut name_token_id = [0; NAME_TOKEN_SPAN];
        let mut names = [0; 64];
        let mut leaders = [0; 16];
        let mut unit = [0; 8];
        let mut advisors = [0; 16];
        let mut gender = [0; 4];
        let enabled = [
            true,
            required[0] || required[1] || required[3],
            required[0],
            required[0],
            required[1],
            required[2],
            required[3],
            required[0],
        ];
        let offsets = [
            0,
            COUNTRY,
            NAME_TOKEN_ID,
            NAME,
            LEADER_MAP_HEAD,
            UNIT_LEADER,
            ADVISOR_MAP_HEAD,
            GENDER,
        ];
        let addresses = offsets.map(|offset| pointer.checked_add(offset).unwrap_or(0));
        let mut spans: [&mut [u8]; 8] = [
            &mut identity,
            &mut country,
            &mut name_token_id,
            &mut names,
            &mut leaders,
            &mut unit,
            &mut advisors,
            &mut gender,
        ];
        for (span, enabled) in spans.iter_mut().zip(enabled) {
            if !enabled {
                *span = &mut [];
            }
        }
        let completed = memory.copy_scattered(addresses, &mut spans);
        if completed == 0 {
            return Err("character identity unreadable");
        }
        let mut copied =
            std::array::from_fn::<_, 8, _>(|index| enabled[index] && index < completed);
        // The failed field may contain a copied prefix and is never retried or
        // exposed. Resume only later independent fields, each at most once.
        for index in completed.saturating_add(1)..spans.len() {
            if enabled[index] {
                copied[index] = read_slice(memory, addresses[index], spans[index]);
            }
        }
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
        if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64
            || u32::from_le_bytes(identity[TYPE..TYPE + 4].try_into().unwrap()) != REFERENCE_TYPE
        {
            return Err("character vtable or reference type mismatch");
        }
        Ok(Self {
            memory,
            pointer,
            roster,
            ordinal,
            identity,
            country: copied[1].then_some(country),
            name_token_id: copied[2].then_some(name_token_id),
            names: copied[3].then_some(names),
            leaders: copied[4].then_some(leaders),
            unit: copied[5].then_some(unit),
            advisors: copied[6].then_some(advisors),
            gender: copied[7].then_some(gender),
        })
    }

    /// The view `read` returns for an occurrence whose identity and role
    /// fields another acquisition in this callback already authenticated and
    /// copied. Names are never supplied; `None` fields stay uncopied, exactly
    /// like fields `read` does not request or fails to copy.
    pub fn acquired(
        memory: &'a M,
        base: usize,
        occurrence: Occurrence,
        fields: AcquiredFields,
    ) -> Result<Self, &'static str> {
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
        let mut identity = [0; 16];
        identity[..8].copy_from_slice(&(expected as u64).to_le_bytes());
        identity[TYPE..TYPE + 4].copy_from_slice(&REFERENCE_TYPE.to_le_bytes());
        identity[ID..ID + 4].copy_from_slice(&occurrence.id.to_le_bytes());
        Ok(Self {
            memory,
            pointer: occurrence.pointer,
            roster: occurrence.roster,
            ordinal: occurrence.ordinal,
            identity,
            country: fields.country,
            name_token_id: None,
            names: None,
            gender: None,
            leaders: fields.leaders,
            unit: fields.unit,
            advisors: fields.advisors,
        })
    }

    /// Unit-leader rows do not depend on the character's country field.
    pub fn reference(&self) -> (usize, u32) {
        (
            self.pointer,
            u32::from_le_bytes(self.identity[ID..ID + 4].try_into().unwrap()),
        )
    }

    pub fn entry(&self) -> Result<ccharacter_manager::Entry, &'static str> {
        let (pointer, id) = self.reference();
        Ok(ccharacter_manager::Entry {
            pointer,
            id,
            country: u32::from_le_bytes(self.country.ok_or("character country unreadable")?),
            roster: self.roster,
            ordinal: self.ordinal,
        })
    }

    /// Ok(None) is foreign memory; Err is an uncopied local field and must never
    /// fall back to reading the failed or unrequested span a second time.
    fn copied(&self, address: usize, len: usize) -> Result<Option<&[u8]>, ()> {
        let end = address.checked_add(len).ok_or(())?;
        let object_end = self.pointer.saturating_add(VIEW_EXTENT);
        if end <= self.pointer || address >= object_end {
            return Ok(None);
        }
        let start = address.checked_sub(self.pointer).ok_or(())?;
        let end = start.checked_add(len).ok_or(())?;
        // The same eight spans in the same order, tested one at a time. The
        // walk asks this question many times per character, so the candidates
        // are never materialised as a table of slices.
        macro_rules! field {
            ($offset:expr, $size:expr, $bytes:expr) => {
                if let Some(from) = start.checked_sub($offset)
                    && end <= $offset + $size
                {
                    return $bytes
                        .as_ref()
                        .and_then(|bytes| bytes.get(from..end - $offset))
                        .map(Some)
                        .ok_or(());
                }
            };
        }
        field!(0, 16, Some(&self.identity));
        field!(COUNTRY, 4, self.country);
        field!(NAME_TOKEN_ID, NAME_TOKEN_SPAN, self.name_token_id);
        field!(NAME, 64, self.names);
        field!(LEADER_MAP_HEAD, 16, self.leaders);
        field!(UNIT_LEADER, 8, self.unit);
        field!(ADVISOR_MAP_HEAD, 16, self.advisors);
        field!(GENDER, 4, self.gender);
        Err(())
    }
}

impl<M: Memory> Memory for View<'_, M> {
    #[inline]
    fn read_scalar<T: crate::memory::Raw>(&self, address: usize) -> Option<T> {
        if size_of::<T>() == 0 {
            return Some(T::ZERO);
        }
        match self.copied(address, size_of::<T>()) {
            Ok(Some(source)) => crate::memory::scalar_from_bytes(source),
            Ok(None) => self.memory.read_scalar(address),
            Err(()) => None,
        }
    }

    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        match self.copied(address, destination.len()) {
            Ok(Some(source)) => {
                destination.copy_from_slice(source);
                true
            }
            Ok(None) => self.memory.copy(address, destination),
            Err(()) => false,
        }
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        // Foreign role/leader fields retain the native batched SEH path. Mixed
        // copied/foreign spans remain ordered, including the first fault rule.
        if addresses
            .iter()
            .zip(destinations.iter())
            .all(|(&address, destination)| {
                matches!(self.copied(address, destination.len()), Ok(None))
            })
        {
            return self.memory.copy_scattered(addresses, destinations);
        }
        for (index, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if !destination.is_empty()
                && (address == 0
                    || address.checked_add(destination.len()).is_none()
                    || !self.copy(address, destination))
            {
                return index;
            }
        }
        N
    }
}

/// Reads one entry from the current authenticated manager census. The entry's
/// ordinal and raw identity are retained; all returned offsets are arena-local.
///
/// After both names the character's template (`TEMPLATE`) is read, unless
/// `known` supplies the identity an earlier hour read for the same template
/// pointer; the file bytes follow the names in the arena. A row whose file
/// does not fit is omitted with its whole text demand, like the names.
pub fn walk_entry<'k>(
    memory: &impl Memory,
    base: usize,
    entry: ccharacter_manager::Entry,
    known: impl FnOnce(usize) -> Option<ccharacter_template::Known<'k>>,
    output: &mut [Row],
    bytes: &mut [u8],
) -> NameWalk {
    let result = (|| {
        let Some(row) = output.first_mut() else {
            return Ok(NameWalk {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 1,
                },
                bytes_len: 0,
                required_bytes: 0,
            });
        };
        let name_token_id = read::<i32>(memory, at(entry.pointer, NAME_TOKEN_ID)?)
            .ok_or("character name token id unreadable")?;
        let name = Literal::read(memory, at(entry.pointer, NAME)?)?;
        let token = Literal::read(memory, at(entry.pointer, TOKEN)?)?;
        Literal::validate(memory, [&name, &token])?;
        let required_bytes = name
            .len
            .checked_add(token.len)
            .ok_or("character text size overflow")?;
        if required_bytes > bytes.len() {
            return Ok(NameWalk {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 1,
                },
                bytes_len: 0,
                required_bytes,
            });
        }
        Literal::copy_all(memory, [&name, &token], &mut bytes[..required_bytes])?;
        let names = required_bytes;
        let template = read::<u64>(memory, at(entry.pointer, TEMPLATE)?)
            .and_then(|pointer| usize::try_from(pointer).ok())
            .ok_or("character template pointer unreadable")?;
        let mut identity = ccharacter_template::Identity::default();
        let mut file_len = 0;
        if template != 0 {
            let fits = |len: usize| {
                names
                    .checked_add(len)
                    .ok_or("character text size overflow")
                    .map(|required| (required, required <= bytes.len()))
            };
            let (required, fit) = if let Some(known) = known(template) {
                identity = known.identity;
                file_len = known.file.len();
                let (required, fit) = fits(file_len)?;
                if fit {
                    bytes[names..required].copy_from_slice(known.file);
                }
                (required, fit)
            } else {
                let header = ccharacter_template::read(memory, base, template)?;
                Literal::validate(memory, [&header.file])?;
                identity = header.identity;
                file_len = header.file.len;
                let (required, fit) = fits(file_len)?;
                if fit {
                    Literal::copy_all(memory, [&header.file], &mut bytes[names..required])?;
                }
                (required, fit)
            };
            if !fit {
                return Ok(NameWalk {
                    outcome: Outcome::Truncated {
                        len: 0,
                        unread_rows: 1,
                    },
                    bytes_len: 0,
                    required_bytes: required,
                });
            }
        }
        let required_bytes = names + file_len;
        *row = Row {
            id: entry.id,
            country: entry.country,
            name_token_id,
            gender: read::<u32>(memory, at(entry.pointer, GENDER)?)
                .ok_or("character gender unreadable")?,
            roster: entry.roster,
            ordinal: entry.ordinal,
            name_start: 0,
            name_len: name.len,
            token_start: name.len,
            token_len: token.len,
            template,
            template_identity: identity,
            template_file_start: names,
            template_file_len: file_len,
        };
        Ok(NameWalk {
            outcome: Outcome::Rows { len: 1 },
            bytes_len: required_bytes,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

/// Reads all manager-owned characters, including inactive/unrecruited entries.
/// The two native strings and lexer token remain separate, exact values. A row
/// whose strings do not fit is wholly omitted and the remaining capacity is
/// reported for pool growth. A fault never publishes a successful prefix.
/// Rows retain native roster order; the spool thread sorts and checks row keys.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    bytes: &mut [u8],
) -> NameWalk {
    let result = (|| {
        let mut len = 0;
        let mut bytes_len = 0;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        let source = ccharacter_manager::Roster::read(memory, base, game_state)?;
        for index in 0..source.count() {
            if len == output.len() {
                unread_rows = source.count() - index;
                break;
            }
            let entry = source.at(memory, index)?;
            let result = walk_entry(
                memory,
                base,
                entry,
                |_| None,
                &mut output[len..],
                &mut bytes[bytes_len..],
            );
            required_bytes = required_bytes
                .checked_add(result.required_bytes)
                .ok_or("character text size overflow")?;
            match result.outcome {
                Outcome::Rows { len: 1 } => {
                    output[len].name_start += bytes_len;
                    output[len].token_start += bytes_len;
                    output[len].template_file_start += bytes_len;
                    bytes_len += result.bytes_len;
                    len += 1;
                }
                Outcome::Truncated { .. } => {
                    unread_rows = source.count() - index;
                    break;
                }
                Outcome::Unavailable { reason } => return Err(reason),
                _ => return Err("character entry returned an invalid row count"),
            }
        }
        Ok(NameWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

/// The Characters rows and text bytes a complete walk needs now: one row per
/// roster occurrence, both names and the template file of each.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub rows: usize,
    pub bytes: usize,
}

/// Sizes the Characters buffers from the live roster before a walk needs
/// them. It reads each character's view (identity, country, name token span
/// and both string headers) and, with a template, the template's header; it
/// copies no text, and nothing it reads is recorded.
pub fn demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<Demand, &'static str> {
    let roster = ccharacter_manager::Roster::read(memory, base, game_state)?;
    let mut cursor = roster.cursor();
    let mut demand = Demand::default();
    while let Some(view) = cursor.next(memory, [true, false, false, false])? {
        let (pointer, _) = view.reference();
        let name = Literal::read(&view, at(pointer, NAME)?)?;
        let token = Literal::read(&view, at(pointer, TOKEN)?)?;
        let template = read::<u64>(&view, at(pointer, TEMPLATE)?)
            .and_then(|template| usize::try_from(template).ok())
            .ok_or("character template pointer unreadable")?;
        let file = if template == 0 {
            0
        } else {
            ccharacter_template::read(memory, base, template)?.file.len
        };
        demand.rows += 1;
        demand.bytes = [name.len, token.len, file]
            .into_iter()
            .try_fold(demand.bytes, usize::checked_add)
            .ok_or("character text size overflow")?;
    }
    Ok(demand)
}

pub fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("character address overflow")
}

/// Authenticates the reference header without narrowing its native id or tag.
pub fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(u32, u32), &'static str> {
    let id = identity_header(memory, base, pointer)?;
    let country =
        read::<u32>(memory, at(pointer, COUNTRY)?).ok_or("character country unreadable")?;
    Ok((id, country))
}

/// `View::read`'s identity authentication with up to three more fixed spans
/// of the same object in the same scattered read. Returns the native id and
/// how many of those spans were copied, in order; an empty span counts as
/// copied. A span whose address overflows is not copied.
pub fn identity_with_spans(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    offsets: [usize; 3],
    spans: [&mut [u8]; 3],
) -> Result<(u32, usize), &'static str> {
    let mut identity = [0; 16];
    let [first, second, third] = spans;
    let [a, b, c] = offsets.map(|offset| pointer.checked_add(offset).unwrap_or(0));
    let completed = memory.copy_scattered(
        [pointer, a, b, c],
        &mut [&mut identity[..], first, second, third],
    );
    if completed == 0 {
        return Err("character identity unreadable");
    }
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected as u64
        || u32::from_le_bytes(identity[TYPE..ID].try_into().unwrap()) != REFERENCE_TYPE
    {
        return Err("character vtable or reference type mismatch");
    }
    Ok((
        u32::from_le_bytes(identity[ID..16].try_into().unwrap()),
        completed - 1,
    ))
}

/// The authenticated CCharacter id behind a role's parent pointer: the vtable and reference type
/// must match before the id is returned.
pub(crate) fn identity_header(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<u32, &'static str> {
    let header = read::<[u8; 16]>(memory, pointer).ok_or("character identity unreadable")?;
    identity_from_header(base, header)
}

/// [`identity_header`] over its 16 header bytes, already copied.
pub(crate) fn identity_from_header(base: usize, header: [u8; 16]) -> Result<u32, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64
        || u32::from_le_bytes(header[TYPE..ID].try_into().unwrap()) != REFERENCE_TYPE
    {
        return Err("character vtable or reference type mismatch");
    }
    Ok(u32::from_le_bytes(header[ID..16].try_into().unwrap()))
}

/// A copied native string header. Inline bytes are reused directly, so a short
/// name needs one guarded read rather than several field and payload reads.
pub struct Literal {
    header: [u8; 32],
    source: usize,
    pub len: usize,
    inline: bool,
}

impl Literal {
    pub(crate) fn read(memory: &impl Memory, pointer: usize) -> Result<Self, &'static str> {
        let header =
            read::<[u8; 32]>(memory, pointer).ok_or("character string header unreadable")?;
        Self::from_header(pointer, header)
    }

    /// The caller validates heap terminators together before publishing a row
    /// or reporting its byte capacity. Inline terminators are checked here.
    pub fn from_header(pointer: usize, header: [u8; 32]) -> Result<Self, &'static str> {
        let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
        let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
        if capacity < msvc_string::INLINE_CAPACITY || len > capacity {
            return Err("character string header invalid");
        }
        let len = usize::try_from(len).map_err(|_| "character string length overflow")?;
        let inline = capacity == msvc_string::INLINE_CAPACITY;
        let source = if inline {
            pointer
        } else {
            usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
                .ok()
                .filter(|&p| p != 0)
                .ok_or("character string pointer invalid")?
        };
        if inline && header[len] != 0 {
            return Err("character string terminator invalid");
        }
        Ok(Self {
            header,
            source,
            len,
            inline,
        })
    }

    /// Address of the string's first byte (inline storage or heap buffer).
    pub fn source(&self) -> usize {
        self.source
    }

    pub fn validate<const N: usize>(
        memory: &impl Memory,
        literals: [&Self; N],
    ) -> Result<(), &'static str> {
        let mut terminators = [0; N];
        let mut addresses = [0; N];
        let mut destinations = terminators.each_mut().map(std::slice::from_mut);
        let mut heap = false;
        for (index, literal) in literals.iter().enumerate() {
            if literal.inline {
                destinations[index] = &mut [];
            } else {
                heap = true;
                addresses[index] = at(literal.source, literal.len)?;
            }
        }
        if heap && memory.copy_scattered(addresses, &mut destinations) != N {
            return Err("character string terminator unreadable or invalid");
        }
        if terminators.iter().any(|&value| value != 0) {
            return Err("character string terminator unreadable or invalid");
        }
        Ok(())
    }

    pub fn copy_all<const N: usize>(
        memory: &impl Memory,
        literals: [&Self; N],
        output: &mut [u8],
    ) -> Result<(), &'static str> {
        let len = literals
            .iter()
            .try_fold(0_usize, |len, literal| len.checked_add(literal.len));
        if len != Some(output.len()) {
            return Err("character string destination length mismatch");
        }
        let mut remaining = output;
        let mut destinations = std::array::from_fn(|index| {
            let (destination, rest) =
                std::mem::take(&mut remaining).split_at_mut(literals[index].len);
            remaining = rest;
            destination
        });
        let mut heap = false;
        for (literal, destination) in literals.iter().zip(destinations.iter_mut()) {
            if literal.inline {
                destination.copy_from_slice(&literal.header[..literal.len]);
                *destination = &mut [];
            } else if literal.len != 0 {
                heap = true;
            }
        }
        if heap
            && memory.copy_scattered(literals.map(|literal| literal.source), &mut destinations) != N
        {
            return Err("character string bytes unreadable");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

mod portraits;
pub use portraits::{
    PortraitRow, PortraitWalk, sample_reused_portraits, walk_portraits, walk_portraits_growing,
};

fn portrait_header(memory: &impl Memory, pointer: usize) -> Result<[u8; 16], &'static str> {
    let country_at = at(pointer, COUNTRY)?;
    let header_at = pointer.checked_add(PORTRAIT_DATA);
    if country_at.checked_add(4).is_some()
        && let Some(header_at) = header_at.filter(|address| address.checked_add(16).is_some())
    {
        let mut country = [0; 4];
        let mut header = [0; 16];
        // Country has no value-dependent gate before the portrait header.
        // Retain both original spans and their first-fault order under one guard.
        match memory
            .copy_scattered_prefetched([country_at, header_at], &mut [&mut country, &mut header])
        {
            0 => Err("character country unreadable"),
            2 => Ok(header),
            _ => Err("character portrait header unreadable"),
        }
    } else {
        // A later address overflow must not discard the country observation.
        read::<u32>(memory, country_at).ok_or("character country unreadable")?;
        let address = header_at.ok_or("character portrait address overflow")?;
        read(memory, address).ok_or("character portrait header unreadable")
    }
}
