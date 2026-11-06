//! Shared recorded country access and intel branches for map and equipment queries.
#[cfg(test)]
use replay_frame::frame::DecodedFrame;

use crate::{
    frame::{DecodedSource, Tables},
    rows::{
        CountryIntelRow, Recorded, RelationLinkRow, ReservedCountryNamingRow, ZeroSubjectInputs,
        tables,
    },
};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntelBranch {
    Army,
    Navy,
    Air,
}
impl IntelBranch {
    fn raw(self, row: &CountryIntelRow) -> i64 {
        match self {
            Self::Army => row.army,
            Self::Navy => row.navy,
            Self::Air => row.air,
        }
    }
}

/// Equipment-detail reveal is an observer/target query, never an observer/design
/// matrix. Explicit bypass and disabled fog compare the full fraction against
/// the threshold, even when a mod sets a threshold above full visibility.
#[allow(
    clippy::too_many_arguments,
    reason = "Keep observer/target visibility gates and content thresholds explicit without an observer/design state matrix."
)]
pub fn design_details(
    frame: &dyn DecodedSource,
    links: &BTreeMap<u32, u32>,
    observer: u32,
    target: u32,
    fog: Result<bool, &'static str>,
    bypass: bool,
    branch: IntelBranch,
    maximum: i64,
    threshold: i64,
) -> Result<bool, &'static str> {
    if bypass || fog == Ok(false) {
        return Ok(threshold <= 100_000);
    }
    if event_shared(frame, links, observer, target, false)? {
        return Ok(threshold <= 100_000);
    }
    fog?;
    // The native intel getter tests the signed raw target before indexing.
    let raw = if target as i32 <= 0 {
        0
    } else {
        let owner = map_intel_slot(links, observer)?;
        let target_slot = map_intel_slot(links, target)?;
        let intel = Intel {
            rows: complete::<tables::CountryIntel>(frame, "CountryIntel"),
            raws: raw_references(links),
        };
        if owner == 0 {
            intel.zero_channel(frame, target, target_slot, branch)?
        } else {
            intel.channel(owner, target_slot, branch)?
        }
    };
    Ok(threshold <= intel_fraction(raw, maximum)?)
}
fn link(links: &BTreeMap<u32, u32>, raw: u32, what: &'static str) -> Result<u32, &'static str> {
    links.get(&raw).copied().ok_or(what)
}
fn complete<'a, T: crate::rows::TableSchema>(
    frame: &'a dyn DecodedSource,
    name: &'static str,
) -> Result<&'a [T::Row], &'static str> {
    frame.complete::<T>().map_err(|_| name)
}
/// Each row of `T` when complete, wherever `frame` holds it; refused as `name` otherwise.
fn each<T: crate::rows::TableSchema, F: Tables + ?Sized>(
    frame: &F,
    name: &'static str,
    mut row: impl FnMut(&T::Row) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    for held in frame
        .complete_rows::<T>(T::ID.full_mask())
        .map_err(|_| name)?
        .iter()
    {
        row(&*held.map_err(|_| name)?)?;
    }
    Ok(())
}
/// A zero maximum or reserved raw lane produces zero-extended -1, exceeding every cutoff.
pub fn intel_fraction(raw: i64, maximum: i64) -> Result<i64, &'static str> {
    if maximum == 0 || matches!(raw, i64::MAX | i64::MIN) || raw == i64::MIN + 1 {
        Ok(i64::from(u32::MAX))
    } else {
        raw.wrapping_mul(100_000)
            .checked_div(maximum)
            .ok_or("visibility intel division overflow")
    }
}

/// One owner's `CountryIntel` rows. Decoded rows are in key order
/// (owner reference, then target slot), so a binary search finds the slice.
pub fn owner_rows(rows: &[CountryIntelRow], owner: u32) -> &[CountryIntelRow] {
    let start = rows.partition_point(|row| row.owner_country < owner);
    let end = rows.partition_point(|row| row.owner_country <= owner);
    &rows[start..end]
}

