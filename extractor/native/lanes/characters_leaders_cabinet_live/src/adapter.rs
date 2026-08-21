use std::collections::{BTreeMap, BTreeSet};

use crate::domain47_characters_leaders_cabinet_live as compact;
use crate::extractor::{
    advisor_bucket, read_active_ruler_from_country, read_advisor_raw_fields,
    read_character_raw_fields, read_unit_leader_skill, read_unit_leader_trait_vector,
    read_vtable_and_character_ptr, AdvisorRecord, CharacterPortraitRecord, CharacterRecord,
    CountryLeaderRoleRecord, ExtractError, LaneFrame, LaneMemory, ResolvedTypeRoots,
    UnitLeaderRecord, UnitLeaderTraitRecord, CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
    CCOUNTRY_LEADER_RAW_FIELDS_SPAN, CUNIT_LEADER_CHARACTER_ID_COPY_OFF, CUNIT_LEADER_ID_OFF,
    CUNIT_LEADER_TYPE_ID, CUNIT_LEADER_TYPE_OFF,
};

use compact::{UNIT_LEADER_ARMY, UNIT_LEADER_NAVY};

pub const CUNIT_LEADER_EXPERIENCE_RAW_OFF: u64 = 0xE68;
pub const CARMY_LEADER_ATTACK_OFF: u64 = 0xF58;
pub const CARMY_LEADER_DEFENSE_OFF: u64 = 0xF68;
pub const CARMY_LEADER_PLANNING_OFF: u64 = 0xF78;
pub const CARMY_LEADER_LOGISTICS_OFF: u64 = 0xF88;
pub const CNAVY_LEADER_ATTACK_OFF: u64 = 0xF68;
pub const CNAVY_LEADER_DEFENSE_OFF: u64 = 0xF78;
pub const CNAVY_LEADER_MANEUVERING_OFF: u64 = 0xF88;
pub const CNAVY_LEADER_COORDINATION_OFF: u64 = 0xF98;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdapterInput {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub skipped_input_rows: u32,
    pub characters: Vec<CharacterRoot>,
    pub unit_leaders: Vec<UnitLeaderRoot>,
    pub unit_leader_trait_tokens: Vec<TraitDefinitionRoot>,
    pub country_leader_roles: Vec<CountryLeaderRoleRoot>,
    pub countries: Vec<CountryPoliticsRoot>,
    pub advisors: Vec<AdvisorRoot>,
    pub portraits: Vec<CharacterPortraitInput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdapterInputRef<'a> {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub skipped_input_rows: u32,
    pub characters: &'a [CharacterRoot],
    pub unit_leaders: &'a [UnitLeaderRoot],
    pub unit_leader_trait_tokens: &'a [TraitDefinitionRoot],
    pub country_leader_roles: &'a [CountryLeaderRoleRoot],
    pub countries: &'a [CountryPoliticsRoot],
    pub advisors: &'a [AdvisorRoot],
    pub portraits: &'a [CharacterPortraitInput],
}

