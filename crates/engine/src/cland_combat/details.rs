use super::{
    ATTACKER, ATTACKER_BONUS, DEFENDER, DEFENDER_BONUS, Members, PHASE, PROVINCE, Reader,
    STORED_TERRAIN, Source, WIDTH_FACTOR,
};
use crate::{Memory, carmy, cland_combatant, cpdx_array::Array, read};

/// Each side's selected commander, made once per combat side and shared by
/// that combat's window, detail and tactic readers. 14129E190
/// (`CLandCombatant` vslot 29) selects from the side's own unit list alone, so
/// a selection is keyed by that list (its data and count): the pass holds the
/// simulation thread, and a second selection over the same list would read
/// the same words. Each reader meets the selection's result, or its failure,
/// where it would select; any other list selects afresh.
#[derive(Default, Clone, Copy)]
pub struct Commanders {
    sides: [Option<(Array, Selection)>; 2],
}

type Selection = Result<CommanderSource, &'static str>;

/// A span of one of the caller's [`DetailArenas`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

impl Span {
    pub fn range(self) -> std::ops::Range<usize> {
        self.start..self.start + self.len
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommanderTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub kind: u32,
    /// 141C60E40 filters on CUnitLeaderTrait+A74 for combat-relevant traits.
    pub relevant: u8,
    pub token_start: usize,
    pub token_len: usize,
}

/// One timed trait: its definition identity and the signed days left.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommanderTimedTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub remaining_days: i32,
}

/// Entries of each detail arena.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DetailCounts {
    pub participants: usize,
    pub air: usize,
    pub text: usize,
    pub traits: usize,
    pub timed: usize,
    pub learning: usize,
}

/// Caller-owned storage for every variable-length detail list: committed and
/// reserve handles, air-support plane counts, all token bytes, and both side
/// commanders' assigned, timed and learning traits. Nothing bounds a list but
/// its live vector: `required` counts every entry read, `used` those stored.
/// An entry that does not fit is still authenticated and counted but not
/// stored, so `used` falls short of `required` and the caller publishes no
/// partial detail; it grows the arenas off the game thread from `required`.
pub struct DetailArenas<'a> {
    pub participants: &'a mut [u64],
    pub air: &'a mut [i32],
    pub text: &'a mut [u8],
    pub traits: &'a mut [CommanderTrait],
    pub timed: &'a mut [CommanderTimedTrait],
    pub learning: &'a mut [CommanderLearningTrait],
    pub used: DetailCounts,
    pub required: DetailCounts,
}

impl<'a> DetailArenas<'a> {
    pub fn new(
        participants: &'a mut [u64],
        air: &'a mut [i32],
        text: &'a mut [u8],
        traits: &'a mut [CommanderTrait],
        timed: &'a mut [CommanderTimedTrait],
        learning: &'a mut [CommanderLearningTrait],
    ) -> Self {
        Self {
            participants,
            air,
            text,
            traits,
            timed,
            learning,
            used: DetailCounts::default(),
            required: DetailCounts::default(),
        }
    }

    /// Every entry read was stored.
    pub fn complete(&self) -> bool {
        self.used == self.required
    }
}

/// Count one entry and store it when the arena still has room.
pub(super) fn push<T>(arena: &mut [T], used: &mut usize, required: &mut usize, value: T) {
    *required += 1;
    if let Some(slot) = arena.get_mut(*used) {
        *slot = value;
        *used += 1;
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommanderLearningTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub required_experience: i32,
    pub experience: i64,
    pub token_start: usize,
    pub token_len: usize,
}

/// Authenticated pointers for related readers in the same stopped callback.
/// These are never replay identities and must never be serialized.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommanderSource {
    pub pointer: usize,
    pub handle: u64,
    pub skill: i32,
    pub category: i32,
}