/// Raw country references per canonical slot, for reading keyed rows.
pub fn raw_references(links: &BTreeMap<u32, u32>) -> BTreeMap<u32, Vec<u32>> {
    let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&raw, &slot) in links {
        out.entry(slot).or_default().push(raw);
    }
    out
}

/// Army lanes read on demand from the key-sorted table, never copied.
pub struct Intel<'a> {
    pub rows: Result<&'a [CountryIntelRow], &'static str>,
    pub raws: BTreeMap<u32, Vec<u32>>,
}

impl Intel<'_> {
    pub fn army(&self, owner: u32, target: u32) -> Result<i64, &'static str> {
        self.channel(owner, target, IntelBranch::Army)
    }
    pub fn channel(
        &self,
        owner: u32,
        target: u32,
        branch: IntelBranch,
    ) -> Result<i64, &'static str> {
        let rows = self.rows?;
        Ok(self
            .raws
            .get(&owner)
            .into_iter()
            .flatten()
            .find_map(|&raw| {
                let rows = owner_rows(rows, raw);
                rows.binary_search_by_key(&target, |row| row.target_index)
                    .ok()
                    .map(|index| branch.raw(&rows[index]))
            })
            .unwrap_or(0))
    }

    /// Actual receiver0's native getter, not a positive alias's keyed rows.
    /// The signed raw gate precedes its daily sample and vector demand.
    pub fn zero_army(
        &self,
        frame: &dyn DecodedSource,
        raw_target: u32,
        target: u32,
    ) -> Result<i64, &'static str> {
        self.zero_channel(frame, raw_target, target, IntelBranch::Army)
    }
    pub fn zero_channel(
        &self,
        frame: &dyn DecodedSource,
        raw_target: u32,
        target: u32,
        branch: IntelBranch,
    ) -> Result<i64, &'static str> {
        if raw_target as i32 <= 0 {
            return Ok(0);
        }
        if target > i32::MAX as u32 {
            return Err("CountryIntel owner0 target outside native index range");
        }
        let reserved = reserved(frame)?;
        let sample = match &reserved.country_intel_sample {
            Recorded::NotRecorded => {
                return Err("ReservedCountryNaming.country_intel_sample not recorded");
            }
            Recorded::Value(None) => {
                return Err("ReservedCountryNaming.country_intel_sample capture refused");
            }
            Recorded::Value(Some(sample)) => sample,
        };
        if sample.sample_hour > frame.hour() {
            return Err("ReservedCountryNaming.country_intel_sample is later than frame");
        }
        if sample.vector_count > i32::MAX as u32 {
            return Err("ReservedCountryNaming.country_intel_sample count outside native range");
        }
        let rows = owner_rows(self.rows?, 0);
        if rows
            .iter()
            .any(|row| row.target_index >= sample.vector_count)
            || rows
                .windows(2)
                .any(|pair| pair[0].target_index >= pair[1].target_index)
        {
            return Err("CountryIntel owner0 keys outside admitted vector or duplicated");
        }
        if sample.vector_count > 0
            && rows.last().map(|row| row.target_index) != Some(sample.vector_count - 1)
        {
            return Err("CountryIntel owner0 final sentinel missing");
        }
        if target >= sample.vector_count {
            return Ok(0);
        }
        Ok(rows
            .binary_search_by_key(&target, |row| row.target_index)
            .ok()
            .map_or(0, |index| branch.raw(&rows[index])))
    }
}

pub fn reserved(frame: &dyn DecodedSource) -> Result<&ReservedCountryNamingRow, &'static str> {
    let [row] = complete::<tables::ReservedCountryNaming>(frame, "ReservedCountryNaming")? else {
        return Err("ReservedCountryNaming singleton");
    };
    Ok(row)
}

/// Only the native MapIntel boundary admits literal0. General link readers
/// remain strict, and an unrecorded positive alias never becomes Country0.
pub fn map_intel_slot(links: &BTreeMap<u32, u32>, raw: u32) -> Result<u32, &'static str> {
    if raw == 0 {
        Ok(0)
    } else {
        link(links, raw, "visibility country link")
    }
}