impl AdapterInput {
    pub fn as_ref(&self) -> AdapterInputRef<'_> {
        AdapterInputRef {
            elapsed_wall_ms: self.elapsed_wall_ms,
            status_flags: self.status_flags,
            skipped_input_rows: self.skipped_input_rows,
            characters: &self.characters,
            unit_leaders: &self.unit_leaders,
            unit_leader_trait_tokens: &self.unit_leader_trait_tokens,
            country_leader_roles: &self.country_leader_roles,
            countries: &self.countries,
            advisors: &self.advisors,
            portraits: &self.portraits,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterRoot {
    pub character_addr: u64,
    pub char_id: u32,
    pub name: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeaderRoot {
    pub leader_addr: u64,
    pub country_tag_db_id: u32,
    pub leader_kind: u8,
    pub name: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitDefinitionRoot {
    pub trait_addr: u64,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryLeaderRoleRoot {
    pub char_id: u32,
    pub country_leader_addr: u64,
    pub name: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryPoliticsRoot {
    pub country_addr: u64,
    pub country_tag_db_id: u32,
    pub ruling_party: Option<String>,
    pub ruler_name: Option<String>,
    pub ruler_token: Option<String>,
    pub active_idea_tokens: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvisorRoot {
    pub advisor_addr: u64,
    pub idea_token: String,
    pub slot: Option<String>,
    pub ledger: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterPortraitInput {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub role: String,
    pub size: String,
    pub gfx: String,
    pub gfx_encoding: compact::PortraitIdentityEncoding,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdapterBuildStats {
    pub character_roots_seen: u32,
    pub character_rows: u32,
    pub deduplicated_character_rows: u32,
    pub ambiguous_character_rows: u32,
    pub unit_leader_roots_seen: u32,
    pub unit_leader_rows: u32,
    pub unit_leader_trait_rows: u32,
    pub country_leader_roots_seen: u32,
    pub country_leader_role_rows: u32,
    pub country_roots_seen: u32,
    pub active_ruler_rows: u32,
    pub active_ruler_unresolved_roots: u32,
    pub active_ruler_absent_roots: u32,
    pub advisor_roots_seen: u32,
    pub advisor_roster_rows: u32,
    pub appointed_advisor_rows: u32,
    pub portrait_inputs_seen: u32,
    pub portrait_rows: u32,
    pub rejected_roots: u32,
    pub identity_mismatch_rows: u32,
    pub identity_degraded_rows: u32,
    pub missing_trait_tokens: u32,
    pub dead_roots: u32,
    pub walk_characters_us: u32,
    pub walk_unit_leaders_us: u32,
    pub walk_country_leader_roles_us: u32,
    pub walk_active_rulers_us: u32,
    pub walk_advisors_us: u32,
    pub walk_appointed_advisors_us: u32,
    pub walk_portraits_us: u32,
    /// Lane PH: how many active-idea tokens this frame's country rows carried
    /// in total. It is the ONE knob that decides how the appointed-advisor
    /// join behaves — the membership test is per country slice, so its cost
    /// per roster row turns on the slice length — and no capture recorded it
    /// before, which left the join's price a guess. Sums per frame; divide by
    /// `country_roots_seen` for tokens per country.
    pub active_idea_tokens_seen: u32,
    /// Lane PR: how many roster rows the appointed-advisor join found a
    /// COUNTRY for. It splits the two ways that join can return nothing.
    ///
    /// `appointed_advisor_rows` was 0 across every frame of two independent
    /// captures while `active_idea_tokens_seen` measured 2,891 tokens a frame,
    /// so the join was answering "no" to every row and nothing recorded WHICH
    /// half of the key missed. This counts the rows that got past the
    /// `country_tag_db_id` lookup and reached the token test: 0 means the two
    /// sides do not agree on the country key at all; `advisor_roster_rows`
    /// means every row found its country and the token is what did not meet.
    pub appointed_join_country_tag_hits: u32,
    /// Lane PR: the first country whose active ruler this frame could not
    /// resolve, and the reader decision that refused it
    /// ([`extract_error_reason`]). `active_ruler_unresolved_roots` counted
    /// those refusals without naming one, and a refusal that costs the whole
    /// ACTIVE_RULER family its frame has to say what it was — the same
    /// "degraded outcomes name the cause" rule the rest of the lane follows.
    /// Both are 0 when no root was refused; `..._reason` is 0 only then,
    /// because [`extract_error_reason`] never returns 0.
    pub active_ruler_unresolved_first_tag: u32,
    pub active_ruler_unresolved_first_reason: u32,
    /// Lane PR: the two scalars the refusing comparison held, when it had two
    /// ([`extract_error_operands`]). The reason code says the SHAPE of the
    /// refusal and there are three places on the active-ruler chain that raise
    /// the same shape; these say WHICH, because each compares against a
    /// different constant. Both 0 when the refusal carried no operands.
    pub active_ruler_unresolved_first_expected: u32,
    pub active_ruler_unresolved_first_observed: u32,
}

/// Lane PR: the `(expected, observed)` pair of a refusal that compared two
/// 32-bit scalars, or `(0, 0)` when it compared none.
///
/// kills: collapsing this to `(0, 0)` would leave
/// `..._first_reason` = 6 (`BadTypeId`) unable to say whether the active-ruler
/// chain refused at `CCharacter.type` or at the country-tag echo, which are a
/// broken object and a leader owned by another country — different faults with
/// different fixes.
pub fn extract_error_operands(error: &ExtractError) -> (u32, u32) {
    match error {
        ExtractError::BadTypeId {
            expected, observed, ..
        }
        | ExtractError::IdentityMismatch {
            expected, observed, ..
        } => (*expected, *observed),
        _ => (0, 0),
    }
}

/// Lane PR: a small stable code per [`ExtractError`] shape, for counters that
/// have to name a refusal in one number. Never 0, so a counter pair can use 0
/// for "nothing was refused".
///
/// kills: returning a constant would make
/// `domain47_adapter_active_ruler_unresolved_first_reason` unable to tell a
/// null political-status pointer from an ownership-echo mismatch, which is
/// the whole reason the word exists.
pub fn extract_error_reason(error: &ExtractError) -> u32 {
    match error {
        ExtractError::RegistryMismatch(_) => 1,
        ExtractError::AddressOverflow(_) => 2,
        ExtractError::NullPointer(_) => 3,
        ExtractError::MissingRead { .. } => 4,
        ExtractError::BadVtable { .. } => 5,
        ExtractError::BadTypeId { .. } => 6,
        ExtractError::BadVector { .. } => 7,
        ExtractError::IdentityMismatch { .. } => 8,
        ExtractError::EmptyLeaderVector => 9,
    }
}

fn walk_phase_us(since: std::time::Instant) -> u32 {
    since.elapsed().as_micros().min(u32::MAX as u128) as u32
}

impl AdapterBuildStats {
    pub fn has_rejections(&self) -> bool {
        self.rejected_roots != 0
            || self.missing_trait_tokens != 0
            || self.dead_roots != 0
            || self.identity_mismatch_rows != 0
    }
}

enum RootFailure {
    Dead,
    Torn,
}

fn classify_root_failure<T, E: PartialEq>(first: &E, second: &Result<T, E>) -> RootFailure {
    match second {
        Err(e) if e == first => RootFailure::Dead,
        _ => RootFailure::Torn,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterBuildResult {
    pub frame: LaneFrame,
    pub stats: AdapterBuildStats,
}

pub const CACHEABLE_WALK_FAMILIES: u32 = compact::FAMILY_CHARACTER
    | compact::FAMILY_UNIT_LEADER
    | compact::FAMILY_UNIT_LEADER_TRAIT
    | compact::FAMILY_COUNTRY_LEADER_ROLE
    | compact::FAMILY_ADVISOR_ROSTER;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CachedWalk {
    pub families: u32,
    pub characters: Vec<CharacterRecord>,
    pub unit_leaders: Vec<UnitLeaderRecord>,
    pub unit_leader_traits: Vec<UnitLeaderTraitRecord>,
    pub country_leader_roles: Vec<CountryLeaderRoleRecord>,
    pub advisor_roster: Vec<AdvisorRecord>,
    pub character_portraits: Vec<CharacterPortraitRecord>,
    pub known_keys: Vec<(u32, u32)>,
    pub character_identity: Vec<((u32, u32), u32)>,
    pub stats: AdapterBuildStats,
    /// Lane PH: the portrait phase's answer, precomputed once at adoption.
    ///
    /// The phase reads NO engine memory (`mem.walk_family` is a witness-tap
    /// mark, not a read), so nothing about it is licensed by the witness — the
    /// witness "covers" it only because the record range between its mark and
    /// the end of the log is EMPTY. What it is really a function of is
    /// `input.portraits` (a root-cache list), `known_keys` (this cache), and
    /// `known_characters` (this frame's active rulers). Only the last varies
    /// between the adopting frame and the frames that reuse it, because the
    /// root cache cannot be refreshed while this cache is alive: every arm of
    /// a full discovery either seals a NEW witness or sets `cache.witness =
    /// None`, and this walk is dropped with the witness that carries it. So
    /// the two portrait-dependent quantities are fixed at adoption:
    ///
    /// * `portrait_unknown_keys` — the deduplicated, sorted keys of input
    ///   portraits this cache does NOT know. `live_only_admitted` is exactly
    ///   "one of these is in `known_characters`".
    /// * `portrait_unknown_rows` — how many input ROWS carried such a key
    ///   (a key repeats across roles and sizes), which is what those rows
    ///   contribute to `rejected_roots` when `live_only_admitted` is false.
    ///
    /// `portrait_input_rows` records the list length the precompute was taken
    /// over. It is belt-and-braces on top of the frozen-roots argument above:
    /// a reuse against a list of some other length falls back to the scan.
    pub portrait_index: Option<CachedPortraitIndex>,
}

/// Lane PH: see [`CachedWalk::portrait_index`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CachedPortraitIndex {
    pub portrait_input_rows: u32,
    pub portrait_unknown_keys: Vec<(u32, u32)>,
    pub portrait_unknown_rows: u32,
}

impl CachedWalk {
    pub fn serves(&self, family: u32) -> bool {
        self.families & family != 0
    }

    pub fn knows(&self, key: &(u32, u32)) -> bool {
        self.known_keys.binary_search(key).is_ok()
    }

    pub fn character(&self, key: &(u32, u32)) -> Option<&CharacterRecord> {
        self.character_identity
            .binary_search_by_key(key, |(k, _)| *k)
            .ok()
            .and_then(|i| self.characters.get(self.character_identity[i].1 as usize))
    }
}

pub fn build_lane_frame_from_adapter_input<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    input: &AdapterInput,
) -> AdapterBuildResult {
    build_lane_frame_from_adapter_input_ref(mem, roots, &input.as_ref())
}

pub fn build_lane_frame_from_adapter_input_ref<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    input: &AdapterInputRef<'_>,
) -> AdapterBuildResult {
    build_lane_frame_inner(mem, roots, input, None)
}

pub fn build_lane_frame_reusing_walk<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    input: &AdapterInputRef<'_>,
    cached: &CachedWalk,
) -> AdapterBuildResult {
    build_lane_frame_inner(mem, roots, input, Some(cached))
}

fn build_lane_frame_inner<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    input: &AdapterInputRef<'_>,
    cached: Option<&CachedWalk>,
) -> AdapterBuildResult {
    let served = |family: u32| cached.map_or(false, |c| c.serves(family));
    let mut frame = LaneFrame::new(input.elapsed_wall_ms);
    frame.status_flags = input.status_flags;
    let mut stats = AdapterBuildStats::default();
    let trait_tokens = input
        .unit_leader_trait_tokens
        .iter()
        .map(|row| (row.trait_addr, row.token.as_str()))
        .collect::<BTreeMap<_, _>>();

    let track_known_characters = !input.portraits.is_empty();
    let mut known_characters = BTreeSet::new();
    let mut character_identity = BTreeMap::<(u32, u32), usize>::new();
    let mut characters_by_id = BTreeMap::<u32, CharacterRecord>::new();
    let mut ambiguous_character_ids = BTreeSet::new();
    stats.character_roots_seen = input.characters.len().min(u32::MAX as usize) as u32;
    let phase_started = std::time::Instant::now();
    mem.walk_family(compact::FAMILY_CHARACTER);
    if let Some(cached) = cached.filter(|c| c.serves(compact::FAMILY_CHARACTER)) {
        stats.character_rows = cached.stats.character_rows;
        stats.deduplicated_character_rows = cached.stats.deduplicated_character_rows;
        stats.ambiguous_character_rows = cached.stats.ambiguous_character_rows;
        stats.identity_degraded_rows = cached.stats.identity_degraded_rows;
    }
    for root in input
        .characters
        .iter()
        .filter(|_| !served(compact::FAMILY_CHARACTER))
    {
        match read_character_raw_fields(mem, roots, root.character_addr) {
            Ok(raw) => {
                let identity_matches = raw.char_id == root.char_id;
                if !identity_matches {
                    stats.identity_degraded_rows = stats.identity_degraded_rows.saturating_add(1);
                }
                if ambiguous_character_ids.contains(&raw.char_id) {
                    stats.ambiguous_character_rows =
                        stats.ambiguous_character_rows.saturating_add(1);
                    continue;
                }
                let candidate = CharacterRecord {
                    country_tag_db_id: raw.country_tag_db_id,
                    char_id: raw.char_id,
                    name: identity_matches.then(|| root.name.clone()).flatten(),
                    token: identity_matches.then(|| root.token.clone()).flatten(),
                };
                match characters_by_id.entry(raw.char_id) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(candidate);
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry)
                        if entry.get().country_tag_db_id == raw.country_tag_db_id =>
                    {
                        let kept = entry.get_mut();
                        if kept.name != candidate.name {
                            kept.name = None;
                        }
                        if kept.token != candidate.token {
                            kept.token = None;
                        }
                        stats.deduplicated_character_rows =
                            stats.deduplicated_character_rows.saturating_add(1);
                    }
                    std::collections::btree_map::Entry::Occupied(entry) => {
                        entry.remove();
                        ambiguous_character_ids.insert(raw.char_id);
                        stats.ambiguous_character_rows =
                            stats.ambiguous_character_rows.saturating_add(2);
                    }
                }
            }
            Err(first) => match classify_root_failure(
                &first,
                &read_character_raw_fields(mem, roots, root.character_addr),
            ) {
                RootFailure::Dead => stats.dead_roots = stats.dead_roots.saturating_add(1),
                RootFailure::Torn => stats.rejected_roots = stats.rejected_roots.saturating_add(1),
            },
        }
    }
    frame.characters.extend(characters_by_id.into_values());
    if !served(compact::FAMILY_CHARACTER) {
        stats.character_rows = frame.characters.len().min(u32::MAX as usize) as u32;
    }
    for (index, row) in frame.characters.iter().enumerate() {
        let key = (row.country_tag_db_id, row.char_id);
        if track_known_characters {
            known_characters.insert(key);
        }
        character_identity.insert(key, index);
    }
    stats.walk_characters_us = walk_phase_us(phase_started);

    stats.unit_leader_roots_seen = input.unit_leaders.len().min(u32::MAX as usize) as u32;
    let phase_started = std::time::Instant::now();
    mem.walk_family(compact::FAMILY_UNIT_LEADER | compact::FAMILY_UNIT_LEADER_TRAIT);
    if let Some(cached) = cached.filter(|c| c.serves(compact::FAMILY_UNIT_LEADER)) {
        stats.unit_leader_rows = cached.stats.unit_leader_rows;
        stats.unit_leader_trait_rows = cached.stats.unit_leader_trait_rows;
        stats.missing_trait_tokens = cached.stats.missing_trait_tokens;
    }
    for root in input
        .unit_leaders
        .iter()
        .filter(|_| !served(compact::FAMILY_UNIT_LEADER))
    {
        match read_unit_leader(mem, roots, root, &trait_tokens) {
            Ok((leader, traits, missing_trait_tokens)) => {
                if track_known_characters {
                    known_characters.insert((leader.country_tag_db_id, leader.char_id));
                }
                frame.unit_leader_traits.extend(traits);
                frame.unit_leaders.push(leader);
                stats.missing_trait_tokens = stats
                    .missing_trait_tokens
                    .saturating_add(missing_trait_tokens);
            }
            Err(first) => match classify_root_failure(
                &first,
                &read_unit_leader(mem, roots, root, &trait_tokens),
            ) {
                RootFailure::Dead => stats.dead_roots = stats.dead_roots.saturating_add(1),
                RootFailure::Torn => stats.rejected_roots = stats.rejected_roots.saturating_add(1),
            },
        }
    }
    if !served(compact::FAMILY_UNIT_LEADER) {
        stats.unit_leader_rows = frame.unit_leaders.len().min(u32::MAX as usize) as u32;
        stats.unit_leader_trait_rows = frame.unit_leader_traits.len().min(u32::MAX as usize) as u32;
    }
    stats.walk_unit_leaders_us = walk_phase_us(phase_started);

    stats.country_leader_roots_seen =
        input.country_leader_roles.len().min(u32::MAX as usize) as u32;
    let phase_started = std::time::Instant::now();
    mem.walk_family(compact::FAMILY_COUNTRY_LEADER_ROLE);
    if let Some(cached) = cached.filter(|c| c.serves(compact::FAMILY_COUNTRY_LEADER_ROLE)) {
        stats.country_leader_role_rows = cached.stats.country_leader_role_rows;
        stats.identity_mismatch_rows = cached.stats.identity_mismatch_rows;
    }
    for root in input
        .country_leader_roles
        .iter()
        .filter(|_| !served(compact::FAMILY_COUNTRY_LEADER_ROLE))
    {
        match read_country_leader_role(mem, roots, root) {
            Ok(row) => {
                if track_known_characters {
                    known_characters.insert((row.country_tag_db_id, row.char_id));
                }
                frame.country_leader_roles.push(row);
            }
            Err(ExtractError::IdentityMismatch { .. }) => {
                stats.identity_mismatch_rows = stats.identity_mismatch_rows.saturating_add(1)
            }
            Err(first) => {
                match classify_root_failure(&first, &read_country_leader_role(mem, roots, root)) {
                    RootFailure::Dead => stats.dead_roots = stats.dead_roots.saturating_add(1),
                    RootFailure::Torn => {
                        stats.rejected_roots = stats.rejected_roots.saturating_add(1)
                    }
                }
            }
        }
    }
    if !served(compact::FAMILY_COUNTRY_LEADER_ROLE) {
        stats.country_leader_role_rows =
            frame.country_leader_roles.len().min(u32::MAX as usize) as u32;
    }
    stats.walk_country_leader_roles_us = walk_phase_us(phase_started);

    stats.country_roots_seen = input.countries.len().min(u32::MAX as usize) as u32;
    let phase_started = std::time::Instant::now();
    // Lane PH: NOT SERVED FROM THE CACHE, and the reason is a production
    // counter rather than a judgement. This phase does read engine memory —
    // the CCountry -> political status -> ruling party -> country-leader vector
    // -> CCountryLeader -> CCharacter chain, about ten guarded reads per
    // country — so unlike portraits and appointed advisors it WOULD need the
    // witness to vouch for it. The witness does not: over the whole oz_tip
    // capture `domain47_walk_covered_family_mask` is 225
    // (CHARACTER | ADVISOR_ROSTER | APPOINTED_ADVISOR | CHARACTER_PORTRAIT) and
    // FAMILY_ACTIVE_RULER (16) is clear in every one of the 35 seals, because
    // discovery never reads the country-politics chain and so never sealed a
    // byte of it. Nothing weaker would do either: the record also carries
    // `ruling_party`, `ruler_name` and `ruler_token` straight off this frame's
    // country row, which is the live half of d47 by design.
    mem.walk_family(compact::FAMILY_ACTIVE_RULER);
    let cached_characters = cached.filter(|c| c.serves(compact::FAMILY_CHARACTER));
    for root in input.countries {
        match read_active_ruler_from_country(
            mem,
            roots,
            root.country_tag_db_id,
            root.country_addr,
            root.ruling_party.clone(),
            root.ruler_name.clone(),
            root.ruler_token.clone(),
        ) {
            Ok(mut row) => {
                let key = (row.country_tag_db_id, row.char_id);
                let identity = match cached_characters {
                    Some(cached) => cached.character(&key),
                    None => character_identity.get(&key).map(|i| &frame.characters[*i]),
                };
                if let Some(identity) = identity {
                    if row.name.is_none() {
                        row.name = identity.name.clone();
                    }
                    if row.token.is_none() {
                        row.token = identity.token.clone();
                    }
                }
                if track_known_characters {
                    known_characters.insert((row.country_tag_db_id, row.char_id));
                }
                frame.active_rulers.push(row);
            }
            Err(ExtractError::EmptyLeaderVector) => {
                stats.active_ruler_absent_roots = stats.active_ruler_absent_roots.saturating_add(1)
            }
            Err(error) => {
                // Lane PR: NAME the first one. This refusal costs the whole
                // ACTIVE_RULER family its frame (the caller clears the rows and
                // declares the family absent), and until now the only trace it
                // left was a count, which cannot tell a torn read from a
                // structural mismatch that will refuse again every frame.
                if stats.active_ruler_unresolved_roots == 0 {
                    let (expected, observed) = extract_error_operands(&error);
                    stats.active_ruler_unresolved_first_tag = root.country_tag_db_id;
                    stats.active_ruler_unresolved_first_reason = extract_error_reason(&error);
                    stats.active_ruler_unresolved_first_expected = expected;
                    stats.active_ruler_unresolved_first_observed = observed;
                }
                stats.active_ruler_unresolved_roots =
                    stats.active_ruler_unresolved_roots.saturating_add(1)
            }
        }
    }
    stats.active_ruler_rows = frame.active_rulers.len().min(u32::MAX as usize) as u32;
    stats.walk_active_rulers_us = walk_phase_us(phase_started);

    stats.advisor_roots_seen = input.advisors.len().min(u32::MAX as usize) as u32;
    let phase_started = std::time::Instant::now();
    mem.walk_family(compact::FAMILY_ADVISOR_ROSTER);
    if let Some(cached) = cached.filter(|c| c.serves(compact::FAMILY_ADVISOR_ROSTER)) {
        stats.advisor_roster_rows = cached.stats.advisor_roster_rows;
    }
    for root in input
        .advisors
        .iter()
        .filter(|_| !served(compact::FAMILY_ADVISOR_ROSTER))
    {
        match read_advisor_raw_fields(mem, roots, root.advisor_addr) {
            Ok(raw) => {
                if track_known_characters {
                    known_characters.insert((raw.country_tag_db_id, raw.char_id));
                }
                let bucket = advisor_bucket(
                    root.slot.as_deref(),
                    root.ledger.as_deref(),
                    &root.idea_token,
                );
                frame.advisor_roster.push(AdvisorRecord {
                    country_tag_db_id: raw.country_tag_db_id,
                    char_id: raw.char_id,
                    slot: root.slot.clone(),
                    ledger: Some(bucket.as_str().to_string()),
                    idea_token: root.idea_token.clone(),
                });
            }
            Err(first) => match classify_root_failure(
                &first,
                &read_advisor_raw_fields(mem, roots, root.advisor_addr),
            ) {
                RootFailure::Dead => stats.dead_roots = stats.dead_roots.saturating_add(1),
                RootFailure::Torn => stats.rejected_roots = stats.rejected_roots.saturating_add(1),
            },
        }
    }
    if !served(compact::FAMILY_ADVISOR_ROSTER) {
        stats.advisor_roster_rows = frame.advisor_roster.len().min(u32::MAX as usize) as u32;
    }
    stats.walk_advisors_us = walk_phase_us(phase_started);
    let phase_started = std::time::Instant::now();
    mem.walk_family(compact::FAMILY_APPOINTED_ADVISOR);
    let roster: &[AdvisorRecord] = match cached.filter(|c| c.serves(compact::FAMILY_ADVISOR_ROSTER))
    {
        Some(cached) => &cached.advisor_roster,
        None => &frame.advisor_roster,
    };
    stats.active_idea_tokens_seen = input
        .countries
        .iter()
        .map(|country| country.active_idea_tokens.len())
        .sum::<usize>()
        .min(u32::MAX as usize) as u32;
    let join = appointed_advisors_from_country_active_ideas_counted(roster, input.countries);
    frame.appointed_advisors = join.rows;
    stats.appointed_join_country_tag_hits = join.country_tag_hits;
    stats.appointed_advisor_rows = frame.appointed_advisors.len().min(u32::MAX as usize) as u32;
    stats.walk_appointed_advisors_us = walk_phase_us(phase_started);

    stats.portrait_inputs_seen = input.portraits.len().min(u32::MAX as usize) as u32;
    let phase_started = std::time::Instant::now();
    mem.walk_family(compact::FAMILY_CHARACTER_PORTRAIT);
    let (portrait_rejections, _portrait_arm) = portrait_phase(
        input.portraits,
        cached,
        &known_characters,
        &mut frame.character_portraits,
    );
    stats.rejected_roots = stats.rejected_roots.saturating_add(portrait_rejections);
    stats.portrait_rows = if frame.character_portraits.is_empty() {
        cached.map_or(0, |c| c.character_portraits.len())
    } else {
        frame.character_portraits.len()
    }
    .min(u32::MAX as usize) as u32;
    stats.walk_portraits_us = walk_phase_us(phase_started);

    if stats.has_rejections() {
        frame.status_flags |= compact::STATUS_PARTIAL_DATA;
    }
    if input.skipped_input_rows != 0 {
        frame.status_flags |= compact::STATUS_PARTIAL_DATA;
    }
    if stats.identity_degraded_rows != 0 {
        frame.status_flags |= compact::STATUS_PARTIAL_DATA | compact::STATUS_SOURCE_UNAVAILABLE;
    }
    if input.countries.is_empty() {
        frame.status_flags |=
            compact::STATUS_BRIDGE_UNAVAILABLE | compact::STATUS_SOURCE_UNAVAILABLE;
    }
    if input.characters.is_empty()
        && input.unit_leaders.is_empty()
        && input.country_leader_roles.is_empty()
        && input.advisors.is_empty()
    {
        frame.status_flags |= compact::STATUS_SOURCE_UNAVAILABLE;
    }

    AdapterBuildResult { frame, stats }
}

/// Lane PH: the whole `character_portraits` walk phase, admitting into `out`
/// and returning what it contributes to `rejected_roots`.
///
/// It reads no engine memory, so — like the appointed-advisor join below — it
/// is not a candidate for the witness-licensed walk cache. What makes it
/// cheap instead is that the only per-frame input is `known_characters`: see
/// [`CachedWalk::portrait_index`] for why the portrait list and the cache's
/// known keys are both fixed for as long as a cached walk is alive, and for
/// what the precomputed index therefore stands in for.
/// Which arm [`portrait_phase`] took. Returned so a test can require the O(1)
/// arm to have RUN rather than infer it from the fixture: the two arms agree on
/// every output by construction, so without this a mutation that deleted the
/// fast return and always scanned would keep every equivalence assertion green
/// while restoring the whole per-row cost (Codex round 1, finding 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortraitPhaseArm {
    /// Answered from `CachedWalk::portrait_index` without touching a row.
    Index,
    /// Walked every input row.
    Scan,
}

fn portrait_phase(
    portraits: &[CharacterPortraitInput],
    cached: Option<&CachedWalk>,
    known_characters: &BTreeSet<(u32, u32)>,
    out: &mut Vec<CharacterPortraitRecord>,
) -> (u32, PortraitPhaseArm) {
    // The index answers `live_only_admitted` from the unknown-key set alone —
    // in the steady state that set is EMPTY, so the whole question costs
    // nothing instead of a `knows` binary search over every input row. The
    // length test is the belt-and-braces half of the licence.
    let index = cached
        .and_then(|c| c.portrait_index.as_ref())
        .filter(|index| index.portrait_input_rows as usize == portraits.len());
    let live_only_admitted = match index {
        Some(index) => index
            .portrait_unknown_keys
            .iter()
            .any(|key| known_characters.contains(key)),
        None => cached.map_or(false, |c| {
            portraits.iter().any(|portrait| {
                let key = (portrait.country_tag_db_id, portrait.char_id);
                !c.knows(&key) && known_characters.contains(&key)
            })
        }),
    };
    // Every input row splits into one of two cases, and BOTH are settled
    // without touching the row. A row whose key this cache knows would take
    // the `continue` below, because `live_only_admitted` is false. A row whose
    // key it does not know is in `portrait_unknown_keys`, and
    // `live_only_admitted == false` says no key in that set is in
    // `known_characters` — so `admit` is false for it and it is rejected.
    // Nothing is pushed, which is what leaves `stats.portrait_rows` reading
    // the cached count exactly as the scan leaves it.
    if let Some(index) = index.filter(|_| !live_only_admitted) {
        return (index.portrait_unknown_rows, PortraitPhaseArm::Index);
    }
    let mut rejected = 0u32;
    let admit = |key: &(u32, u32)| -> bool {
        known_characters.contains(key) || cached.map_or(false, |c| c.knows(key))
    };
    for portrait in portraits {
        let key = (portrait.country_tag_db_id, portrait.char_id);
        let cached_key = cached.map_or(false, |c| c.knows(&key));
        if cached_key && !live_only_admitted {
            continue;
        }
        if admit(&key) {
            out.push(CharacterPortraitRecord {
                country_tag_db_id: portrait.country_tag_db_id,
                char_id: portrait.char_id,
                role: portrait.role.clone(),
                size: portrait.size.clone(),
                gfx: portrait.gfx.clone(),
                gfx_encoding: portrait.gfx_encoding,
            });
        } else {
            rejected = rejected.saturating_add(1);
        }
    }
    (rejected, PortraitPhaseArm::Scan)
}

/// Lane PH: this is the whole `appointed_advisors` walk phase — it reads no
/// engine memory, so it is not a candidate for the witness-licensed walk cache.
/// It is a pure function of the roster and of THIS frame's country rows, and
/// the country rows are the live half of d47 (a coup or an election has to be
/// caught the frame it happens), so it is recomputed every frame by design and
/// only its cost is negotiable.
///
/// The membership test is a LINEAR SCAN of the country's own token slice
/// instead of a `BTreeSet<&str>` built per country. The old form paid one heap
/// node per active idea per country per frame to answer at most one `contains`
/// per roster row, and that construction — not the lookup — was the phase's
/// cost: at oz_tip's 439 countries and 8,615 roster rows the arena form is
/// 74% cheaper at 16 active ideas per country and 68% cheaper at 63
/// (`runs/ph_d47cache/ph_profile_appointed_sweep.log`).
///
/// Bisecting a sorted arena was measured and NOT taken: sorting each country's
/// slice costs more than every scan it saves, and the bisect arm only reached
/// PARITY with the old form (+0.5% at 64 ideas per country, +0.4% at 192)
/// where the scan was three times faster at 63. A scan and a `BTreeSet` are
/// the same membership question over the same tokens, so nothing about the
/// join's answer turns on which is used.
///
/// `seen` borrows `idea_token` instead of cloning it; `String` orders exactly
/// as `str`, so the deduplication is unchanged.
pub fn appointed_advisors_from_country_active_ideas(
    roster: &[AdvisorRecord],
    countries: &[CountryPoliticsRoot],
) -> Vec<AdvisorRecord> {
    appointed_advisors_from_country_active_ideas_counted(roster, countries).rows
}

/// The join's rows plus the ONE number that says which half of its key met.
pub struct AppointedAdvisorJoin {
    pub rows: Vec<AdvisorRecord>,
    /// Roster rows whose `country_tag_db_id` named a country in this frame's
    /// country rows — i.e. rows that got as far as the token test. See
    /// [`AdapterBuildStats::appointed_join_country_tag_hits`].
    pub country_tag_hits: u32,
}

pub fn appointed_advisors_from_country_active_ideas_counted(
    roster: &[AdvisorRecord],
    countries: &[CountryPoliticsRoot],
) -> AppointedAdvisorJoin {
    let mut arena: Vec<&str> = Vec::with_capacity(
        countries
            .iter()
            .map(|country| country.active_idea_tokens.len())
            .sum(),
    );
    // Insertion order, so a repeated tag resolves to its LAST row exactly as
    // collecting into a `BTreeMap` did.
    let mut active_by_country = BTreeMap::<u32, (usize, usize)>::new();
    for country in countries {
        let start = arena.len();
        arena.extend(country.active_idea_tokens.iter().map(String::as_str));
        active_by_country.insert(country.country_tag_db_id, (start, arena.len()));
    }
    let mut seen = BTreeSet::new();
    let mut country_tag_hits = 0u32;
    let rows = roster
        .iter()
        .filter(|row| {
            active_by_country
                .get(&row.country_tag_db_id)
                .map(|&(start, end)| {
                    country_tag_hits = country_tag_hits.saturating_add(1);
                    let token = row.idea_token.as_str();
                    arena[start..end].iter().any(|held| *held == token)
                })
                .unwrap_or(false)
        })
        .filter(|row| seen.insert((row.country_tag_db_id, row.char_id, row.idea_token.as_str())))
        .cloned()
        .collect();
    AppointedAdvisorJoin {
        rows,
        country_tag_hits,
    }
}

fn read_country_leader_role<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    root: &CountryLeaderRoleRoot,
) -> Result<CountryLeaderRoleRecord, ExtractError> {
    let character_addr = read_vtable_and_character_ptr::<M, CCOUNTRY_LEADER_RAW_FIELDS_SPAN>(
        mem,
        root.country_leader_addr,
        roots.ccountry_leader_vtable,
        "CCountryLeader",
        CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
        "CCountryLeader.character",
    )?;
    let character = read_character_raw_fields(mem, roots, character_addr)?;
    if character.char_id != root.char_id {
        return Err(ExtractError::IdentityMismatch {
            addr: character_addr,
            field: "CCountryLeader.character.char_id",
            expected: root.char_id,
            observed: character.char_id,
        });
    }
    Ok(CountryLeaderRoleRecord {
        country_tag_db_id: character.country_tag_db_id,
        char_id: character.char_id,
        name: root.name.clone(),
        token: root.token.clone(),
    })
}

fn read_unit_leader<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    root: &UnitLeaderRoot,
    trait_tokens: &BTreeMap<u64, &str>,
) -> Result<(UnitLeaderRecord, Vec<UnitLeaderTraitRecord>, u32), ExtractError> {
    let expected_vtable = match root.leader_kind {
        UNIT_LEADER_ARMY => roots.carmy_leader_vtable,
        UNIT_LEADER_NAVY => roots.cnavy_leader_vtable,
        _ => 0,
    };
    if expected_vtable == 0 {
        return Err(ExtractError::BadTypeId {
            addr: root.leader_addr,
            field: "CUnitLeader.kind",
            expected: UNIT_LEADER_ARMY as u32,
            observed: root.leader_kind as u32,
        });
    }
    require_vtable(mem, root.leader_addr, expected_vtable, "CUnitLeader")?;
    require_type_id(
        mem,
        root.leader_addr
            .checked_add(CUNIT_LEADER_TYPE_OFF)
            .ok_or(ExtractError::AddressOverflow("CUnitLeader.type"))?,
        CUNIT_LEADER_TYPE_ID,
        "CUnitLeader.type",
    )?;
    let unit_leader_id = read_u32(mem, root.leader_addr, CUNIT_LEADER_ID_OFF, "CUnitLeader.id")?;
    let char_id = read_u32(
        mem,
        root.leader_addr,
        CUNIT_LEADER_CHARACTER_ID_COPY_OFF,
        "CUnitLeader.character_id_copy",
    )?;
    let skill = read_unit_leader_skill(mem, root.leader_addr)?;
    let experience_raw = mem
        .read_i32_le(
            root.leader_addr
                .checked_add(CUNIT_LEADER_EXPERIENCE_RAW_OFF)
                .ok_or(ExtractError::AddressOverflow("CUnitLeader.experience_raw"))?,
        )
        .ok_or(ExtractError::MissingRead {
            addr: root.leader_addr + CUNIT_LEADER_EXPERIENCE_RAW_OFF,
            field: "CUnitLeader.experience_raw",
        })?;
    let mut trait_rows = Vec::new();
    let mut missing_trait_tokens = 0u32;
    if roots.cunit_leader_trait_vtable != 0 && !trait_tokens.is_empty() {
        let trait_vector = read_unit_leader_trait_vector(mem, root.leader_addr)?;
        for (ordinal, trait_addr) in trait_vector.trait_ptrs.iter().copied().enumerate() {
            require_vtable(
                mem,
                trait_addr,
                roots.cunit_leader_trait_vtable,
                "CUnitLeaderTrait",
            )?;
            if let Some(token) = trait_tokens.get(&trait_addr) {
                trait_rows.push(UnitLeaderTraitRecord {
                    unit_leader_id,
                    ordinal: ordinal.min(u16::MAX as usize) as u16,
                    trait_token: (*token).to_string(),
                });
            } else {
                missing_trait_tokens = missing_trait_tokens.saturating_add(1);
            }
        }
    }
    let stat_offsets = stat_offsets(root.leader_kind);
    let leader = UnitLeaderRecord {
        country_tag_db_id: root.country_tag_db_id,
        unit_leader_id,
        char_id,
        leader_kind: root.leader_kind,
        skill,
        experience_raw: Some(experience_raw),
        attack: read_optional_u8(mem, root.leader_addr, stat_offsets.attack),
        defense: read_optional_u8(mem, root.leader_addr, stat_offsets.defense),
        planning: read_optional_u8(mem, root.leader_addr, stat_offsets.planning),
        logistics: read_optional_u8(mem, root.leader_addr, stat_offsets.logistics),
        maneuvering: read_optional_u8(mem, root.leader_addr, stat_offsets.maneuvering),
        coordination: read_optional_u8(mem, root.leader_addr, stat_offsets.coordination),
        token: root.token.clone(),
        name: root.name.clone(),
        trait_count: trait_rows.len().min(u16::MAX as usize) as u16,
    };
    Ok((leader, trait_rows, missing_trait_tokens))
}

#[derive(Clone, Copy)]
struct StatOffsets {
    attack: Option<u64>,
    defense: Option<u64>,
    planning: Option<u64>,
    logistics: Option<u64>,
    maneuvering: Option<u64>,
    coordination: Option<u64>,
}

fn stat_offsets(leader_kind: u8) -> StatOffsets {
    match leader_kind {
        UNIT_LEADER_ARMY => StatOffsets {
            attack: Some(CARMY_LEADER_ATTACK_OFF),
            defense: Some(CARMY_LEADER_DEFENSE_OFF),
            planning: Some(CARMY_LEADER_PLANNING_OFF),
            logistics: Some(CARMY_LEADER_LOGISTICS_OFF),
            maneuvering: None,
            coordination: None,
        },
        UNIT_LEADER_NAVY => StatOffsets {
            attack: Some(CNAVY_LEADER_ATTACK_OFF),
            defense: Some(CNAVY_LEADER_DEFENSE_OFF),
            planning: None,
            logistics: None,
            maneuvering: Some(CNAVY_LEADER_MANEUVERING_OFF),
            coordination: Some(CNAVY_LEADER_COORDINATION_OFF),
        },
        _ => StatOffsets {
            attack: None,
            defense: None,
            planning: None,
            logistics: None,
            maneuvering: None,
            coordination: None,
        },
    }
}

fn read_optional_u8<M: LaneMemory>(mem: &M, base: u64, offset: Option<u64>) -> Option<u8> {
    mem.read_u8(base.checked_add(offset?)?)
}

fn require_vtable<M: LaneMemory>(
    mem: &M,
    addr: u64,
    expected: u64,
    field: &'static str,
) -> Result<(), ExtractError> {
    let observed = mem
        .read_u64_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })?;
    if observed != expected {
        return Err(ExtractError::BadVtable {
            addr,
            field,
            expected,
            observed,
        });
    }
    Ok(())
}

