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
pub fn intel_fraction(raw: i64, maximum: i64) -> Result<i64, &'static str> {
    if maximum == 0 || matches!(raw, i64::MAX | i64::MIN) || raw == i64::MIN + 1 {
        Ok(i64::from(u32::MAX))
    } else {
        raw.wrapping_mul(100_000)
            .checked_div(maximum)
            .ok_or("visibility intel division overflow")
    }
}

pub fn owner_rows(rows: &[CountryIntelRow], owner: u32) -> &[CountryIntelRow] {
    let start = rows.partition_point(|row| row.owner_country < owner);
    let end = rows.partition_point(|row| row.owner_country <= owner);
    &rows[start..end]
}

pub fn raw_references(links: &BTreeMap<u32, u32>) -> BTreeMap<u32, Vec<u32>> {
    let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&raw, &slot) in links {
        out.entry(slot).or_default().push(raw);
    }
    out
}

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

fn reserved_tables<F: Tables + ?Sized>(
    frame: &F,
) -> Result<(Result<u64, &'static str>, Result<crate::rows::ReservedReverseVisibilityInputs, &'static str>), &'static str> {
    let mut found = None;
    each::<tables::ReservedCountryNaming, F>(frame, "ReservedCountryNaming", |row| {
        if found.is_some() {
            return Err("ReservedCountryNaming singleton");
        }
        let faction = reserved_faction(row);
        let reverse = match &row.reverse_visibility {
            Recorded::NotRecorded => Err("ReservedCountryNaming.reverse_visibility not recorded"),
            Recorded::Value(None) => Err("ReservedCountryNaming.reverse_visibility capture refused"),
            Recorded::Value(Some(packet)) => {
                if !packet.sample_hour.fresh_at(frame.at_hour()) {
                    Err("ReservedCountryNaming.reverse_visibility sample hour mismatch")
                } else {
                    Ok(packet.clone())
                }
            }
        };
        found = Some((faction, reverse));
        Ok(())
    })?;
    found.ok_or("ReservedCountryNaming singleton")
}

fn map_intel_factions_tables<F: Tables + ?Sized>(
    frame: &F,
    links: &BTreeMap<u32, u32>,
) -> Result<BTreeMap<u32, Result<u64, &'static str>>, &'static str> {
    let mut out: BTreeMap<u32, Result<u64, &'static str>> = BTreeMap::new();
    each::<tables::FactionMembers, F>(frame, "FactionMembers", |row| {
        let country = map_intel_slot(links, row.country)?;
        if row.faction == 0 {
            return Err("FactionMembers has null faction identity");
        }
        match out.entry(country) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Ok(row.faction));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if *entry.get() != Ok(row.faction) {
                    *entry.get_mut() = Err("FactionMembers has inconsistent faction identity");
                }
            }
        }
        Ok(())
    })?;
    Ok(out)
}

#[derive(Clone)]
pub struct ZeroVisibility {
    forward: Result<BTreeMap<u32, Result<bool, &'static str>>, &'static str>,
    reverse: Result<Vec<Result<bool, &'static str>>, &'static str>,
    reverse_sample_hour: Option<crate::rows::PacketHour>,
}