pub fn commander_sources(
    memory: &impl Memory,
    base: usize,
    source: Source,
    members: &Members<'_>,
    commanders: &mut Commanders,
) -> Result<[CommanderSource; 2], &'static str> {
    let reader = Reader {
        memory,
        base,
        provinces: source.provinces,
    };
    let border = reader
        .class(source.pointer)?
        .ok_or("land commander source is naval")?;
    let header = reader.header(source.pointer, border)?;
    if header != (source.id, source.attackers, source.defenders) {
        return Err("land commander source changed");
    }
    Ok([
        reader.selected(commanders, members, 0, source.attackers)?,
        reader.selected(commanders, members, 1, source.defenders)?,
    ])
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SideDetails {
    pub committed_start: usize,
    pub committed_len: usize,
    pub reserve_start: usize,
    pub reserve_len: usize,
    pub tactic_id: i32,
    /// 0: null pointer (1415C66C0 returns); 1: CNullCombatTactic; 2: CCombatTactic.
    pub tactic_kind: u8,
    pub tactic_counter_id: u32,
    /// Spans of the text and air arenas.
    pub tactic_token: Span,
    pub air_counts: Span,
    /// The side's anti-air attack, which the CAS tooltip (1415C1A60) shows as
    /// AA_EFFECTIVENESS through 141298090; replayed by L4c's CAS tooltip.
    pub anti_air_attack: i64,
    pub air_kills: u32,
    /// Air strength/org, ground strength/org, prevented strength/org damage.
    pub damage: [i64; 6],
    pub organisation_loss: [i64; 24],
    pub organisation_loss_count: u32,
    pub commander: u64,
    pub commander_skill: i32,
    pub commander_category: i32,
    pub commander_country: u32,
    /// Raw CUnitLeader experience for L4c's commander skill tooltip.
    pub commander_experience: i64,
    /// The raw saved name at CUnitLeader+0x20 (a localisation key or literal);
    /// the localised +0x40 copy is the recording client's cache.
    pub commander_name_token: Span,
    /// The raw saved description token at +0x80; 140C0C950 localises it into
    /// the client cache at +0x60. L4c's portrait tooltip localises it instead.
    pub commander_description_token: Span,
    /// CArmyLeader+0x1048, the commander's formation reference, for the HQ badge
    /// (1415C5F10); zero when absent or the category cannot carry the badge.
    pub commander_formation: u64,
    pub presentation: crate::naval_commander_inputs::Presentation,
    /// The assigned-trait vector (+0xDC8) in native order, as a span of the
    /// caller's [`DetailArenas`]; each trait's token is a span of its text.
    pub traits_start: usize,
    pub traits_len: usize,
    /// The timed-trait vector (+0xDE0) in native order, as a span of the
    /// caller's arena; the trait tooltip's expiry line (141C5F730) reads the
    /// first entry matching a trait.
    pub timed_traits_start: usize,
    pub timed_traits_len: usize,
    /// The trait-experience vector (+0xDF8) in native order.
    pub learning_traits: Span,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DetailsRow {
    pub id: u64,
    pub province: u32,
    pub phase: i32,
    pub width_factor: i64,
    pub attacker_bonus: i64,
    pub defender_bonus: i64,
    pub stored_terrain_valid: u8,
    pub stored_terrain_name: Span,
    /// Attacker then defender, regardless of local viewing country.
    pub sides: [SideDetails; 2],
}

/// Copy only already-reached leaf spans. A checked-address failure stops at its
/// original ordinal after the readable prefix; no failed field is retried.
pub(super) fn copy_detail_spans<const N: usize>(
    memory: &impl Memory,
    sources: [(usize, usize); N],
    destinations: &mut [&mut [u8]; N],
) -> Result<(), &'static str> {
    if memory.copy_scattered_prefetched(
        sources.map(|(pointer, offset)| pointer.checked_add(offset).unwrap_or(0)),
        destinations,
    ) == N
    {
        Ok(())
    } else {
        Err("land combat field unreadable or address overflow")
    }
}

const SELECTED_NAME: crate::cname_group_member::StringErrors = [
    "land combat commander name token header invalid",
    "land combat commander name token pointer unreadable or null",
    "land combat commander name token bytes unreadable",
    "land combat commander name token terminator unreadable or invalid",
];
const SELECTED_DESCRIPTION: crate::cname_group_member::StringErrors = [
    "land combat commander description token header invalid",
    "land combat commander description token pointer unreadable or null",
    "land combat commander description token bytes unreadable",
    "land combat commander description token terminator unreadable or invalid",
];