pub fn map_intel_equal(links: &BTreeMap<u32, u32>, a: u32, b: u32) -> Result<bool, &'static str> {
    Ok(a == b || (a != 0 && b != 0 && map_intel_slot(links, a)? == map_intel_slot(links, b)?))
}

pub fn active_war(war: [u8; 2]) -> Result<bool, &'static str> {
    let [present, retired] = war;
    if present > 1 || (present == 0 && retired != 0) {
        return Err("visibility war has invalid native presence");
    }
    Ok(present == 1 && retired == 0)
}

/// Complete same-hour membership is demanded only after the war veto and a
/// nonnull source faction. Missing target membership then means native null.
pub fn map_intel_faction(
    frame: &dyn DecodedSource,
    links: &BTreeMap<u32, u32>,
    target: u32,
) -> Result<Option<u64>, &'static str> {
    let mut faction = None;
    for row in complete::<tables::FactionMembers>(frame, "FactionMembers")? {
        let country = map_intel_slot(links, row.country)?;
        if row.faction == 0 {
            return Err("FactionMembers has null faction identity");
        }
        if country == target {
            if faction.is_some_and(|previous| previous != row.faction) {
                return Err("FactionMembers has inconsistent faction identity");
            }
            faction = Some(row.faction);
        }
    }
    Ok(faction)
}

pub fn reserved_faction(row: &ReservedCountryNamingRow) -> Result<u64, &'static str> {
    match row.faction {
        Recorded::NotRecorded => Err("ReservedCountryNaming.faction not recorded"),
        Recorded::Value(None) => Err("ReservedCountryNaming.faction capture refused"),
        Recorded::Value(Some(faction)) => Ok(faction),
    }
}

/// Country0's fresh reverse-visibility cell toward `target`.
fn reverse_cell<'a>(
    frame: &dyn DecodedSource,
    row: &'a ReservedCountryNamingRow,
    target: u32,
) -> Result<&'a crate::rows::ReverseZeroVisibilityInputs, &'static str> {
    let packet = match &row.reverse_visibility {
        Recorded::NotRecorded => {
            return Err("ReservedCountryNaming.reverse_visibility not recorded");
        }
        Recorded::Value(None) => {
            return Err("ReservedCountryNaming.reverse_visibility capture refused");
        }
        Recorded::Value(Some(packet)) => packet,
    };
    if !packet.sample_hour.fresh_at(frame.hour()) {
        return Err("ReservedCountryNaming.reverse_visibility sample hour mismatch");
    }
    packet
        .cells
        .get(target as usize)
        .and_then(Option::as_ref)
        .ok_or("ReservedCountryNaming.reverse_visibility cell capture refused")
}

/// Test subject parties in order against the reached cell's raw target. Null or wrong-type
/// subjects decide before requiring that target. A known second match can preserve truth after an
/// unavailable first party.
pub fn map_intel_subject(
    subject: &ZeroSubjectInputs,
    links: &BTreeMap<u32, u32>,
    target: Result<u32, &'static str>,
) -> Result<bool, &'static str> {
    let parties = match (subject.relation_type, subject.parties) {
        (None, None) => return Ok(false),
        (Some(kind), None) if kind != engine_native::csubject_relation::TYPE_VALUE => {
            return Ok(false);
        }
        (Some(kind), Some(parties)) if kind == engine_native::csubject_relation::TYPE_VALUE => {
            parties
        }
        _ => return Err("visibility subject has inconsistent native branches"),
    };
    let matches = |party: Option<u32>| {
        map_intel_equal(
            links,
            party.ok_or("visibility subject party capture refused")?,
            target?,
        )
    };
    let first = matches(parties[0]);
    if first == Ok(true) {
        return Ok(true);
    }
    let second = matches(parties[1]);
    if second == Ok(true) {
        return Ok(true);
    }
    first?;
    second
}