impl ZeroVisibility {
    pub fn unavailable(reason: &'static str) -> Self {
        Self {
            forward: Err(reason),
            reverse: Err(reason),
            reverse_sample_hour: None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(forward: impl IntoIterator<Item = (u32, Result<bool, &'static str>)>) -> Self {
        Self {
            forward: Ok(forward.into_iter().collect()),
            reverse: Err("ReservedCountryNaming"),
            reverse_sample_hour: None,
        }
    }

    pub fn read_tables<F: Tables + ?Sized>(
        frame: &F,
        links: &BTreeMap<u32, u32>,
    ) -> Self {
        let factions = map_intel_factions_tables(frame, links);
        let reserved_row = reserved_tables(frame);
        let reserved_faction = match &reserved_row {
            Ok((faction, _)) => *faction,
            Err(reason) => Err(*reason),
        };
        let mut forward_map = BTreeMap::new();
        let forward = each::<tables::CountryCounterInputs, F>(
            frame,
            "CountryCounterInputs",
            |row| {
                let Some(&owner) = links.get(&row.country) else {
                    return Ok(());
                };
                let result = forward_zero_shared_of(
                    links,
                    &row.forward_visibility,
                    &row.zero_war,
                    &row.zero_subject,
                    || match &factions {
                        Ok(map) => match map.get(&owner) {
                            Some(&Ok(faction)) => Ok(Some(faction)),
                            Some(&Err(reason)) => Err(reason),
                            None => Ok(None),
                        },
                        Err(reason) => Err(reason),
                    },
                    || reserved_faction,
                );
                match forward_map.entry(owner) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(result);
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        *entry.get_mut() = Err("CountryCounterInputs visibility owner duplicated");
                    }
                }
                Ok(())
            },
        )
        .map(|()| forward_map);

        let (reverse, reverse_sample_hour) = match reserved_row {
            Ok((_, Ok(packet))) => {
                let sample_hour = packet.sample_hour;
                let cells = packet
                    .cells
                    .iter()
                    .enumerate()
                    .map(|(target, cell)| {
                        let target = target as u32;
                        let cell = cell
                            .as_ref()
                            .ok_or("ReservedCountryNaming.reverse_visibility cell capture refused")?;
                        if active_war(
                            cell.war
                                .ok_or("ReservedCountryNaming.reverse_visibility war capture refused")?,
                        )? {
                            return Ok(false);
                        }
                        let faction = (|| {
                            let faction = reserved_faction?;
                            if faction == 0 {
                                return Ok(false);
                            }
                            if target == 0 {
                                return Ok(true);
                            }
                            match &factions {
                                Ok(map) => match map.get(&target) {
                                    Some(&Ok(f)) => Ok(f == faction),
                                    Some(&Err(reason)) => Err(reason),
                                    None => Ok(false),
                                },
                                Err(reason) => Err(reason),
                            }
                        })();
                        event_or(faction, || {
                            let cell_target = cell.target.ok_or(
                                "ReservedCountryNaming.reverse_visibility target capture refused",
                            );
                            let subject = cell.subject.as_ref().ok_or(
                                "ReservedCountryNaming.reverse_visibility subject capture refused",
                            )?;
                            map_intel_subject(subject, links, cell_target)
                        })
                    })
                    .collect();
                (Ok(cells), Some(sample_hour))
            }
            Ok((_, Err(reason))) => (Err(reason), None),
            Err(reason) => (Err(reason), None),
        };

        Self {
            forward,
            reverse,
            reverse_sample_hour,
        }
    }

    pub fn reverse_stale_at(&self, hour: crate::Hour) -> bool {
        self.reverse_sample_hour
            .is_some_and(|sample_hour| !sample_hour.fresh_at(hour))
    }

    pub fn forward_shared(&self, owner: u32) -> Result<bool, &'static str> {
        self.forward
            .as_ref()
            .map_err(|reason| *reason)?
            .get(&owner)
            .copied()
            .unwrap_or(Err("CountryCounterInputs visibility owner missing"))
    }

    pub fn reverse_shared(&self, target: u32) -> Result<bool, &'static str> {
        self.reverse
            .as_ref()
            .map_err(|reason| *reason)?
            .get(target as usize)
            .copied()
            .unwrap_or(Err(
                "ReservedCountryNaming.reverse_visibility cell capture refused",
            ))
    }
}

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

pub fn map_intel_subject(
    subject: &ZeroSubjectInputs,
    links: &BTreeMap<u32, u32>,
    target: Result<u32, &'static str>,
) -> Result<bool, &'static str> {
    let parties = match (subject.relation_type, subject.parties) {
        (None, None) => return Ok(false),
        (Some(kind), None) if kind != engine_native::crelation::subject::TYPE_VALUE => {
            return Ok(false);
        }
        (Some(kind), Some(parties)) if kind == engine_native::crelation::subject::TYPE_VALUE => {
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
    forward_zero_shared_of(
        links,
        &row.forward_visibility,
        &row.zero_war,
        &row.zero_subject,
        || map_intel_faction(frame, links, owner),
        || reserved_faction(reserved(frame)?),
    )
}

pub fn forward_zero_shared_of(
    links: &BTreeMap<u32, u32>,
    forward: &Recorded<Option<crate::rows::ReverseZeroVisibilityInputs>>,
    zero_war: &Recorded<Option<[u8; 2]>>,
    zero_subject: &Recorded<Option<ZeroSubjectInputs>>,
    faction: impl Fn() -> Result<Option<u64>, &'static str>,
    reserved: impl Fn() -> Result<u64, &'static str>,
) -> Result<bool, &'static str> {
    if !matches!(forward, Recorded::NotRecorded) {
        return forward_visibility_shared_of(links, forward, false, || {
            let Some(faction) = faction()? else {
                return Ok(false);
            };
            Ok(reserved()? == faction)
        });
    }
    let war = match zero_war {
        Recorded::NotRecorded => return Err("CountryCounterInputs.zero_war not recorded"),
        Recorded::Value(None) => return Err("CountryCounterInputs.zero_war capture refused"),
        Recorded::Value(Some(war)) => *war,
    };
    if active_war(war)? {
        return Ok(false);
    }
    if let Some(faction) = faction()?
        && reserved()? == faction
    {
        return Ok(true);
    }
    let subject = match zero_subject {
        Recorded::NotRecorded => return Err("CountryCounterInputs.zero_subject not recorded"),
        Recorded::Value(None) => return Err("CountryCounterInputs.zero_subject capture refused"),
        Recorded::Value(Some(subject)) => subject,
    };
    map_intel_subject(subject, links, Ok(0))
}