/// Capture window operands from the authenticated census. Committed/reserve
/// references retain native order in the caller arena. Nothing calls the engine,
/// computes UI totals, or records a native pointer. Loss history is sampled only
/// when its native count admits all 24 entries, exactly as 1412A2AA0 does.
pub fn read_details(
    memory: &impl Memory,
    base: usize,
    source: Source,
    members: &Members<'_>,
    commanders: &mut Commanders,
    arenas: &mut DetailArenas<'_>,
) -> Result<DetailsRow, &'static str> {
    let reader = Reader {
        memory,
        base,
        provinces: source.provinces,
    };
    let border = reader
        .class(source.pointer)?
        .ok_or("land details source is naval")?;
    let (id, attackers, defenders) = reader.header(source.pointer, border)?;
    if id != source.id || attackers != source.attackers || defenders != source.defenders {
        return Err("land details source identity changed");
    }
    let province_id = reader.row(source.pointer, id)?.province;
    let (mut phase, mut width_factor, mut attacker_bonus, mut defender_bonus) =
        ([0; 4], [0; 8], [0; 8], [0; 8]);
    copy_detail_spans(
        memory,
        [
            (source.pointer, PHASE),
            (source.pointer, WIDTH_FACTOR),
            (source.pointer, ATTACKER_BONUS),
            (source.pointer, DEFENDER_BONUS),
        ],
        &mut [
            &mut phase,
            &mut width_factor,
            &mut attacker_bonus,
            &mut defender_bonus,
        ],
    )?;
    let mut row = DetailsRow {
        id,
        province: province_id,
        phase: i32::from_le_bytes(phase),
        width_factor: i64::from_le_bytes(width_factor),
        attacker_bonus: i64::from_le_bytes(attacker_bonus),
        defender_bonus: i64::from_le_bytes(defender_bonus),
        ..DetailsRow::default()
    };
    let stored_terrain = reader.pointer(source.pointer, STORED_TERRAIN)?;
    let stored_vtable = reader.field::<u64>(stored_terrain, 0)?;
    if ![
        crate::cterrain_type::VTABLE_RVA,
        crate::cterrain_type::NULL_VTABLE_RVA,
    ]
    .into_iter()
    .any(|rva| base.checked_add(rva).map(|p| p as u64) == Some(stored_vtable))
    {
        return Err("land stored terrain vtable mismatch");
    }
    row.stored_terrain_valid = reader.field(stored_terrain, crate::cterrain_type::VALID)?;
    if row.stored_terrain_valid != 0 {
        row.stored_terrain_name =
            reader.text(stored_terrain, crate::cterrain_type::NAME, arenas)?;
    }
    for (index, offset) in [ATTACKER, DEFENDER].into_iter().enumerate() {
        let pointer = reader.pointer(source.pointer, offset)?;
        let side = &mut row.sides[index];
        let units = [attackers, defenders][index];
        reader.commander(pointer, members, commanders, index, units, side, arenas)?;
        for (which, vector_offset) in [cland_combatant::COMMITTED, cland_combatant::RESERVES]
            .into_iter()
            .enumerate()
        {
            let vector = reader.array(pointer, vector_offset, 8)?;
            let (start, count) = (arenas.used.participants, vector.count as usize);
            if which == 0 {
                side.committed_start = start;
                side.committed_len = count;
            } else {
                side.reserve_start = start;
                side.reserve_len = count;
            }
            let mut hint = 0;
            for member in 0..vector.count {
                let unit = reader.element(vector, member)?;
                let handle = reader.known_unit(members, index, unit, &mut hint)?;
                push(
                    arenas.participants,
                    &mut arenas.used.participants,
                    &mut arenas.required.participants,
                    handle,
                );
            }
        }
        let tactic = reader.pointer(pointer, cland_combatant::TACTIC)?;
        let tactic_vtable = if tactic == 0 {
            0
        } else {
            reader.field::<u64>(tactic, 0)?
        };
        if Some(tactic_vtable)
            == base
                .checked_add(crate::ccombat_tactic::VTABLE_RVA)
                .map(|v| v as u64)
        {
            side.tactic_kind = 2;
            side.tactic_id = reader.field(tactic, crate::ccombat_tactic::ID)?;
            side.tactic_counter_id = reader.field(tactic, crate::ccombat_tactic::COUNTER_ID)?;
            side.tactic_token = reader.text(tactic, crate::ccombat_tactic::TOKEN, arenas)?;
        } else if tactic == 0 {
            side.tactic_kind = 0;
        } else if Some(tactic_vtable)
            != base
                .checked_add(crate::cnull_combat_tactic::VTABLE_RVA)
                .map(|v| v as u64)
        {
            return Err("land detail tactic vtable mismatch");
        } else {
            side.tactic_kind = 1;
        }
        let air = reader.array(pointer, cland_combatant::AIR_SUPPORT, 8)?;
        side.air_counts = Span {
            start: arenas.used.air,
            len: air.count as usize,
        };
        for member in 0..air.count {
            let planes = reader.field(
                reader.element(air, member)?,
                crate::land_combat_air_entry::PLANE_COUNT,
            )?;
            push(
                arenas.air,
                &mut arenas.used.air,
                &mut arenas.required.air,
                planes,
            );
        }
        let (mut anti_air, mut kills, mut damage, mut loss_count) =
            ([0; 8], [0; 4], [0; 48], [0; 4]);
        copy_detail_spans(
            memory,
            [
                (pointer, cland_combatant::ANTI_AIR_ATTACK),
                (pointer, cland_combatant::AIR_KILLS),
                (pointer, cland_combatant::AIR_STRENGTH_DAMAGE),
                (pointer, cland_combatant::ORGANISATION_LOSS_COUNT),
            ],
            &mut [&mut anti_air, &mut kills, &mut damage, &mut loss_count],
        )?;
        side.anti_air_attack = i64::from_le_bytes(anti_air);
        side.air_kills = u32::from_le_bytes(kills);
        for (value, bytes) in side.damage.iter_mut().zip(damage.chunks_exact(8)) {
            *value = i64::from_le_bytes(bytes.try_into().unwrap());
        }
        side.organisation_loss_count = u32::from_le_bytes(loss_count);
        if side.organisation_loss_count >= 24 {
            let samples = reader.pointer(pointer, cland_combatant::ORGANISATION_LOSS_WINDOW)?;
            let loss = reader.field::<[u8; 192]>(samples, 0)?;
            for (value, bytes) in side.organisation_loss.iter_mut().zip(loss.chunks_exact(8)) {
                *value = i64::from_le_bytes(bytes.try_into().unwrap());
            }
        }
    }
    Ok(row)
}