fn require_type_id<M: LaneMemory>(
    mem: &M,
    addr: u64,
    expected: u32,
    field: &'static str,
) -> Result<(), ExtractError> {
    let observed = mem
        .read_u32_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })?;
    if observed != expected {
        return Err(ExtractError::BadTypeId {
            addr,
            field,
            expected,
            observed,
        });
    }
    Ok(())
}

fn read_u32<M: LaneMemory>(
    mem: &M,
    base: u64,
    offset: u64,
    field: &'static str,
) -> Result<u32, ExtractError> {
    let addr = base
        .checked_add(offset)
        .ok_or(ExtractError::AddressOverflow(field))?;
    mem.read_u32_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })
}

#[cfg(test)]
#[path = "walk_phase_ph_tests.rs"]
mod walk_phase_ph_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::{
        self, CADVISOR_CHARACTER_PTR_OFF, CCHARACTER_COUNTRY_TAG_DB_ID_OFF, CCHARACTER_ID_OFF,
        CCHARACTER_TYPE_ID, CCHARACTER_TYPE_OFF, CCHARACTER_UNIT_LEADER_PTR_OFF,
        CCOUNTRY_POLITICAL_STATUS_OFF, CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF,
        CPOLITICAL_PARTY_LEADER_VEC_OFF, CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF,
        CPOLITICAL_STATUS_RULING_PARTY_OFF, CUNIT_LEADER_SKILL_SOURCE_PTR_OFF,
        CUNIT_LEADER_SKILL_VALUE_OFF, CUNIT_LEADER_TRAIT_CAPACITY_OFF, CUNIT_LEADER_TRAIT_DATA_OFF,
        CUNIT_LEADER_TRAIT_SIZE_OFF,
    };

    const CHAR_VT: u64 = 0x1000_1000;
    const ARMY_VT: u64 = 0x1000_2000;
    const NAVY_VT: u64 = 0x1000_3000;
    const COUNTRY_LEADER_VT: u64 = 0x1000_4000;
    const ADVISOR_VT: u64 = 0x1000_5000;
    const TRAIT_VT: u64 = 0x1000_6000;

    #[derive(Default)]
    struct FakeMemory {
        bytes: BTreeMap<u64, u8>,
    }

    impl FakeMemory {
        fn write_u8(&mut self, addr: u64, value: u8) {
            self.bytes.insert(addr, value);
        }

        fn write_u32(&mut self, addr: u64, value: u32) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_i32(&mut self, addr: u64, value: i32) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_u64(&mut self, addr: u64, value: u64) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_bytes(&mut self, addr: u64, bytes: &[u8]) {
            for (idx, byte) in bytes.iter().enumerate() {
                self.bytes.insert(addr + idx as u64, *byte);
            }
        }
    }

    impl LaneMemory for FakeMemory {
        fn read_u8(&self, addr: u64) -> Option<u8> {
            self.bytes.get(&addr).copied()
        }
    }

    fn roots() -> ResolvedTypeRoots {
        ResolvedTypeRoots {
            ccharacter_vtable: CHAR_VT,
            carmy_leader_vtable: ARMY_VT,
            cnavy_leader_vtable: NAVY_VT,
            ccountry_leader_vtable: COUNTRY_LEADER_VT,
            cadvisor_vtable: ADVISOR_VT,
            cunit_leader_trait_vtable: TRAIT_VT,
        }
    }

    fn write_character(mem: &mut FakeMemory, addr: u64, char_id: u32, tag_db_id: u32) {
        mem.write_u64(addr, CHAR_VT);
        mem.write_u32(addr + CCHARACTER_TYPE_OFF, CCHARACTER_TYPE_ID);
        mem.write_u32(addr + CCHARACTER_ID_OFF, char_id);
        mem.write_u32(addr + CCHARACTER_COUNTRY_TAG_DB_ID_OFF, tag_db_id);
        mem.write_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF, 0x2000);
    }

    #[test]
    fn adapter_builds_full_lane_frame_from_valid_roots() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        let leader = 0x2000;
        let skill_source = 0x2100;
        let trait_data = 0x2200;
        let trait_def = 0x2300;
        let country_leader = 0x3000;
        let country = 0x4000;
        let status = 0x4100;
        let party = 0x4200;
        let leader_vec = 0x4300;
        let advisor = 0x5000;

        write_character(&mut mem, character, 100, 2);
        mem.write_u64(leader, ARMY_VT);
        mem.write_u32(leader + CUNIT_LEADER_TYPE_OFF, CUNIT_LEADER_TYPE_ID);
        mem.write_u32(leader + CUNIT_LEADER_ID_OFF, 500);
        mem.write_u32(leader + CUNIT_LEADER_CHARACTER_ID_COPY_OFF, 100);
        mem.write_i32(leader + CUNIT_LEADER_EXPERIENCE_RAW_OFF, 42_000);
        mem.write_u64(leader + CUNIT_LEADER_SKILL_SOURCE_PTR_OFF, skill_source);
        mem.write_u32(skill_source + CUNIT_LEADER_SKILL_VALUE_OFF, 4);
        mem.write_u8(leader + CARMY_LEADER_ATTACK_OFF, 6);
        mem.write_u8(leader + CARMY_LEADER_DEFENSE_OFF, 7);
        mem.write_u8(leader + CARMY_LEADER_PLANNING_OFF, 5);
        mem.write_u8(leader + CARMY_LEADER_LOGISTICS_OFF, 8);
        mem.write_u64(leader + CUNIT_LEADER_TRAIT_DATA_OFF, trait_data);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_CAPACITY_OFF, 1);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_SIZE_OFF, 1);
        mem.write_u64(trait_data, trait_def);
        mem.write_u64(trait_def, TRAIT_VT);

        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            character,
        );
        mem.write_u64(country + CCOUNTRY_POLITICAL_STATUS_OFF, status);
        mem.write_u64(status + CPOLITICAL_STATUS_RULING_PARTY_OFF, party);
        mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF, leader_vec);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF, 1);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF, 1);
        mem.write_u64(leader_vec, country_leader);

        mem.write_u64(advisor, ADVISOR_VT);
        mem.write_u64(advisor + CADVISOR_CHARACTER_PTR_OFF, character);

        let input = AdapterInput {
            elapsed_wall_ms: 77,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 100,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            unit_leaders: vec![UnitLeaderRoot {
                leader_addr: leader,
                country_tag_db_id: 2,
                leader_kind: UNIT_LEADER_ARMY,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            unit_leader_trait_tokens: vec![TraitDefinitionRoot {
                trait_addr: trait_def,
                token: "old_guard".to_string(),
            }],
            country_leader_roles: vec![CountryLeaderRoleRoot {
                country_leader_addr: country_leader,
                char_id: 100,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            countries: vec![
                CountryPoliticsRoot {
                    country_addr: country,
                    country_tag_db_id: 2,
                    ruling_party: Some("democratic".to_string()),
                    ruler_name: Some("Maxwell Aitken".to_string()),
                    ruler_token: Some("ENG_maxwell_aitken".to_string()),
                    active_idea_tokens: vec!["ENG_maxwell_aitken".to_string()],
                },
                CountryPoliticsRoot {
                    country_addr: country,
                    country_tag_db_id: 3,
                    ruling_party: Some("communism".to_string()),
                    ruler_name: None,
                    ruler_token: None,
                    active_idea_tokens: vec!["OTHER_shared_token".to_string()],
                },
                CountryPoliticsRoot {
                    country_addr: 0xDEAD_0000,
                    country_tag_db_id: 4,
                    ruling_party: None,
                    ruler_name: None,
                    ruler_token: None,
                    active_idea_tokens: vec![],
                },
            ],
            advisors: vec![AdvisorRoot {
                advisor_addr: advisor,
                idea_token: "ENG_maxwell_aitken".to_string(),
                slot: Some("political_advisor".to_string()),
                ledger: None,
            }],
            portraits: vec![CharacterPortraitInput {
                country_tag_db_id: 2,
                char_id: 100,
                role: "civilian".to_string(),
                size: "large".to_string(),
                gfx: "GFX_portrait_ENG_maxwell_aitken".to_string(),
                gfx_encoding: compact::PortraitIdentityEncoding::Utf8,
            }],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.frame.status_flags, 0);
        assert_eq!(result.stats.character_rows, 1);
        assert_eq!(result.stats.unit_leader_rows, 1);
        assert_eq!(result.stats.unit_leader_trait_rows, 1);
        assert_eq!(result.stats.country_leader_role_rows, 1);
        // ruler fails the tag-echo ownership guard; country 4 has no memory.
        assert_eq!(result.stats.active_ruler_rows, 1);
        assert_eq!(result.stats.active_ruler_unresolved_roots, 2);
        // Lane PR: the FIRST of those two refusals, NAMED. Country 3's ruler
        // resolves to character 100, whose CCharacter says country 2, so the
        // tag-echo guard raises `BadTypeId` with expected 3 / observed 2 - the
        // same shape lane PR measured live (reason 6, exactly one country per
        // frame, every frame of two independent captures).
        assert_eq!(result.stats.active_ruler_unresolved_first_tag, 3);
        assert_eq!(result.stats.active_ruler_unresolved_first_reason, 6);
        assert_eq!(result.stats.active_ruler_unresolved_first_expected, 3);
        assert_eq!(result.stats.active_ruler_unresolved_first_observed, 2);
        assert_eq!(result.stats.rejected_roots, 0);
        assert!(!result.stats.has_rejections());
        assert_eq!(result.stats.advisor_roster_rows, 1);
        assert_eq!(result.stats.appointed_advisor_rows, 1);
        assert_eq!(result.stats.portrait_rows, 1);

        let payload = extractor::encode_frame_payload(&result.frame);
        let summary = extractor::decode_frame_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.character_portrait_rows, 1);
        assert_eq!(summary.appointed_advisor_rows, 1);
    }

    #[test]
    fn an_unavailable_country_bridge_publishes_a_word_an_outcome_can_speak_for() {
        let mut mem = FakeMemory::default();
        write_character(&mut mem, 0x1000, 100, 2);
        let input = AdapterInput {
            characters: vec![CharacterRoot {
                character_addr: 0x1000,
                char_id: 100,
                name: None,
                token: Some("ENG_character".to_string()),
            }],
            ..AdapterInput::default()
        };
        assert!(input.countries.is_empty());

        let status = build_lane_frame_from_adapter_input(&mem, &roots(), &input)
            .frame
            .status_flags;
        assert_ne!(
            status & compact::STATUS_BRIDGE_UNAVAILABLE,
            0,
            "an empty country bridge must still say so"
        );
        assert_ne!(
            status & compact::STATUS_SOURCE_UNAVAILABLE,
            0,
            "the bridge outage must also carry a bit the outcome vocabulary derives, or the \
             collector's sweep cannot substantiate this word and the frame is refused for it"
        );
    }

    #[test]
    fn frame_walk_merges_same_country_duplicate_character_ids() {
        let mut mem = FakeMemory::default();
        write_character(&mut mem, 0x1000, 100, 2);
        write_character(&mut mem, 0x1800, 100, 2);
        let input = AdapterInput {
            characters: vec![
                CharacterRoot {
                    character_addr: 0x1000,
                    char_id: 100,
                    name: Some("first spelling".to_string()),
                    token: Some("ENG_character".to_string()),
                },
                CharacterRoot {
                    character_addr: 0x1800,
                    char_id: 100,
                    name: Some("second spelling".to_string()),
                    token: Some("ENG_character".to_string()),
                },
            ],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.character_roots_seen, 2);
        assert_eq!(result.stats.character_rows, 1);
        assert_eq!(result.stats.deduplicated_character_rows, 1);
        assert_eq!(result.stats.ambiguous_character_rows, 0);
        assert!(!result.stats.has_rejections());
        assert_eq!(result.frame.characters[0].country_tag_db_id, 2);
        assert_eq!(result.frame.characters[0].char_id, 100);
        assert_eq!(result.frame.characters[0].name, None);
        assert_eq!(
            result.frame.characters[0].token.as_deref(),
            Some("ENG_character")
        );
        let summary =
            extractor::decode_frame_payload(0, &extractor::encode_frame_payload(&result.frame));
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn frame_walk_quarantines_cross_country_character_id_conflicts() {
        let mut mem = FakeMemory::default();
        write_character(&mut mem, 0x1000, 100, 2);
        write_character(&mut mem, 0x1800, 100, 3);
        let input = AdapterInput {
            characters: vec![
                CharacterRoot {
                    character_addr: 0x1000,
                    char_id: 100,
                    name: None,
                    token: None,
                },
                CharacterRoot {
                    character_addr: 0x1800,
                    char_id: 100,
                    name: None,
                    token: None,
                },
            ],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.character_rows, 0);
        assert_eq!(result.stats.deduplicated_character_rows, 0);
        assert_eq!(result.stats.ambiguous_character_rows, 2);
        assert!(!result.stats.has_rejections());
        assert!(result.frame.characters.is_empty());
        let summary =
            extractor::decode_frame_payload(0, &extractor::encode_frame_payload(&result.frame));
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
    }

    struct HealingMemory {
        inner: FakeMemory,
        target: u64,
        miss_pending: std::cell::Cell<bool>,
    }

    impl LaneMemory for HealingMemory {
        fn read_u8(&self, addr: u64) -> Option<u8> {
            if addr == self.target && self.miss_pending.replace(false) {
                return None;
            }
            self.inner.read_u8(addr)
        }
    }

    #[test]
    fn stably_failing_root_still_rejects_the_frame_but_is_attributed_dead() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 100, 2);

        let input = AdapterInput {
            elapsed_wall_ms: 5,
            characters: vec![
                CharacterRoot {
                    character_addr: character,
                    char_id: 100,
                    name: None,
                    token: None,
                },
                CharacterRoot {
                    character_addr: 0xDEAD_0000,
                    char_id: 101,
                    name: None,
                    token: None,
                },
            ],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.character_rows, 1);
        assert_eq!(result.stats.dead_roots, 1);
        assert_eq!(result.stats.rejected_roots, 0);
        assert!(
            result.stats.has_rejections(),
            "stable failures must still fail the frame closed"
        );
        assert_ne!(result.frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
    }

    #[test]
    fn torn_root_read_still_rejects_the_frame_fail_closed() {
        let mut inner = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut inner, character, 100, 2);
        let mem = HealingMemory {
            inner,
            target: character,
            miss_pending: std::cell::Cell::new(true),
        };

        let input = AdapterInput {
            elapsed_wall_ms: 5,
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 100,
                name: None,
                token: None,
            }],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.dead_roots, 0);
        assert_eq!(result.stats.rejected_roots, 1);
        assert!(result.stats.has_rejections());
        assert_ne!(result.frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
    }

    #[test]
    fn a_recycled_character_keeps_its_live_identity_and_loses_the_census_text() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 777, 9);

        let input = AdapterInput {
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 100,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.identity_degraded_rows, 1);
        assert_eq!(result.stats.character_rows, 1);
        assert_eq!(result.frame.characters[0].char_id, 777);
        assert_eq!(result.frame.characters[0].country_tag_db_id, 9);
        assert_eq!(result.frame.characters[0].name, None);
        assert_eq!(result.frame.characters[0].token, None);
        assert_eq!(result.stats.dead_roots, 0);
        assert_eq!(result.stats.rejected_roots, 0);
        assert_eq!(result.stats.identity_mismatch_rows, 0);
        assert!(!result.stats.has_rejections());
        assert_ne!(result.frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
    }

    #[test]
    fn a_recycled_character_frame_decodes_without_a_parse_error() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 777, 9);

        let input = AdapterInput {
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 100,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.identity_degraded_rows, 1);
        assert_ne!(result.frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        assert_ne!(
            result.frame.status_flags & compact::STATUS_SOURCE_UNAVAILABLE,
            0,
            "an undeclared row loss must be spelled with SOURCE_UNAVAILABLE, or the decoder \
             refuses the payload and the refusal rebuilds the prewarm"
        );

        let decoded =
            extractor::decode_frame_payload(0, &extractor::encode_frame_payload(&result.frame));
        assert!(
            decoded.parse_errors.is_empty(),
            "one recycled character must not make the payload unparseable: {:?}",
            decoded.parse_errors
        );
        assert_eq!(result.stats.character_rows, 1);
        assert!(!result.frame.characters.is_empty());
    }

    #[test]
    fn a_character_that_changed_country_since_the_census_keeps_its_text() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 100, 3);

        let input = AdapterInput {
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 100,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.identity_degraded_rows, 0);
        assert!(!result.stats.has_rejections());
        assert_eq!(result.frame.characters[0].country_tag_db_id, 3);
        assert_eq!(
            result.frame.characters[0].name.as_deref(),
            Some("Maxwell Aitken")
        );
    }

    #[test]
    fn an_unchanged_character_keeps_the_census_text() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 100, 2);

        let input = AdapterInput {
            characters: vec![CharacterRoot {
                character_addr: character,
                char_id: 100,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.identity_degraded_rows, 0);
        assert!(!result.stats.has_rejections());
        assert_eq!(result.frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
        assert_eq!(
            result.frame.characters[0].name.as_deref(),
            Some("Maxwell Aitken")
        );
    }

    #[test]
    fn a_recycled_country_leader_character_refuses_the_role_row() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        let country_leader = 0x3000;
        write_character(&mut mem, character, 777, 2);
        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            character,
        );

        let input = AdapterInput {
            country_leader_roles: vec![CountryLeaderRoleRoot {
                country_leader_addr: country_leader,
                char_id: 100,
                name: None,
                token: None,
            }],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.identity_mismatch_rows, 1);
        assert_eq!(result.stats.country_leader_role_rows, 0);
        assert!(result.frame.country_leader_roles.is_empty());
        assert_eq!(result.stats.dead_roots, 0);
        assert_eq!(result.stats.rejected_roots, 0);
        assert!(result.stats.has_rejections());
        assert_ne!(result.frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
    }

    #[test]
    fn a_country_leader_whose_character_changed_country_is_emitted_under_the_live_country() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        let country_leader = 0x3000;
        write_character(&mut mem, character, 100, 3);
        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            character,
        );

        let input = AdapterInput {
            country_leader_roles: vec![CountryLeaderRoleRoot {
                country_leader_addr: country_leader,
                char_id: 100,
                name: None,
                token: None,
            }],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.identity_mismatch_rows, 0);
        assert_eq!(result.stats.country_leader_role_rows, 1);
        assert_eq!(result.frame.country_leader_roles[0].country_tag_db_id, 3);
        assert_eq!(result.frame.country_leader_roles[0].char_id, 100);
        assert!(!result.stats.has_rejections());
    }

    #[test]
    fn an_unchanged_country_leader_role_is_emitted() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        let country_leader = 0x3000;
        write_character(&mut mem, character, 100, 2);
        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            character,
        );

        let input = AdapterInput {
            country_leader_roles: vec![CountryLeaderRoleRoot {
                country_leader_addr: country_leader,
                char_id: 100,
                name: None,
                token: None,
            }],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.identity_mismatch_rows, 0);
        assert_eq!(result.stats.country_leader_role_rows, 1);
        assert_eq!(result.frame.country_leader_roles[0].char_id, 100);
        assert_eq!(result.frame.country_leader_roles[0].country_tag_db_id, 2);
        assert!(!result.stats.has_rejections());
    }

    #[test]
    fn a_country_leader_row_is_insensitive_to_which_object_carries_it() {
        const CCOUNTRY_LEADER_BODY_LEN: u64 = 0x150;

        let build = |country_leader: u64, fill: u8| {
            let mut mem = FakeMemory::default();
            let character = 0x1000;
            write_character(&mut mem, character, 100, 2);
            let mut offset = 0u64;
            while offset < CCOUNTRY_LEADER_BODY_LEN {
                mem.write_u8(country_leader + offset, fill);
                offset += 1;
            }
            mem.write_u64(country_leader, COUNTRY_LEADER_VT);
            mem.write_u64(
                country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
                character,
            );

            let input = AdapterInput {
                country_leader_roles: vec![CountryLeaderRoleRoot {
                    country_leader_addr: country_leader,
                    char_id: 100,
                    name: None,
                    token: None,
                }],
                ..AdapterInput::default()
            };
            build_lane_frame_from_adapter_input(&mem, &roots(), &input)
        };

        let held_by_the_map = build(0x3000, 0x5A);
        let seen_only_by_the_sweep = build(0x8000, 0xA5);

        assert_eq!(held_by_the_map.stats.country_leader_role_rows, 1);
        assert_eq!(seen_only_by_the_sweep.stats.country_leader_role_rows, 1);
        assert_eq!(
            held_by_the_map.frame.country_leader_roles,
            seen_only_by_the_sweep.frame.country_leader_roles
        );
        assert!(!held_by_the_map.stats.has_rejections());
        assert!(!seen_only_by_the_sweep.stats.has_rejections());
    }

    #[test]
    fn appointed_intersection_is_country_scoped() {
        let roster = vec![AdvisorRecord {
            country_tag_db_id: 2,
            char_id: 100,
            slot: Some("political_advisor".to_string()),
            ledger: Some("political_advisor".to_string()),
            idea_token: "shared_token".to_string(),
        }];
        let countries = vec![CountryPoliticsRoot {
            country_addr: 0,
            country_tag_db_id: 3,
            ruling_party: None,
            ruler_name: None,
            ruler_token: None,
            active_idea_tokens: vec!["shared_token".to_string()],
        }];
        assert!(appointed_advisors_from_country_active_ideas(&roster, &countries).is_empty());
    }

    /// Lane PR: `appointed_join_country_tag_hits` counts the roster rows that
    /// found a country, so a zero join says WHICH half of its key missed.
    ///
    /// kills: wiring the counter to `roster.len()` (it would report 3 for the
    /// shape below, where only 2 rows name a country this frame carries) or to
    /// the result length (it would report 1, and a country-key failure and a
    /// token failure would again be indistinguishable).
    #[test]
    fn pr_the_join_counts_the_rows_that_found_a_country() {
        let roster = vec![
            AdvisorRecord {
                country_tag_db_id: 2,
                char_id: 100,
                slot: None,
                ledger: None,
                idea_token: "held".to_string(),
            },
            AdvisorRecord {
                country_tag_db_id: 2,
                char_id: 101,
                slot: None,
                ledger: None,
                idea_token: "not_held".to_string(),
            },
            AdvisorRecord {
                country_tag_db_id: 9,
                char_id: 102,
                slot: None,
                ledger: None,
                idea_token: "held".to_string(),
            },
        ];
        let countries = vec![CountryPoliticsRoot {
            country_addr: 0x1000,
            country_tag_db_id: 2,
            ruling_party: None,
            ruler_name: None,
            ruler_token: None,
            active_idea_tokens: vec!["held".to_string()],
        }];

        let join = appointed_advisors_from_country_active_ideas_counted(&roster, &countries);
        // Two rows named country 2 and reached the token test; one named
        // country 9, which this frame does not carry.
        assert_eq!(join.country_tag_hits, 2);
        // ... and only one of those two held the token.
        assert_eq!(join.rows.len(), 1);
        assert_eq!(join.rows[0].char_id, 100);

        // THE LIVE SHAPE lane PR measured: every row finds its country and
        // none holds its token. Hits are then the whole roster while the
        // result is empty, which is the reading that says the fault is the
        // token namespace and not the country key.
        let no_tokens = vec![CountryPoliticsRoot {
            country_addr: 0x1000,
            country_tag_db_id: 2,
            ruling_party: None,
            ruler_name: None,
            ruler_token: None,
            active_idea_tokens: vec!["a_law".to_string(), "a_spirit".to_string()],
        }];
        let join = appointed_advisors_from_country_active_ideas_counted(&roster[..2], &no_tokens);
        assert_eq!(join.country_tag_hits, 2);
        assert!(join.rows.is_empty());
    }

    /// Lane PR: the counted join and the plain one are the same join.
    ///
    /// kills: any divergence between `appointed_advisors_from_country_active_ideas`
    /// and the counted form it now delegates to - the counter must be an
    /// observation of the join, never a second implementation of it.
    #[test]
    fn pr_the_counted_join_returns_what_the_plain_join_returns() {
        let roster = vec![
            AdvisorRecord {
                country_tag_db_id: 2,
                char_id: 100,
                slot: Some("political_advisor".to_string()),
                ledger: None,
                idea_token: "held".to_string(),
            },
            AdvisorRecord {
                country_tag_db_id: 2,
                char_id: 100,
                slot: Some("political_advisor".to_string()),
                ledger: None,
                idea_token: "held".to_string(),
            },
            AdvisorRecord {
                country_tag_db_id: 3,
                char_id: 101,
                slot: None,
                ledger: None,
                idea_token: "also_held".to_string(),
            },
        ];
        let countries = vec![
            CountryPoliticsRoot {
                country_addr: 0x1000,
                country_tag_db_id: 2,
                ruling_party: None,
                ruler_name: None,
                ruler_token: None,
                active_idea_tokens: vec!["held".to_string()],
            },
            CountryPoliticsRoot {
                country_addr: 0x2000,
                country_tag_db_id: 3,
                ruling_party: None,
                ruler_name: None,
                ruler_token: None,
                active_idea_tokens: vec!["also_held".to_string(), "held".to_string()],
            },
        ];
        assert_eq!(
            appointed_advisors_from_country_active_ideas(&roster, &countries),
            appointed_advisors_from_country_active_ideas_counted(&roster, &countries).rows
        );
        // The dedup still runs: the repeated (country, char, token) triple
        // collapses, so two rows come out of three.
        assert_eq!(
            appointed_advisors_from_country_active_ideas(&roster, &countries).len(),
            2
        );
    }

    /// Lane PR: every `ExtractError` shape gets its own reason code, and none
    /// of them is 0.
    ///
    /// kills: a reason function that returned one constant, which would leave
    /// `domain47_adapter_active_ruler_unresolved_first_reason` unable to
    /// separate a torn read from a structural refusal; and a 0 code, which the
    /// counter reserves for "nothing was refused".
    #[test]
    fn pr_every_refusal_shape_has_its_own_nonzero_reason_code() {
        let errors = [
            ExtractError::RegistryMismatch(String::new()),
            ExtractError::AddressOverflow("f"),
            ExtractError::NullPointer("f"),
            ExtractError::MissingRead { addr: 1, field: "f" },
            ExtractError::BadVtable {
                addr: 1,
                field: "f",
                expected: 2,
                observed: 3,
            },
            ExtractError::BadTypeId {
                addr: 1,
                field: "f",
                expected: 2,
                observed: 3,
            },
            ExtractError::BadVector {
                field: "f",
                begin: 1,
                end: 2,
                size: 3,
            },
            ExtractError::IdentityMismatch {
                addr: 1,
                field: "f",
                expected: 2,
                observed: 3,
            },
            ExtractError::EmptyLeaderVector,
        ];
        let codes: BTreeSet<u32> = errors.iter().map(extract_error_reason).collect();
        assert_eq!(codes.len(), errors.len(), "every shape has its own code");
        assert!(!codes.contains(&0), "0 means nothing was refused");
        // Only the two shapes that COMPARE two scalars carry operands; the
        // rest report none rather than a misleading pair.
        assert_eq!(
            errors
                .iter()
                .filter(|error| extract_error_operands(error) != (0, 0))
                .count(),
            2
        );
        assert_eq!(
            extract_error_operands(&ExtractError::BadTypeId {
                addr: 1,
                field: "f",
                expected: 2,
                observed: 3,
            }),
            (2, 3)
        );
    }

    /// Lane MX (Codex 01a00f9e b2 R2, finding 1's empty-map arm): with an
    /// EMPTY trait-token map the adapter does NOT read a leader's trait
    /// vector, so a live vector holding a trait publishes `trait_count = 0`
    /// with no `missing_trait_tokens`. This is the long-standing k5 shape
    /// (trait family unobserved) and it is exactly why the collector's
    /// sweep-only skip must never LEAVE the map empty: a pass that skipped
    /// hits down to an empty map is discarded and rerun without the skip
    /// (`trait_skip_left_empty_map` in attach_log.rs).
    ///
    /// kills: making the adapter read vectors under an empty map (a lawful
    /// alternative fix) — this test then fails to remind whoever does it to
    /// remove the collector's empty-map retry arm in the same change.
    #[test]
    fn an_empty_trait_token_map_skips_the_vector_read() {
        let mut mem = FakeMemory::default();
        let leader = 0x2000u64;
        let skill_source = 0x2900u64;
        let trait_data = 0x2A00u64;
        let trait_def = 0x2B00u64;
        mem.write_u64(leader, ARMY_VT);
        mem.write_u32(leader + CUNIT_LEADER_TYPE_OFF, CUNIT_LEADER_TYPE_ID);
        mem.write_u32(leader + CUNIT_LEADER_ID_OFF, 500);
        mem.write_u32(leader + CUNIT_LEADER_CHARACTER_ID_COPY_OFF, 100);
        mem.write_i32(leader + CUNIT_LEADER_EXPERIENCE_RAW_OFF, 42_000);
        mem.write_u64(leader + CUNIT_LEADER_SKILL_SOURCE_PTR_OFF, skill_source);
        mem.write_u32(skill_source + CUNIT_LEADER_SKILL_VALUE_OFF, 4);
        // The live vector HOLDS one trait.
        mem.write_u64(leader + CUNIT_LEADER_TRAIT_DATA_OFF, trait_data);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_CAPACITY_OFF, 1);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_SIZE_OFF, 1);
        mem.write_u64(trait_data, trait_def);
        mem.write_u64(trait_def, TRAIT_VT);
        let root = UnitLeaderRoot {
            leader_addr: leader,
            country_tag_db_id: 2,
            leader_kind: UNIT_LEADER_ARMY,
            name: None,
            token: None,
        };
        let empty: BTreeMap<u64, &str> = BTreeMap::new();
        let (leader_row, trait_rows, missing) =
            read_unit_leader(&mem, &roots(), &root, &empty).expect("leader reads");
        assert_eq!(trait_rows.len(), 0);
        assert_eq!(missing, 0, "an empty map reports nothing missing: the vector is never read");
        assert_eq!(leader_row.trait_count, 0);
        // CONTROL: a non-empty map over the same bytes decodes the trait.
        let mut tokens: BTreeMap<u64, &str> = BTreeMap::new();
        tokens.insert(trait_def, "old_guard");
        let (leader_row, trait_rows, missing) =
            read_unit_leader(&mem, &roots(), &root, &tokens).expect("leader reads");
        assert_eq!((trait_rows.len(), missing, leader_row.trait_count), (1, 0, 1));
    }
}