/// Physical receiver0, : no symmetric relation-table inference.
pub fn reverse_zero_shared(
    frame: &dyn DecodedSource,
    links: &BTreeMap<u32, u32>,
    target: u32,
) -> Result<bool, &'static str> {
    let reserved = reserved(frame)?;
    let cell = reverse_cell(frame, reserved, target)?;
    if active_war(
        cell.war
            .ok_or("ReservedCountryNaming.reverse_visibility war capture refused")?,
    )? {
        return Ok(false);
    }
    let faction = reserved_faction(reserved)?;
    if faction != 0 && (target == 0 || map_intel_faction(frame, links, target)? == Some(faction)) {
        return Ok(true);
    }
    let cell_target = cell
        .target
        .ok_or("ReservedCountryNaming.reverse_visibility target capture refused");
    let subject = cell
        .subject
        .as_ref()
        .ok_or("ReservedCountryNaming.reverse_visibility subject capture refused")?;
    map_intel_subject(subject, links, cell_target)
}

/// Prefer the current positive receiver's directed status0 packet. Only older
/// field absence keeps the established exact-zero publication guard; a fresh
/// refused packet never falls back to the independent legacy observations.
pub fn forward_zero_shared(
    frame: &dyn DecodedSource,
    links: &BTreeMap<u32, u32>,
    owner: u32,
) -> Result<bool, &'static str> {
    let mut rows = complete::<tables::CountryCounterInputs>(frame, "CountryCounterInputs")?
        .iter()
        .filter(|row| links.get(&row.country) == Some(&owner));
    let row = rows
        .next()
        .ok_or("CountryCounterInputs visibility owner missing")?;
    if rows.next().is_some() {
        return Err("CountryCounterInputs visibility owner duplicated");
    }
    if !matches!(row.forward_visibility, Recorded::NotRecorded) {
        return forward_visibility_shared(frame, links, owner, &row.forward_visibility, false);
    }
    let war = match row.zero_war {
        Recorded::NotRecorded => return Err("CountryCounterInputs.zero_war not recorded"),
        Recorded::Value(None) => return Err("CountryCounterInputs.zero_war capture refused"),
        Recorded::Value(Some(war)) => war,
    };
    if active_war(war)? {
        return Ok(false);
    }
    if let Some(faction) = map_intel_faction(frame, links, owner)?
        && reserved_faction(reserved(frame)?)? == faction
    {
        return Ok(true);
    }
    let subject = match &row.zero_subject {
        Recorded::NotRecorded => return Err("CountryCounterInputs.zero_subject not recorded"),
        Recorded::Value(None) => return Err("CountryCounterInputs.zero_subject capture refused"),
        Recorded::Value(Some(subject)) => subject,
    };
    map_intel_subject(subject, links, Ok(0))
}

/// A positive receiver's visibility matrix from this complete hourly table. The cell's raw target
/// includes positive aliases of Country0. War and military admission precede any demand for the
/// target or subject.
pub fn forward_visibility_shared(
    frame: &dyn DecodedSource,
    links: &BTreeMap<u32, u32>,
    owner: u32,
    inputs: &Recorded<Option<crate::rows::ReverseZeroVisibilityInputs>>,
    military: bool,
) -> Result<bool, &'static str> {
    let cell = match inputs {
        Recorded::NotRecorded => {
            return Err("CountryCounterInputs.forward_visibility not recorded");
        }
        Recorded::Value(None) => {
            return Err("CountryCounterInputs.forward_visibility capture refused");
        }
        Recorded::Value(Some(cell)) => cell,
    };
    if active_war(
        cell.war
            .ok_or("CountryCounterInputs.forward_visibility war capture refused")?,
    )? {
        return Ok(false);
    }
    let access = if military {
        match cell.military_access {
            Some(1) => Ok(true),
            Some(0) => Ok(false),
            None => Err("CountryCounterInputs.forward_visibility military access capture refused"),
            Some(_) => {
                return Err("CountryCounterInputs.forward_visibility military access invalid");
            }
        }
    } else {
        Ok(false)
    };
    event_or(access, || {
        let faction = (|| {
            let Some(faction) = map_intel_faction(frame, links, owner)? else {
                return Ok(false);
            };
            Ok(reserved_faction(reserved(frame)?)? == faction)
        })();
        event_or(faction, || {
            let target = cell
                .target
                .ok_or("CountryCounterInputs.forward_visibility target capture refused");
            let subject = cell
                .subject
                .as_ref()
                .ok_or("CountryCounterInputs.forward_visibility subject capture refused")?;
            map_intel_subject(subject, links, target)
        })
    })
}