impl<M: Memory> Reader<'_, M> {
    /// `side`'s commander selection over `units`, made once (`Commanders`).
    fn selected(
        &self,
        commanders: &mut Commanders,
        members: &Members<'_>,
        side: usize,
        units: Array,
    ) -> Result<CommanderSource, &'static str> {
        if let Some((held, selection)) = commanders.sides[side]
            && held == units
        {
            return selection;
        }
        let selection = self.commander_source(members, side, units);
        commanders.sides[side] = Some((units, selection));
        selection
    }

    fn commander_source(
        &self,
        members: &Members<'_>,
        side: usize,
        units: Array,
    ) -> Result<CommanderSource, &'static str> {
        use crate::{carmy_leader, corders_group, cunit_leader, cunit_leader_skill};
        let mut selected = CommanderSource::default();
        for index in 0..units.count {
            let (unit, _) = self.participant(members, side, units, index)?;
            let group = self.pointer(unit, carmy::ORDERS_GROUP)?;
            if group == 0 {
                continue;
            }
            self.vtable(group, 0, corders_group::VTABLE_RVA)?;
            let leader = self.pointer(group, corders_group::LEADER)?;
            if leader == 0 {
                continue;
            }
            self.vtable(leader, 0, carmy_leader::VTABLE_RVA)?;
            let handle = self.field::<u64>(leader, cunit_leader::TYPE)?;
            if handle as u32 != cunit_leader::REFERENCE_TYPE || handle >> 32 == 0 {
                return Err("land commander reference invalid");
            }
            let skill = self.pointer(leader, cunit_leader::SKILL)?;
            self.vtable(skill, 0, cunit_leader_skill::VTABLE_RVA)?;
            let skill = self.field(skill, cunit_leader_skill::LEVEL)?;
            let category = self.field(leader, cunit_leader::CATEGORY)?;
            if selected.pointer == 0
                || skill > selected.skill
                || (skill == selected.skill && selected.category == 0 && category == 1)
            {
                selected = CommanderSource {
                    pointer: leader,
                    handle,
                    skill,
                    category,
                };
            }
        }
        Ok(selected)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn commander(
        &self,
        side: usize,
        members: &Members<'_>,
        commanders: &mut Commanders,
        index: usize,
        units: Array,
        row: &mut SideDetails,
        arenas: &mut DetailArenas<'_>,
    ) -> Result<(), &'static str> {
        use crate::{carmy_leader, cunit_leader, cunit_leader_trait};
        // 14129E190 selects the first highest skill, except an equal category-1
        // candidate replaces a category-0 leader. The selected reference is an
        // identity; numeric bonuses are kept as raw source fields.
        let commander = self.selected(commanders, members, index, units)?;
        let selected = commander.pointer;
        row.commander = commander.handle;
        row.commander_skill = commander.skill;
        row.commander_category = commander.category;
        if selected == 0 {
            // 1415C5F10 falls back to the first country-set entry, then the
            // attacked province controller when that set's +48 count is zero.
            row.commander_country = if self.field::<i32>(side, 0x48)? == 0 {
                let combat = self.pointer(side, cland_combatant::COMBAT)?;
                self.field(self.pointer(combat, PROVINCE)?, 0x188)?
            } else {
                self.field(self.pointer(side, cland_combatant::COUNTRIES)?, 0)?
            };
            return Ok(());
        }
        // Country +120 is consumed by 1415C5F10 and the trait-grid call in
        // 1415C8950. Preserve the exact tag rather than a canonicalized slot.
        row.commander_country = self.field(selected, 0x120)?;
        row.commander_experience = self.field(selected, cunit_leader::EXPERIENCE)?;
        // 140C16FB0 stores +0x20 as an ordinary MSVC string and hands it to
        // 142230F20, which takes any length, empty included; +0x80 is the same
        // kind of string. Both keep the game's own length.
        for (offset, unreadable, errors, span) in [
            (
                cunit_leader::TOKEN,
                "land combat commander name token header unreadable",
                &SELECTED_NAME,
                &mut row.commander_name_token,
            ),
            (
                cunit_leader::DESCRIPTION_TOKEN,
                "land combat commander description token header unreadable",
                &SELECTED_DESCRIPTION,
                &mut row.commander_description_token,
            ),
        ] {
            let header: [u8; crate::msvc_string::STRIDE] = selected
                .checked_add(offset)
                .and_then(|address| read(self.memory, address))
                .ok_or(unreadable)?;
            *span = self.string(&header, errors, arenas)?;
        }
        if (commander.category as u32) < 2 {
            row.commander_formation = self.field(selected, carmy_leader::FORMATION_REFERENCE)?;
        }
        row.presentation =
            crate::naval_commander_inputs::capture_presentation(self.memory, self.base, selected)?;
        // Every vector is sized by its live count alone; entries that do not
        // fit the caller's arenas are still authenticated and counted.
        let traits = self.array(selected, cunit_leader::TRAIT_DATA, 8)?;
        row.traits_start = arenas.used.traits;
        row.traits_len = traits.count as usize;
        for ordinal in 0..traits.count {
            let pointer = self.element(traits, ordinal)?;
            self.vtable(pointer, 0, cunit_leader_trait::VTABLE_RVA)?;
            let token = self.text(pointer, cunit_leader_trait::TOKEN, arenas)?;
            let value = CommanderTrait {
                token_id: self.field(pointer, cunit_leader_trait::TOKEN_ID)?,
                definition_ordinal: self.field(pointer, cunit_leader_trait::ORDINAL)?,
                kind: self.field(pointer, cunit_leader_trait::KIND)?,
                relevant: self.field(pointer, 0xa74)?,
                token_start: token.start,
                token_len: token.len,
            };
            push(
                arenas.traits,
                &mut arenas.used.traits,
                &mut arenas.required.traits,
                value,
            );
        }
        let timed = self.array(
            selected,
            cunit_leader::TIMED_TRAIT_DATA,
            cunit_leader::TIMED_TRAIT_STRIDE,
        )?;
        row.timed_traits_start = arenas.used.timed;
        row.timed_traits_len = timed.count as usize;
        // The (trait, days) entries in 64-entry guarded batches, then each
        // trait's identity and fields.
        const STRIDE: usize = cunit_leader::TIMED_TRAIT_STRIDE;
        let mut batch = [0_u8; 64 * STRIDE];
        for first in (0..timed.count).step_by(64) {
            let count = (timed.count - first).min(64) as usize;
            let entries = &mut batch[..count * STRIDE];
            let address = timed
                .element(first, STRIDE)
                .ok_or("land timed trait address overflow")?;
            if !crate::read_slice(self.memory, address, entries) {
                return Err("land combat field unreadable or address overflow");
            }
            for entry in entries.chunks_exact(STRIDE) {
                let pointer = usize::try_from(u64::from_le_bytes(entry[..8].try_into().unwrap()))
                    .map_err(|_| "land combat pointer overflow")?;
                self.vtable(pointer, 0, cunit_leader_trait::VTABLE_RVA)?;
                let value = CommanderTimedTrait {
                    token_id: self.field(pointer, cunit_leader_trait::TOKEN_ID)?,
                    definition_ordinal: self.field(pointer, cunit_leader_trait::ORDINAL)?,
                    remaining_days: i32::from_le_bytes(entry[8..12].try_into().unwrap()),
                };
                push(
                    arenas.timed,
                    &mut arenas.used.timed,
                    &mut arenas.required.timed,
                    value,
                );
            }
        }
        let learning = self.array(selected, cunit_leader::TRAIT_EXPERIENCE_DATA, 16)?;
        row.learning_traits = Span {
            start: arenas.used.learning,
            len: learning.count as usize,
        };
        for index in 0..learning.count {
            let entry = learning
                .element(index, 16)
                .ok_or("land learning trait address overflow")?;
            let pointer = self.pointer(entry, 0)?;
            self.vtable(pointer, 0, cunit_leader_trait::VTABLE_RVA)?;
            let token = self.text(pointer, cunit_leader_trait::TOKEN, arenas)?;
            let value = CommanderLearningTrait {
                token_id: self.field(pointer, cunit_leader_trait::TOKEN_ID)?,
                definition_ordinal: self.field(pointer, cunit_leader_trait::ORDINAL)?,
                required_experience: self
                    .field(pointer, cunit_leader_trait::REQUIRED_EXPERIENCE)?,
                experience: self.field(entry, 8)?,
                token_start: token.start,
                token_len: token.len,
            };
            push(
                arenas.learning,
                &mut arenas.used.learning,
                &mut arenas.required.learning,
                value,
            );
        }
        Ok(())
    }

    /// A nonempty string's bytes into the text arena. The native length is
    /// the only bound; bytes that do not fit are counted, not stored.
    pub(super) fn text(
        &self,
        pointer: usize,
        offset: usize,
        arenas: &mut DetailArenas<'_>,
    ) -> Result<Span, &'static str> {
        let address = pointer
            .checked_add(offset)
            .ok_or("land detail string address overflow")?;
        let (source, len) = crate::cdynamic_modifier::literal(self.memory, address)?;
        let start = arenas.used.text;
        arenas.required.text = arenas
            .required
            .text
            .checked_add(len)
            .ok_or("land detail text overflows")?;
        if let Some(output) = start
            .checked_add(len)
            .and_then(|end| arenas.text.get_mut(start..end))
        {
            if !crate::read_slice(self.memory, source, output) {
                return Err("land detail string unreadable");
            }
            arenas.used.text += len;
        }
        Ok(Span { start, len })
    }

    /// An MSVC string of any length, empty included, into the text arena
    /// (cname_group_member::copy_string, which names each invalid part).
    /// Bytes that do not fit are counted, not stored.
    fn string(
        &self,
        header: &[u8],
        errors: &crate::cname_group_member::StringErrors,
        arenas: &mut DetailArenas<'_>,
    ) -> Result<Span, &'static str> {
        let mut names = crate::cname_group_member::Names {
            bytes: &mut *arenas.text,
            len: arenas.used.text,
            required: arenas.required.text,
        };
        let copied =
            crate::cname_group_member::copy_string(self.memory, header, &mut names, errors)?;
        let len = names.required - arenas.required.text;
        arenas.required.text = names.required;
        let start = arenas.used.text;
        if copied.is_some() {
            arenas.used.text += len;
        }
        Ok(Span { start, len })
    }
}
