use std::collections::{BTreeMap, BTreeSet};

use crate::domain47_characters_leaders_cabinet_live as compact;
use crate::extractor::{
    advisor_bucket, read_active_ruler_from_country, read_advisor_raw_fields,
    read_character_raw_fields, read_unit_leader_skill, read_unit_leader_trait_vector,
    read_vtable_and_character_ptr, CCOUNTRY_LEADER_RAW_FIELDS_SPAN,
    AdvisorRecord, CharacterPortraitRecord, CharacterRecord, CountryLeaderRoleRecord, ExtractError,
    LaneFrame, LaneMemory, ResolvedTypeRoots, UnitLeaderRecord, UnitLeaderTraitRecord,
    CCOUNTRY_LEADER_CHARACTER_PTR_OFF, CUNIT_LEADER_CHARACTER_ID_COPY_OFF, CUNIT_LEADER_ID_OFF,
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

/// Borrowed twin of [`AdapterInput`]. The frame walk only ever READS its root
/// lists, but the scheduled reader used to hand it an owned `AdapterInput`
/// deep-cloned out of the persistent root cache — thousands of `String`
/// allocations per hour purely to be dropped again at the end of the frame.
/// The build below is written against this borrowed form and [`AdapterInput`]
/// simply lends its vectors to it, so both entry points execute the identical
/// code over the identical data.
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
    /// The IDENTITY the prewarm census read at `character_addr`, in the same
    /// observation that produced `name` and `token`.
    ///
    /// Without it the root carried an ADDRESS and two strings and nothing
    /// that ties them together: the frame walk re-reads `char_id` and
    /// `country_tag_db_id` live at the same address, and if the engine freed
    /// that `CCharacter` and the allocator handed the storage to a DIFFERENT
    /// character between the prewarm and the walk, the walk publishes the new
    /// identity wearing the old character's name and token. Every check
    /// downstream passes: the vtable is a `CCharacter` vtable, the type id is
    /// right, the tag is valid, the strings are well-formed — the row is
    /// silently wrong, not absent, which is the outcome this project refuses.
    ///
    /// Carrying the census identity costs NO extra engine read (the census
    /// already reads this field to dedup and to attribute unit leaders) and
    /// lets the walk compare what it read against what the strings belong to.
    /// See `build_lane_frame_from_adapter_input_ref`.
    ///
    /// `char_id` ALONE, deliberately: it is the engine's own identity for the
    /// character and does not change while the character exists, which is
    /// exactly what a reuse discriminator has to be. `country_tag_db_id` is
    /// NOT one — a character legitimately changes country when its country is
    /// annexed or it is transferred, and treating that as evidence of a
    /// recycled allocation would throw away the name and token of every
    /// character caught by an annexation between the prewarm and the walk of
    /// the same frame. Nothing is lost by leaving it out: the emitted row's
    /// country has always come from this frame's own live read, never from
    /// the census. (Codex 019fd6ea, finding 2.)
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
    /// The `char_id` the prewarm census read through this role's character
    /// pointer.
    ///
    /// The emitted role row is a (country, char_id) PAIR, and its two halves
    /// used to come from different instants: the country was the census's
    /// (this struct used to carry a `country_tag_db_id`) and the char id was
    /// the walk's, combined WITHOUT ever being compared. A `CCountryLeader`
    /// whose `CCharacter` had been recycled since the census therefore
    /// emitted a real char id under a country it never belonged to, and a
    /// character whose country had legitimately changed emitted the STALE
    /// country.
    ///
    /// Both are gone because the row is now built entirely from this frame's
    /// own read — the country included — and the census contributes only this
    /// id, used to prove the object is still the character the census saw.
    /// `domain47_country_leader_identity` already reads the id and used to
    /// discard it, so this adds no engine read.
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
    /// Which decoding produced `gfx` in the producer that read it. Carried,
    /// never re-derived: the decoded text is identical under both encodings, so
    /// nothing downstream can recover this from `gfx` alone.
    pub gfx_encoding: compact::PortraitIdentityEncoding,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdapterBuildStats {
    pub character_roots_seen: u32,
    pub character_rows: u32,
    /// Same-country roots that re-resolved to an already observed logical
    /// character this frame. They are merged without choosing between
    /// disagreeing ancillary strings.
    pub deduplicated_character_rows: u32,
    /// Roots participating in a cross-country char-id conflict this frame.
    /// Both logical identities are quarantined, matching the prewarm cache's
    /// identity policy; neither country is selected as an arbitrary winner.
    pub ambiguous_character_rows: u32,
    pub unit_leader_roots_seen: u32,
    pub unit_leader_rows: u32,
    pub unit_leader_trait_rows: u32,
    pub country_leader_roots_seen: u32,
    pub country_leader_role_rows: u32,
    pub country_roots_seen: u32,
    pub active_ruler_rows: u32,
    /// Countries whose active-ruler chain COULD NOT BE READ this frame: an
    /// unreadable pointer in the country/status/party chain, or a malformed
    /// leader vector. The ruler row is absent and nothing is fabricated.
    ///
    /// Nonzero here now clears `FAMILY_ACTIVE_RULER` from the frame's declared
    /// families (see `scheduled::read_native_scheduled_frame`), because the
    /// fold full-replaces `commanders.leader` for every family it is told was
    /// observed — so declaring the family whole while one country's read failed
    /// deleted that country's leader and dated the deletion to this hour.
    ///
    /// That is why the empty-vector case had to move out of this counter. The
    /// live 870ebf D04 re-probe resolved 435/437 countries with two expected
    /// STRUCTURAL outliers — rebels and the like, whose ruling party simply has
    /// no leaders. Those two are observations, and while they were counted here
    /// this number was nonzero on every frame and could not be evidence of
    /// anything. They are `active_ruler_absent_roots` now, and on that measured
    /// world this counter is zero.
    pub active_ruler_unresolved_roots: u32,
    /// Countries whose ruling party was READ and holds no country leader.
    ///
    /// Separated from `active_ruler_unresolved_roots` so the latter means only
    /// "could not tell". Kept as its own count rather than discarded because a
    /// sudden change in how many countries have no leader is a real signal
    /// about the walk, just not a failure one.
    pub active_ruler_absent_roots: u32,
    pub advisor_roots_seen: u32,
    pub advisor_roster_rows: u32,
    pub appointed_advisor_rows: u32,
    pub portrait_inputs_seen: u32,
    pub portrait_rows: u32,
    pub rejected_roots: u32,
    /// Rows REFUSED because the object still reads cleanly but no longer
    /// carries the identity the prewarm census read at that address —
    /// recycled storage. Today this is the country-leader role path, where
    /// the emitted row IS the (country, char_id) pair and there is no half of
    /// it left to publish once the two observations disagree.
    ///
    /// In `has_rejections` because a refused row is an OMISSION from a
    /// full-replacement frame, and a downstream reader cannot tell an
    /// omission from a deletion — the same reason `dead_roots` is in there.
    ///
    /// Kept apart from `rejected_roots` (torn) and `dead_roots` (stably
    /// failing) because the failure mode and the remedy are different: this
    /// one converges the moment the prewarm re-censuses, and a persistent
    /// nonzero means character churn is outrunning the census rather than
    /// that reads are going bad.
    pub identity_mismatch_rows: u32,
    /// Rows DEGRADED for the same reason: the identity at the address moved,
    /// so the census's ancillary text is unattributable and is dropped, but
    /// the row itself is this frame's own live, fully validated observation
    /// and is emitted.
    ///
    /// Deliberately NOT in `has_rejections`. That predicate makes the whole
    /// domain-47 frame rowless-PARTIAL (see `scheduled.rs`), which is the
    /// right answer for a row that is MISSING and the wrong one for a row
    /// that is merely thinner: throwing ~13_000 good character rows away
    /// because one character was recycled is exactly the "missing data
    /// destroys the whole capture" outcome this project refuses. It sets
    /// `STATUS_PARTIAL_DATA` on its own instead, so the thinning is declared
    /// without the frame being discarded.
    pub identity_degraded_rows: u32,
    pub missing_trait_tokens: u32,
    /// Roots whose frame-walk read failed STABLY (an immediate re-read failed
    /// bit-identically), as opposed to TORN (`rejected_roots`, the re-read
    /// disagreed). DIAGNOSTIC ATTRIBUTION ONLY — stable failures still reject
    /// the frame fail-closed, exactly like torn ones (see `has_rejections`).
    /// The 2026-07-11 adversarial gate REFUTED the earlier claim that a
    /// stable failure proves the ROOT is dead: a NESTED read failure (skill
    /// source, trait vector, referenced trait/character) can be stable while
    /// the parent root lives, and omitting a live root from a CLEAN
    /// full-replacement frame silently DELETES it downstream. Positive
    /// absence is only established when a COMPLETE sweep + prewarm rebuild no
    /// longer yields the root — never by a walk read failure. The collector's
    /// rejection-triggered invalidation fires on this counter so the root
    /// cache converges for the genuinely-dead case; row restoration under
    /// per-frame churn is the PARTIAL-upsert fold design (queued), not
    /// clean-emission-with-omissions.
    pub dead_roots: u32,
}

impl AdapterBuildStats {
    pub fn has_rejections(&self) -> bool {
        self.rejected_roots != 0
            || self.missing_trait_tokens != 0
            || self.dead_roots != 0
            || self.identity_mismatch_rows != 0
    }
}

/// Classification of a root whose frame-walk read failed.
enum RootFailure {
    /// The re-read failed bit-identically: the object is stably gone.
    Dead,
    /// The re-read disagreed (different error, or success): mid-read mutation.
    Torn,
}

/// Dead-vs-torn: re-run the SAME read after a failure and compare. Matches the
/// prewarm scan's `(Err(a), Err(b)) if a == b => stable exclusion` discipline,
/// so both layers agree on what "dead" means.
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
    let mut frame = LaneFrame::new(input.elapsed_wall_ms);
    frame.status_flags = input.status_flags;
    let mut stats = AdapterBuildStats::default();
    let trait_tokens = input
        .unit_leader_trait_tokens
        .iter()
        .map(|row| (row.trait_addr, row.token.as_str()))
        .collect::<BTreeMap<_, _>>();

    // `known_characters` exists for ONE consumer: the portrait filter at the
    // end of this function, which admits a portrait row only for a character
    // some earlier loop saw. Every other loop only ever writes to it. When
    // there are no portrait inputs the set is built and then never read, so
    // the ~10k inserts a frame are pure waste; skipping them when the sole
    // reader has nothing to read is exactly equivalent. (The prewarm now DOES
    // populate portraits, so the skip is the exceptional path rather than the
    // normal one — but it stays, because a campaign whose characters define no
    // portraits is a campaign this walk must not pay ~10k inserts for.)
    let track_known_characters = !input.portraits.is_empty();
    let mut known_characters = BTreeSet::new();
    // Index INTO `frame.characters`, not a second copy of every character's
    // name and token. The active-ruler loop below backfills an absent ruler
    // name/token from the character row of the same identity; it needs at
    // most one identity per country, and there are ~171 countries against
    // ~5_200 characters. Cloning both strings for every character to serve
    // that lookup was two heap allocations per character per frame.
    let mut character_identity = BTreeMap::<(u32, u32), usize>::new();
    let mut characters_by_id = BTreeMap::<u32, CharacterRecord>::new();
    let mut ambiguous_character_ids = BTreeSet::new();
    stats.character_roots_seen = input.characters.len().min(u32::MAX as usize) as u32;
    for root in input.characters {
        match read_character_raw_fields(mem, roots, root.character_addr) {
            Ok(raw) => {
                // The root's `name`/`token` were read at this address by the
                // prewarm census, in the SAME observation that read the
                // identity now carried on the root. If what lives here now is
                // a different character, the storage was recycled and pairing
                // this frame's identity with that text would publish a
                // well-formed WRONG row — right vtable, right type id, valid
                // tag, printable strings, and the wrong person.
                //
                // DEGRADE, not drop. The identity below was read live this
                // frame and passed every structural check, so the character
                // exists; only its ancillary text is unattributable. This
                // frame is a full-replacement snapshot, so omitting the row
                // would assert that the character is GONE — the same silent
                // deletion `dead_roots` documents. Absent text is the
                // domain's existing answer to "two observations disagree and
                // there is no arbitrary winner" (see the duplicate merge
                // below, which nulls exactly these two fields).
                //
                // `char_id` only: the country is emitted live and moves for
                // reasons that have nothing to do with allocation — see
                // `CharacterRoot::char_id`.
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
    for (index, row) in frame.characters.iter().enumerate() {
        let key = (row.country_tag_db_id, row.char_id);
        if track_known_characters {
            known_characters.insert(key);
        }
        // `characters_by_id` is keyed by char_id, so each char_id occurs
        // once in `frame.characters` and this key is unique — the map has
        // no overwrite case to preserve. `frame.characters` is not mutated
        // again in this function, so the index stays valid at the read.
        character_identity.insert(key, index);
    }
    stats.character_rows = frame.characters.len().min(u32::MAX as usize) as u32;

    stats.unit_leader_roots_seen = input.unit_leaders.len().min(u32::MAX as usize) as u32;
    for root in input.unit_leaders {
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
    stats.unit_leader_rows = frame.unit_leaders.len().min(u32::MAX as usize) as u32;
    stats.unit_leader_trait_rows = frame.unit_leader_traits.len().min(u32::MAX as usize) as u32;

    stats.country_leader_roots_seen =
        input.country_leader_roles.len().min(u32::MAX as usize) as u32;
    for root in input.country_leader_roles {
        match read_country_leader_role(mem, roots, root) {
            Ok(row) => {
                if track_known_characters {
                    known_characters.insert((row.country_tag_db_id, row.char_id));
                }
                frame.country_leader_roles.push(row);
            }
            // A recycled object is not a dead root and not a torn read: it
            // reads cleanly, it just is not the object the census saw. It
            // gets its own counter so a persistent nonzero is legible as
            // churn rather than as memory going bad, and it deliberately does
            // NOT reach `classify_root_failure` — re-reading a recycled
            // object returns the same mismatch and would be filed as "stably
            // dead", which is a different and wrong claim.
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
    stats.country_leader_role_rows = frame.country_leader_roles.len().min(u32::MAX as usize) as u32;

    stats.country_roots_seen = input.countries.len().min(u32::MAX as usize) as u32;
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
                if let Some(index) = character_identity
                    .get(&(row.country_tag_db_id, row.char_id))
                    .copied()
                {
                    let identity = &frame.characters[index];
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
            // A country whose ruling party has no leaders emits no row, and
            // that is a complete answer rather than a missing one. Counting it
            // as unresolved is what made `active_ruler_unresolved_roots`
            // nonzero on essentially every frame, which in turn made the
            // counter useless as evidence that the family was observed whole.
            Err(ExtractError::EmptyLeaderVector) => {
                stats.active_ruler_absent_roots = stats.active_ruler_absent_roots.saturating_add(1)
            }
            Err(_) => {
                stats.active_ruler_unresolved_roots =
                    stats.active_ruler_unresolved_roots.saturating_add(1)
            }
        }
    }
    stats.active_ruler_rows = frame.active_rulers.len().min(u32::MAX as usize) as u32;

    stats.advisor_roots_seen = input.advisors.len().min(u32::MAX as usize) as u32;
    for root in input.advisors {
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
    stats.advisor_roster_rows = frame.advisor_roster.len().min(u32::MAX as usize) as u32;
    // A currently appointed advisor is the exact intersection of the validated
    // CAdvisor roster and this country's same-frame active-idea identities.
    // `SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS` has described that wire contract
    // since v1; do not replace a measured non-empty intersection with a forced
    // empty list. Leveled/modded idea identities remain fail-closed because
    // only literal identity equality is admitted.
    frame.appointed_advisors =
        appointed_advisors_from_country_active_ideas(&frame.advisor_roster, input.countries);
    stats.appointed_advisor_rows =
        frame.appointed_advisors.len().min(u32::MAX as usize) as u32;

    stats.portrait_inputs_seen = input.portraits.len().min(u32::MAX as usize) as u32;
    for portrait in input.portraits {
        if known_characters.contains(&(portrait.country_tag_db_id, portrait.char_id)) {
            frame.character_portraits.push(CharacterPortraitRecord {
                country_tag_db_id: portrait.country_tag_db_id,
                char_id: portrait.char_id,
                role: portrait.role.clone(),
                size: portrait.size.clone(),
                gfx: portrait.gfx.clone(),
                gfx_encoding: portrait.gfx_encoding,
            });
        } else {
            stats.rejected_roots = stats.rejected_roots.saturating_add(1);
        }
    }
    stats.portrait_rows = frame.character_portraits.len().min(u32::MAX as usize) as u32;

    if stats.has_rejections() {
        frame.status_flags |= compact::STATUS_PARTIAL_DATA;
    }
    if input.skipped_input_rows != 0 {
        frame.status_flags |= compact::STATUS_PARTIAL_DATA;
    }
    // Declared here rather than through `has_rejections`: a degraded row is
    // thinner, not missing, and must not cost the frame its other ~13_000
    // rows. See `AdapterBuildStats::identity_degraded_rows`.
    //
    // SOURCE_UNAVAILABLE rides along, and it is not decoration (task #131,
    // owner-ruled 2026-08-07). A degraded identity is ROW-level loss inside a
    // family this frame DID observe, so `known_families` still says
    // `FAMILY_ALL` and nothing in the presence mask describes the hole.
    // `decode_family_presence_mask` refuses an undeclared payload whose only
    // failure-class bit is `PARTIAL_DATA` — "nothing names what it failed to
    // observe" — and that refusal is a PARSE ERROR.
    //
    // WHAT THE PAIRING BUYS, precisely. It does NOT save this frame's rows:
    // d47's `COMPACT_EXPECTED_PARTIAL_TOLERANCES` entry says in its own words
    // that "row-level loss inside an observed family keeps SOURCE_UNAVAILABLE
    // and stays outside this tolerance", so the scheduled reader still
    // replaces the frame with a rowless one. What it saves is the PARSE ERROR,
    // and with it `domain47_should_invalidate_roots_on_frame`, which turns any
    // parse error into a discarded root cache and a full serial prewarm
    // rebuild on the next frame — a rebuild that did not happen before
    // per-family admission existed, and that costs ~21 ms on the one phase of
    // the frame no worker can overlap. One recycled character was buying a
    // whole prewarm.
    //
    // Pairing the two flags is the spelling `scheduled.rs` already uses for
    // every other undeclared row loss. The owner accepted SOURCE_UNAVAILABLE
    // appearing on a row-bearing frame: it says "this frame's sources did not
    // all yield", not "there is nothing here".
    if stats.identity_degraded_rows != 0 {
        frame.status_flags |= compact::STATUS_PARTIAL_DATA | compact::STATUS_SOURCE_UNAVAILABLE;
    }
    if input.countries.is_empty() {
        frame.status_flags |= compact::STATUS_BRIDGE_UNAVAILABLE;
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

pub fn appointed_advisors_from_country_active_ideas(
    roster: &[AdvisorRecord],
    countries: &[CountryPoliticsRoot],
) -> Vec<AdvisorRecord> {
    let active_by_country = countries
        .iter()
        .map(|country| {
            (
                country.country_tag_db_id,
                country
                    .active_idea_tokens
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    roster
        .iter()
        .filter(|row| {
            active_by_country
                .get(&row.country_tag_db_id)
                .map(|tokens| tokens.contains(row.idea_token.as_str()))
                .unwrap_or(false)
        })
        .filter(|row| seen.insert((row.country_tag_db_id, row.char_id, row.idea_token.clone())))
        .cloned()
        .collect()
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
    // Prove the object is still the character the census saw before anything
    // the census produced is attached to it. `char_id` is the engine's own
    // identity and does not change while the character exists; the country
    // does (annexation, transfer), so it is not a reuse discriminator — see
    // `CharacterRoot::char_id`.
    if character.char_id != root.char_id {
        return Err(ExtractError::IdentityMismatch {
            addr: character_addr,
            field: "CCountryLeader.character.char_id",
            expected: root.char_id,
            observed: character.char_id,
        });
    }
    Ok(CountryLeaderRoleRecord {
        // BOTH halves from this frame's read. The census's country used to
        // be emitted here, which made the row a pair of two different
        // instants; taking the live one removes the staleness rather than
        // detecting it, and on unchanged state it is the same value, so the
        // payload is byte-identical.
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

fn read_nonzero_u64<M: LaneMemory>(
    mem: &M,
    base: u64,
    offset: u64,
    field: &'static str,
) -> Result<u64, ExtractError> {
    let addr = base
        .checked_add(offset)
        .ok_or(ExtractError::AddressOverflow(field))?;
    let value = mem
        .read_u64_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })?;
    if value == 0 {
        return Err(ExtractError::NullPointer(field));
    }
    Ok(value)
}

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
                // A country whose ruler chain does not resolve (nothing
                // mapped at this address): the ruler row must be ABSENT and
                // counted unresolved without failing the frame.
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
        // Country 2 resolves; country 3 shares country 2's object so its
        // ruler fails the tag-echo ownership guard; country 4 has no memory.
        assert_eq!(result.stats.active_ruler_rows, 1);
        assert_eq!(result.stats.active_ruler_unresolved_roots, 2);
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
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
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
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
    }

    /// Memory that misses ONE address exactly once, then serves the backing
    /// bytes: models a mid-read mutation the dead-vs-torn double read must
    /// classify as TORN (the two reads disagree).
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
        // One valid character plus one root with NO memory behind it: both
        // walk reads fail bit-identically. That is ATTRIBUTED as a stable
        // (dead-looking) failure — but the frame still fails closed: the
        // 2026-07-11 adversarial gate proved a stable failure does NOT prove
        // the root is dead (a nested object can stably fail while the parent
        // lives), and omitting a live root from a CLEAN full-replacement
        // frame would silently delete it downstream.
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
        // The root's first read fails, the re-read succeeds (memory "healed"
        // mid-frame): the two observations disagree -> TORN -> fail-closed
        // frame rejection, exactly as before the dead-vs-torn split.
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

    // ---------------------------------------------------------------
    // Recycled engine objects (lane DC, 2026-08-06).
    //
    // Every test below builds the SAME hazard: the prewarm census read an
    // identity at an address and captured that character's text; by the time
    // the frame walk reaches the same address the engine has freed the object
    // and the allocator has handed the storage to a different character.
    // Nothing is malformed afterwards — the vtable is a CCharacter vtable,
    // the type id is right, the tag is valid, the strings are printable — so
    // no structural check can see it. Only comparing the two observations
    // can.
    // ---------------------------------------------------------------

    /// A recycled `CCharacter` must not carry the census's name and token
    /// onto the identity that lives there NOW.
    ///
    /// MUTATION: delete the `identity_matches` test in the character loop (or
    /// go back to `name: root.name.clone()`) and this fails with
    /// `Some("Maxwell Aitken")` on char 777 — the exact silently-wrong row.
    #[test]
    fn a_recycled_character_keeps_its_live_identity_and_loses_the_census_text() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        // The census saw char 100 of country 2 here. A different character
        // now occupies the same storage.
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
        // The character that IS there was read live and validated, so it is
        // still emitted — omitting it from a full-replacement frame would
        // assert it does not exist.
        assert_eq!(result.stats.character_rows, 1);
        assert_eq!(result.frame.characters[0].char_id, 777);
        assert_eq!(result.frame.characters[0].country_tag_db_id, 9);
        assert_eq!(result.frame.characters[0].name, None);
        assert_eq!(result.frame.characters[0].token, None);
        // Not filed as dead or torn: nothing failed to read.
        assert_eq!(result.stats.dead_roots, 0);
        assert_eq!(result.stats.rejected_roots, 0);
        assert_eq!(result.stats.identity_mismatch_rows, 0);
        // And NOT a rejection: `has_rejections` makes the whole domain-47
        // frame rowless, which would trade ~13_000 good rows for one
        // recycled character. The thinning is declared on the status word
        // instead.
        //
        // MUTATION: put `identity_degraded_rows` into `has_rejections` and
        // this fails.
        assert!(!result.stats.has_rejections());
        assert_ne!(result.frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
    }

    /// The frame a recycled character produces must SURVIVE THE DECODER
    /// (task #131, owner-ruled 2026-08-07).
    ///
    /// This is the test the sibling above could not be: that one asserts the
    /// status word, and the status word alone was accepted by the decoder until
    /// per-family admission arrived. A degraded identity is row-level loss
    /// inside families this frame observed IN FULL, so nothing is declared and
    /// `decode_family_presence_mask` refuses a payload whose only failure-class
    /// bit is `PARTIAL_DATA` — "nothing names what it failed to observe". That
    /// refusal is a parse error, and a parse error on a d47 frame invalidates
    /// the root cache and forces a full serial prewarm rebuild the next frame.
    /// So a single recycled character used to cost a whole prewarm.
    ///
    /// WHAT THIS TEST DOES NOT CLAIM: that the frame keeps its rows. It does
    /// not — SOURCE_UNAVAILABLE is deliberately outside d47's expected-partial
    /// tolerance (see the tolerance entry's own wording) and the scheduled
    /// reader replaces the frame with a rowless one either way. The defect
    /// being fixed is the parse error and the prewarm rebuild behind it.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: drop
    /// `| compact::STATUS_SOURCE_UNAVAILABLE` from the `identity_degraded_rows`
    /// arm of `build_lane_frame_from_adapter_input` and the decode below
    /// reports `character lane declares partial data with no family-presence
    /// declaration`. (Verified by mutation, lane DR.)
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
        // The two flags this frame must carry, and why each one is needed: the
        // first says the frame is thinner than a clean one, the second says the
        // thinning is NOT a declared family gap and so may not be absorbed by
        // d47's expected-partial tolerance.
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
        // On a ROW-BEARING payload — a test that decoded an empty frame clean
        // would prove nothing about the frame this defect actually damaged.
        assert_eq!(result.stats.character_rows, 1);
        assert!(!result.frame.characters.is_empty());
    }

    /// A character that CHANGED COUNTRY since the census is not a recycled
    /// object, and must keep its name and token.
    ///
    /// Annexations and transfers move characters between countries while the
    /// game runs, and the prewarm and the walk are two points inside one
    /// frame of a running game, so this is an ordinary state and not a corner
    /// one. Treating it as reuse would silently strip the text off every
    /// character caught by an annexation, and (before the degrade/refuse
    /// split) would have made the whole domain-47 frame rowless.
    ///
    /// MUTATION: add `&& raw.country_tag_db_id == root.country_tag_db_id`
    /// back into `identity_matches` — restoring the country to the root — and
    /// this fails. (Codex 019fd6ea, finding 2.)
    #[test]
    fn a_character_that_changed_country_since_the_census_keeps_its_text() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        // The census saw char 100 under country 2; it is country 3's now.
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
        // The country is this frame's, and the text is still the character's.
        assert_eq!(result.frame.characters[0].country_tag_db_id, 3);
        assert_eq!(
            result.frame.characters[0].name.as_deref(),
            Some("Maxwell Aitken")
        );
    }

    /// The control: an identity that still agrees keeps its text, so the
    /// tests above are not passing because the text is always dropped.
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

    /// A country-leader role row is a (country, char_id) PAIR whose halves
    /// come from two different observations: the country from the census, the
    /// char id from this frame. They must be compared before being combined.
    ///
    /// MUTATION: delete the `char_id` comparison in `read_country_leader_role`
    /// and this fails — a role row is emitted pairing char 777 with country 2,
    /// a combination that never existed in the engine.
    #[test]
    fn a_recycled_country_leader_character_refuses_the_role_row() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        let country_leader = 0x3000;
        // The census read char 100 through this role. A different character
        // is behind the pointer now.
        write_character(&mut mem, character, 777, 2);
        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF, character);

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
        // A recycled object reads cleanly. Filing it as "stably dead" would
        // claim the role no longer exists, which is a different and unproven
        // statement.
        assert_eq!(result.stats.dead_roots, 0);
        assert_eq!(result.stats.rejected_roots, 0);
        assert!(result.stats.has_rejections());
        assert_ne!(result.frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
    }

    /// A role whose character changed country is emitted under the country it
    /// is in NOW, not the census's and not refused.
    ///
    /// The role row used to take its country from the census, so before this
    /// change the row published a stale country with a current char id and
    /// nothing compared the two. Taking the country from the same live read
    /// that produced the char id removes the staleness instead of detecting
    /// it, and a legitimate transfer costs no row at all.
    ///
    /// MUTATION: emit `root.country_tag_db_id` in `read_country_leader_role`
    /// — restoring the field to the root — and this fails with country 2.
    #[test]
    fn a_country_leader_whose_character_changed_country_is_emitted_under_the_live_country() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        let country_leader = 0x3000;
        // The census read this role under country 2; it is country 3's now.
        write_character(&mut mem, character, 100, 3);
        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF, character);

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

    /// The control for the role path.
    #[test]
    fn an_unchanged_country_leader_role_is_emitted() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        let country_leader = 0x3000;
        write_character(&mut mem, character, 100, 2);
        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF, character);

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
}