/// Map-text visibility admits paradrops through directed military access and disables docking.
/// Receiver comparison uses the resolved object's own ID. Actual Country0 therefore requires its
/// first matrix for a positive target alias of Country0, even when incoming aliases are equal.
pub fn event_shared(
    frame: &dyn DecodedSource,
    links: &BTreeMap<u32, u32>,
    receiver: u32,
    raw_target: u32,
    military: bool,
) -> Result<bool, &'static str> {
    let owner = map_intel_slot(links, receiver)?;
    let own_id = if owner == 0 {
        0
    } else {
        let mut rows = complete::<tables::Countries>(frame, "Countries")?
            .iter()
            .filter(|row| row.canonical_slot == owner);
        let row = rows
            .next()
            .ok_or("map text receiver Countries row missing")?;
        if rows.next().is_some() {
            return Err("map text receiver Countries row duplicated");
        }
        row.id
    };
    if own_id == raw_target {
        return Ok(true);
    }
    let target = map_intel_slot(links, raw_target)?;
    if own_id != 0 && raw_target != 0 && owner == target {
        return Ok(true);
    }
    if owner == 0 {
        let row = reserved(frame)?;
        let cell = reverse_cell(frame, row, target)?;
        if active_war(
            cell.war
                .ok_or("ReservedCountryNaming.reverse_visibility war capture refused")?,
        )? {
            return Ok(false);
        }
        let access = if military {
            match cell.military_access {
                Some(1) => Ok(true),
                Some(0) => Ok(false),
                Some(_) => {
                    return Err("ReservedCountryNaming.reverse_visibility military access invalid");
                }
                None => {
                    Err("ReservedCountryNaming.reverse_visibility military access capture refused")
                }
            }
        } else {
            Ok(false)
        };
        return event_or(access, || {
            let faction = (|| {
                let faction = reserved_faction(row)?;
                Ok(faction != 0
                    && (target == 0 || map_intel_faction(frame, links, target)? == Some(faction)))
            })();
            event_or(faction, || {
                let target = cell
                    .target
                    .ok_or("ReservedCountryNaming.reverse_visibility target capture refused");
                let subject = cell
                    .subject
                    .as_ref()
                    .ok_or("ReservedCountryNaming.reverse_visibility subject capture refused")?;
                map_intel_subject(subject, links, target)
            })
        });
    }
    if target == 0 {
        if !military {
            return forward_zero_shared(frame, links, owner);
        }
        let mut rows = complete::<tables::CountryCounterInputs>(frame, "CountryCounterInputs")?
            .iter()
            .filter(|row| links.get(&row.country) == Some(&owner));
        let row = rows
            .next()
            .ok_or("CountryCounterInputs visibility owner missing")?;
        if rows.next().is_some() {
            return Err("CountryCounterInputs visibility owner duplicated");
        }
        return forward_visibility_shared(frame, links, owner, &row.forward_visibility, true);
    }
    for row in complete::<tables::WarRelations>(frame, "WarRelations")? {
        let a = map_intel_slot(links, row.first_country)?;
        let b = map_intel_slot(links, row.second_country)?;
        if (a, b) == (owner, target) || (a, b) == (target, owner) {
            return Ok(false);
        }
    }
    let access = if military {
        (|| {
            for row in complete::<tables::RelationLinks>(frame, "RelationLinks")? {
                if row.target_index == target
                    && map_intel_slot(links, row.country)? == owner
                    && row.links & RelationLinkRow::MILITARY_ACCESS != 0
                {
                    return Ok(true);
                }
            }
            Ok(false)
        })()
    } else {
        Ok(false)
    };
    event_or(access, || {
        let faction = (|| {
            let Some(faction) = map_intel_faction(frame, links, owner)? else {
                return Ok(false);
            };
            Ok(map_intel_faction(frame, links, target)? == Some(faction))
        })();
        event_or(faction, || {
            for row in complete::<tables::Subjects>(frame, "Subjects")? {
                let a = map_intel_slot(links, row.overlord)?;
                let b = map_intel_slot(links, row.subject)?;
                if (a, b) == (owner, target) || (a, b) == (target, owner) {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    })
}

/// Ordered native OR after a known clear war veto. A skipped/refused earlier
/// operand cannot erase an independently proven positive later branch.
pub fn event_or(
    first: Result<bool, &'static str>,
    second: impl FnOnce() -> Result<bool, &'static str>,
) -> Result<bool, &'static str> {
    if first == Ok(true) {
        return Ok(true);
    }
    let second = second();
    if second == Ok(true) {
        return Ok(true);
    }
    first?;
    second
}

/// Faction, subject and attache visibility inputs in canonical slots, without wars. Subjects use
/// recorded installed typed pointers without retirement or date filtering; RelationLinks records
/// attache membership. Callers with their own war census can read these inputs independently.
#[derive(Clone, Default)]
pub struct Families {
    factions: BTreeMap<u32, u64>,
    subjects: BTreeSet<(u32, u32)>,
    attaches: BTreeSet<(u32, u32)>,
}

impl Families {
    /// Faction and subject inputs for full-country visibility.
    pub fn read(
        frame: &dyn DecodedSource,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        let mut out = Self::default();
        out.read_factions(frame, links)?;
        out.read_subjects(frame, links)?;
        Ok(out)
    }

    pub fn read_factions(
        &mut self,
        frame: &dyn DecodedSource,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        for row in complete::<tables::FactionMembers>(frame, "FactionMembers")? {
            // Faction rosters include genuine Country0 members. Recognize the literal only here; unknown
            // positive links remain failures.
            let country = if row.country == 0 {
                0
            } else {
                link(links, row.country, "FactionMembers country")?
            };
            self.factions.insert(country, row.faction);
        }
        Ok(())
    }

    pub fn read_subjects(
        &mut self,
        frame: &dyn DecodedSource,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        for row in complete::<tables::Subjects>(frame, "Subjects")? {
            let (a, b) = (
                link(links, row.overlord, "Subjects country")?,
                link(links, row.subject, "Subjects country")?,
            );
            self.subjects.extend([(a, b), (b, a)]);
        }
        Ok(())
    }

    /// [`Self::read_factions`] of any frame's tables, a walk's views among them.
    pub fn read_factions_tables<F: Tables + ?Sized>(
        &mut self,
        frame: &F,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        each::<tables::FactionMembers, F>(frame, "FactionMembers", |row| {
            let country = if row.country == 0 {
                0
            } else {
                link(links, row.country, "FactionMembers country")?
            };
            self.factions.insert(country, row.faction);
            Ok(())
        })
    }

    /// [`Self::read_subjects`] of any frame's tables, a walk's views among them.
    pub fn read_subjects_tables<F: Tables + ?Sized>(
        &mut self,
        frame: &F,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        each::<tables::Subjects, F>(frame, "Subjects", |row| {
            let (a, b) = (
                link(links, row.overlord, "Subjects country")?,
                link(links, row.subject, "Subjects country")?,
            );
            self.subjects.extend([(a, b), (b, a)]);
            Ok(())
        })
    }

    /// Relations where each pair shares a faction of its own.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn factions_for_tests(pairs: &[(u32, u32)]) -> Self {
        let mut out = Self::default();
        for (faction, &(a, b)) in (1..).zip(pairs) {
            out.factions.extend([(a, faction), (b, faction)]);
        }
        out
    }

    /// Also admits recorded attache country slots.
    pub fn with_attaches(
        mut self,
        frame: &dyn DecodedSource,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        for row in complete::<tables::RelationLinks>(frame, "RelationLinks")? {
            if row.links & RelationLinkRow::ATTACHE != 0 {
                let owner = *links.get(&row.country).ok_or("RelationLinks country")?;
                self.attaches.insert((owner, row.target_index));
            }
        }
        Ok(self)
    }

    /// The same country, a faction partner, an overlord or a subject, before the live-war veto.
    pub fn family(&self, a: u32, b: u32) -> bool {
        a == b || self.subjects.contains(&(a, b)) || self.same_faction(a, b)
    }

    /// one non-null faction.
    pub fn same_faction(&self, a: u32, b: u32) -> bool {
        self.factions
            .get(&a)
            .is_some_and(|&f| self.member_of_faction(b, f))
    }

    /// Scan the target's exact faction roster.
    pub fn member_of_faction(&self, country: u32, faction: u64) -> bool {
        self.factions.get(&country) == Some(&faction)
    }

    /// Counter sharing excludes the null faction, unlike full-country sharing.
    pub fn counter_shared(&self, a: u32, b: u32) -> bool {
        self.subjects.contains(&(a, b))
            || self
                .factions
                .get(&a)
                .is_some_and(|faction| *faction != 0 && self.factions.get(&b) == Some(faction))
    }

    /// Attache sharing has no war veto and uses the source's directed attache slot toward the viewer.
    pub fn allied_for_fog(&self, source: u32, viewer: u32) -> bool {
        self.family(source, viewer) || self.attaches.contains(&(source, viewer))
    }
}

/// [`Families`] with the recorded wars whose retired byte is zero.
#[derive(Clone, Default)]
pub struct Relations {
    families: Families,
    wars: BTreeSet<(u32, u32)>,
}

impl Relations {
    /// Relations at peace where each pair shares a faction of its own.
    #[cfg(any(test, feature = "test-support"))]
    pub fn factions_for_tests(pairs: &[(u32, u32)]) -> Self {
        Self {
            families: Families::factions_for_tests(pairs),
            wars: BTreeSet::new(),
        }
    }

    /// Faction, war and subject inputs for full-country visibility.
    pub fn read(
        frame: &dyn DecodedSource,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        let mut out = Self::default();
        out.families.read_factions(frame, links)?;
        for row in complete::<tables::WarRelations>(frame, "WarRelations")? {
            let (a, b) = (
                link(links, row.first_country, "WarRelations country")?,
                link(links, row.second_country, "WarRelations country")?,
            );
            out.wars.extend([(a, b), (b, a)]);
        }
        out.families.read_subjects(frame, links)?;
        Ok(out)
    }

    /// Also admits recorded attache country slots.
    pub fn with_attaches(
        mut self,
        frame: &dyn DecodedSource,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        self.families = self.families.with_attaches(frame, links)?;
        Ok(self)
    }

    /// [`Self::read`] of any frame's tables, a walk's views among them.
    pub fn read_tables<F: Tables + ?Sized>(
        frame: &F,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        let mut out = Self::default();
        out.families.read_factions_tables(frame, links)?;
        each::<tables::WarRelations, F>(frame, "WarRelations", |row| {
            let (a, b) = (
                link(links, row.first_country, "WarRelations country")?,
                link(links, row.second_country, "WarRelations country")?,
            );
            out.wars.extend([(a, b), (b, a)]);
            Ok(())
        })?;
        out.families.read_subjects_tables(frame, links)?;
        Ok(out)
    }

    /// Distinct countries with an active war relation.
    pub fn at_war(&self, a: u32, b: u32) -> bool {
        a != b && self.wars.contains(&(a, b))
    }

    /// The countries in an active war relation with `a`.
    pub fn enemies(&self, a: u32) -> impl Iterator<Item = u32> + '_ {
        self.wars.range((a, 0)..=(a, u32::MAX)).map(|&(_, b)| b)
    }

    /// See [`Families::same_faction`].
    pub fn same_faction(&self, a: u32, b: u32) -> bool {
        self.families.same_faction(a, b)
    }

    /// Full visibility is vetoed by a live war; otherwise it admits the same country, a faction
    /// partner, an overlord or a subject.
    pub fn shared(&self, a: u32, b: u32) -> bool {
        a == b || (!self.wars.contains(&(a, b)) && self.families.family(a, b))
    }

    /// See [`Families::allied_for_fog`].
    pub fn allied_for_fog(&self, source: u32, viewer: u32) -> bool {
        self.families.allied_for_fog(source, viewer)
    }
}