pub fn forward_visibility_shared(
    frame: &dyn DecodedSource,
    links: &BTreeMap<u32, u32>,
    owner: u32,
    inputs: &Recorded<Option<crate::rows::ReverseZeroVisibilityInputs>>,
    military: bool,
) -> Result<bool, &'static str> {
    forward_visibility_shared_of(links, inputs, military, || {
        let Some(faction) = map_intel_faction(frame, links, owner)? else {
            return Ok(false);
        };
        Ok(reserved_faction(reserved(frame)?)? == faction)
    })
}

pub fn forward_visibility_shared_of(
    links: &BTreeMap<u32, u32>,
    inputs: &Recorded<Option<crate::rows::ReverseZeroVisibilityInputs>>,
    military: bool,
    same_faction: impl FnOnce() -> Result<bool, &'static str>,
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
        event_or(same_faction(), || {
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

#[derive(Clone, Default)]
pub struct Families {
    factions: BTreeMap<u32, u64>,
    subjects: BTreeSet<(u32, u32)>,
    attaches: BTreeSet<(u32, u32)>,
}

impl Families {
    pub fn from_parts(factions: BTreeMap<u32, u64>, subjects: impl IntoIterator<Item = (u32, u32)>) -> Self {
        let mut out = Self { factions, ..Self::default() };
        for (a, b) in subjects {
            out.subjects.extend([(a, b), (b, a)]);
        }
        out
    }

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

    pub fn read_factions_tables<F: Tables + ?Sized>(
        &mut self,
        frame: &F,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        each::<tables::FactionMembers, F>(frame, "FactionMembers", |row| {
            self.add_faction(row.country, row.faction, links)
        })
    }

    fn add_faction(
        &mut self,
        country: u32,
        faction: u64,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        let country = if country == 0 {
            0
        } else {
            link(links, country, "FactionMembers country")?
        };
        self.factions.insert(country, faction);
        Ok(())
    }

    fn add_subject(
        &mut self,
        overlord: u32,
        subject: u32,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        let (a, b) = (
            link(links, overlord, "Subjects country")?,
            link(links, subject, "Subjects country")?,
        );
        self.subjects.extend([(a, b), (b, a)]);
        Ok(())
    }

    pub fn read_subjects_tables<F: Tables + ?Sized>(
        &mut self,
        frame: &F,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        each::<tables::Subjects, F>(frame, "Subjects", |row| {
            self.add_subject(row.overlord, row.subject, links)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn factions_for_tests(pairs: &[(u32, u32)]) -> Self {
        let mut out = Self::default();
        for (faction, &(a, b)) in (1..).zip(pairs) {
            out.factions.extend([(a, faction), (b, faction)]);
        }
        out
    }

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

    pub fn faction(&self, slot: u32) -> Option<u64> {
        self.factions.get(&slot).copied()
    }

    pub fn members(&self) -> impl Iterator<Item = (u32, u64)> + '_ {
        self.factions.iter().map(|(&slot, &faction)| (slot, faction))
    }

    pub fn partners(&self, slot: u32) -> impl Iterator<Item = u32> + '_ {
        self.subjects.range((slot, 0)..=(slot, u32::MAX)).map(|&(_, other)| other)
    }

    pub fn family(&self, a: u32, b: u32) -> bool {
        a == b || self.subjects.contains(&(a, b)) || self.same_faction(a, b)
    }

    pub fn same_faction(&self, a: u32, b: u32) -> bool {
        self.factions
            .get(&a)
            .is_some_and(|&f| self.member_of_faction(b, f))
    }

    pub fn member_of_faction(&self, country: u32, faction: u64) -> bool {
        self.factions.get(&country) == Some(&faction)
    }

    pub fn counter_shared(&self, a: u32, b: u32) -> bool {
        self.subjects.contains(&(a, b))
            || self
                .factions
                .get(&a)
                .is_some_and(|faction| *faction != 0 && self.factions.get(&b) == Some(faction))
    }

    pub fn allied_for_fog(&self, source: u32, viewer: u32) -> bool {
        self.family(source, viewer) || self.attaches.contains(&(source, viewer))
    }
}

#[derive(Clone, Default)]
pub struct Relations {
    families: Families,
    wars: BTreeSet<(u32, u32)>,
}

impl Relations {
    #[cfg(any(test, feature = "test-support"))]
    pub fn factions_for_tests(pairs: &[(u32, u32)]) -> Self {
        Self {
            families: Families::factions_for_tests(pairs),
            wars: BTreeSet::new(),
        }
    }

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

    pub fn with_attaches(
        mut self,
        frame: &dyn DecodedSource,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        self.families = self.families.with_attaches(frame, links)?;
        Ok(self)
    }

    pub fn read_tables<F: Tables + ?Sized>(
        frame: &F,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        let mut out = Self::default();
        out.families.read_factions_tables(frame, links)?;
        each::<tables::WarRelations, F>(frame, "WarRelations", |row| {
            out.add_war(row.first_country, row.second_country, links)
        })?;
        out.families.read_subjects_tables(frame, links)?;
        Ok(out)
    }

    pub fn from_pairs(
        factions: impl IntoIterator<Item = (u32, u64)>,
        wars: impl IntoIterator<Item = (u32, u32)>,
        subjects: impl IntoIterator<Item = (u32, u32)>,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        let mut out = Self::default();
        for (country, faction) in factions {
            out.families.add_faction(country, faction, links)?;
        }
        for (first, second) in wars {
            out.add_war(first, second, links)?;
        }
        for (overlord, subject) in subjects {
            out.families.add_subject(overlord, subject, links)?;
        }
        Ok(out)
    }

    fn add_war(
        &mut self,
        first: u32,
        second: u32,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        let (a, b) = (
            link(links, first, "WarRelations country")?,
            link(links, second, "WarRelations country")?,
        );
        self.wars.extend([(a, b), (b, a)]);
        Ok(())
    }

    pub fn at_war(&self, a: u32, b: u32) -> bool {
        a != b && self.wars.contains(&(a, b))
    }

    pub fn enemies(&self, a: u32) -> impl Iterator<Item = u32> + '_ {
        self.wars.range((a, 0)..=(a, u32::MAX)).map(|&(_, b)| b)
    }

    pub fn same_faction(&self, a: u32, b: u32) -> bool {
        self.families.same_faction(a, b)
    }

    pub fn shared(&self, a: u32, b: u32) -> bool {
        a == b || (!self.wars.contains(&(a, b)) && self.families.family(a, b))
    }

    pub fn allied_for_fog(&self, source: u32, viewer: u32) -> bool {
        self.families.allied_for_fog(source, viewer)
    }
}

#[derive(Clone, Debug, Default)]
pub struct AccessLinks {
    military: BTreeSet<(u32, u32)>,
    docking: BTreeSet<(u32, u32)>,
}

impl AccessLinks {
    pub fn read_tables<F: Tables + ?Sized>(
        frame: &F,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        let mut out = Self::default();
        each::<tables::RelationLinks, F>(frame, "RelationLinks", |row| {
            out.add(row.country, row.target_index, row.links, links)
        })?;
        Ok(out)
    }

    pub fn from_rows(
        rows: impl IntoIterator<Item = (u32, u32, u8)>,
        links: &BTreeMap<u32, u32>,
    ) -> Result<Self, &'static str> {
        let mut out = Self::default();
        for (country, target, held) in rows {
            out.add(country, target, held, links)?;
        }
        Ok(out)
    }

    fn add(
        &mut self,
        country: u32,
        target: u32,
        held: u8,
        links: &BTreeMap<u32, u32>,
    ) -> Result<(), &'static str> {
        let holder = *links.get(&country).ok_or("RelationLinks country")?;
        if held & RelationLinkRow::MILITARY_ACCESS != 0 {
            self.military.insert((holder, target));
        }
        if held & RelationLinkRow::DOCKING_RIGHTS != 0 {
            self.docking.insert((holder, target));
        }
        Ok(())
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
    pub fn known_port(&self, access: &AccessLinks, viewer: u32, owner: u32) -> bool {
        viewer == owner
            || (!self.wars.contains(&(viewer, owner))
                && (access.military(viewer, owner)
                    || access.docking(viewer, owner)
                    || self.families.family(viewer, owner)))
    }

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

#[derive(Clone, Copy, Debug, Default)]
pub struct ProvinceAccess {
    pub lake: bool,
    pub sea: bool,
    pub state_owner: Option<u32>,
    pub controller: Option<u32>,
}
