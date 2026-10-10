//! Political-party layout with signed i64 fixed-point popularity; no low-word
//! projection is used.

use crate::{Memory, cideology_group, read};

/// Political-party primary vtable.
pub const VTABLE_RVA: usize = 0x0298_77e0;
/// Ideology-group ID, u32, copied from the group identity.
pub const ID: usize = 0x08;
/// CPoliticalParty CIdeologyGroup pointer.
pub const IDEOLOGY_GROUP: usize = 0x18;
/// Owning country tag index into the link-indexed tag array, u32.
/// Default naming resolves the stored index through that array.
pub const TAG: usize = 0x20;
/// Name localisation key, MSVC string, localised when the default flag is clear
/// and serialized as `name`.
pub const NAME: usize = 0x28;
/// Long-name localisation key, MSVC string, serialized as `long_name`.
pub const LONG_NAME: usize = 0x48;
/// Default-name flag. Construction sets it; party renaming clears it.
/// Serialization writes the flag, then both stored keys only when clear.
pub const NAME_DEFAULT: usize = 0x68;
/// CPoliticalParty country-leader pointer vector.
pub const COUNTRY_LEADERS: usize = 0x70;
/// CPoliticalParty leader-vector capacity u32.
pub const COUNTRY_LEADER_CAPACITY: usize = 0x78;
/// CPoliticalParty leader-vector count u32.
pub const COUNTRY_LEADER_COUNT: usize = 0x7c;
/// Popularity, signed i64/100000, accumulated and clamped to 0..10000000.
pub const POPULARITY: usize = 0x88;

/// One country leader of a party, in the party's native vector order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LeaderRow {
    /// Raw id of the owning CCountry.
    pub country: u32,
    /// The party's native ideology ID (+0x08).
    pub party: u32,
    /// Position in the party's leader vector (+0x70).
    pub ordinal: u32,
    /// The leader's CCharacter id behind [`crate::ccountry_leader::CHARACTER`];
    /// `None` for a null parent.
    pub character: Option<u32>,
    /// That leader's signed key in its parent's +0x98 role map; absent for a null parent.
    pub role: Option<i32>,
    /// [`crate::ccountry_leader::LEADER_ID`].
    pub leader_id: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Native ideology ID; combine with the owning country ID for a replay key.
    pub id: u32,
    pub popularity: i64,
    pub is_ruling: u8,
    pub token_start: usize,
    pub token_len: usize,
    /// The party's own country tag index (+0x20).
    pub tag: u32,
    /// The default-name flag (+0x68): 1 when the game builds the name from the tag and
    /// ideology group, 0 when it localises the stored keys.
    pub name_default: u8,
    /// Stored name key (+0x28), in the same arena as the token.
    pub name_start: usize,
    pub name_len: usize,
    /// Stored long-name key (+0x48), in the same arena.
    pub long_name_start: usize,
    pub long_name_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub rows: usize,
    pub bytes: usize,
}

/// Complete live party roster and three retained text lengths. Serialization
/// keeps stored name keys. Empty payload storage still visits every header
/// without copying text bytes.
pub fn demand(memory: &impl Memory, base: usize, country: usize) -> Result<Demand, &'static str> {
    let walked =
        crate::cpolitical_status::Reader::new(memory, base, country)?.parties(&mut [], &mut []);
    let rows = match walked.outcome {
        crate::walk::Outcome::Rows { len } => len,
        crate::walk::Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("political party sizing row count overflow")?,
        crate::walk::Outcome::Unavailable { reason } => return Err(reason),
    };
    Ok(Demand {
        rows,
        bytes: walked.required_bytes,
    })
}

pub(crate) fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(u32, usize), &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    let header = read::<[u8; 12]>(memory, pointer).ok_or("political party identity unreadable")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64 {
        return Err("political party vtable mismatch");
    }
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("political party address overflow")
    };
    let id = u32::from_le_bytes(header[ID..].try_into().unwrap());
    let ideology = read::<u64>(memory, at(IDEOLOGY_GROUP)?)
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("political party ideology pointer unreadable or null")?;
    let expected = base
        .checked_add(cideology_group::VTABLE_RVA)
        .ok_or("module address overflow")?;
    let header = read::<[u8; 12]>(memory, ideology).ok_or("ideology identity unreadable")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64 {
        return Err("political party ideology vtable mismatch");
    }
    // The constructor copies this exact native identity word. Neither the
    // ideology's spelling nor membership in a vanilla list authenticates it.
    let ideology_id = u32::from_le_bytes(header[cideology_group::ID..].try_into().unwrap());
    if ideology_id != id {
        return Err("political party and ideology ids differ");
    }
    Ok((id, ideology))
}