/// Directed military access and docking rights from `RelationLinks`, in canonical `(holder, target)`
/// order.
#[derive(Clone, Debug, Default)]
pub struct AccessLinks {
    military: BTreeSet<(u32, u32)>,
    docking: BTreeSet<(u32, u32)>,
}

impl AccessLinks {
    /// [`Self::read`] of any frame's tables, a walk's views among them.
    pub fn read_tables<F: Tables + ?Sized>(
        frame: &F,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        let mut out = Self::default();
        each::<tables::RelationLinks, F>(frame, "RelationLinks", |row| {
            let holder = *links.get(&row.country).ok_or("RelationLinks country")?;
            if row.links & RelationLinkRow::MILITARY_ACCESS != 0 {
                out.military.insert((holder, row.target_index));
            }
            if row.links & RelationLinkRow::DOCKING_RIGHTS != 0 {
                out.docking.insert((holder, row.target_index));
            }
            Ok(())
        })?;
        Ok(out)
    }

    pub fn read(
        frame: &dyn DecodedSource,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        let mut out = Self::default();
        for row in complete::<tables::RelationLinks>(frame, "RelationLinks")? {
            let holder = *links.get(&row.country).ok_or("RelationLinks country")?;
            if row.links & RelationLinkRow::MILITARY_ACCESS != 0 {
                out.military.insert((holder, row.target_index));
            }
            if row.links & RelationLinkRow::DOCKING_RIGHTS != 0 {
                out.docking.insert((holder, row.target_index));
            }
        }
        Ok(out)
    }

    pub fn military(&self, holder: u32, target: u32) -> bool {
        self.military.contains(&(holder, target))
    }

    pub fn docking(&self, holder: u32, target: u32) -> bool {
        self.docking.contains(&(holder, target))
    }
}

impl Relations {
    /// A live war vetoes visibility. Otherwise admit military access or docking rights from the viewer
    /// toward the owner, faction membership, or overlord/subject relations.
    pub fn known_port(&self, access: &AccessLinks, viewer: u32, owner: u32) -> bool {
        viewer == owner
            || (!self.wars.contains(&(viewer, owner))
                && (access.military(viewer, owner)
                    || access.docking(viewer, owner)
                    || self.families.family(viewer, owner)))
    }

    /// Province access excludes lakes. It admits sea provinces; unowned states; the viewer's own states
    /// or controlled provinces; war, military access or docking rights toward the controller; and
    /// shared faction or direct subject relations with the controller.
    pub fn province_access(
        &self,
        access: &AccessLinks,
        viewer: u32,
        province: ProvinceAccess,
    ) -> bool {
        let ProvinceAccess {
            lake,
            sea,
            state_owner,
            controller,
        } = province;
        if lake {
            return false;
        }
        if state_owner.is_none_or(|owner| owner == viewer) || controller == Some(viewer) {
            return true;
        }
        let Some(controller) = controller else {
            return sea;
        };
        self.at_war(viewer, controller)
            || access.military(viewer, controller)
            || access.docking(viewer, controller)
            || sea
            || self.same_faction(viewer, controller)
            || self.families.subjects.contains(&(viewer, controller))
    }
}

/// Province access inputs: definition flags, state owner (`None` for no owner), and controller
/// (`None` for none).
#[derive(Clone, Copy, Debug, Default)]
pub struct ProvinceAccess {
    pub lake: bool,
    pub sea: bool,
    pub state_owner: Option<u32>,
    pub controller: Option<u32>,
}
