
use super::{Columns, Content};
use crate::{owned::{Table, Tables}, playhead_host::mirror::Cell};
use replay_columnar::manifest::Status;
use replay_data::rows::{RelationLinkRow, TableId};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use viewer_foundation::{perspective::{Choice, Resolution}, visibility::{AirDefines, NavalDefines}};

pub(super) const READ: [TableId; 9] = [
    TableId::CountryLinks,
    TableId::Countries,
    TableId::LocalPlayer,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::Subjects,
    TableId::RelationLinks,
    TableId::CountryVisibility,
    TableId::SimulationFeatures,
];

pub(super) const STATUS_ONLY: [TableId; 1] = [TableId::UnitIntel];

pub(super) const STATUSES: [TableId; 4] = [TableId::UnitIntel, TableId::CountryIntel, TableId::StateVisibility, TableId::RadarCoverage];

pub(super) const NAMED: [TableId; 2] = [TableId::CountryLinks, TableId::Countries];

pub(super) struct Source<'a> {
    pub(super) tables: &'a Tables,
    pub(super) statuses: &'a BTreeMap<TableId, Option<Status>>,
    pub(super) refusals: &'a BTreeMap<TableId, String>,
}

impl Source<'_> {
    fn get(&self, table: TableId) -> Option<&Table> {
        self.tables.get(table)
    }

    fn complete(&self, table: TableId) -> Option<&Table> {
        self.get(table).filter(|rows| rows.complete())
    }

    fn status(&self, table: TableId) -> Option<Status> {
        match self.get(table) {
            Some(rows) => rows.status(),
            None => self.statuses.get(&table).copied().flatten(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Relations {
    factions: Arc<BTreeMap<u32, u64>>,
    wars: Arc<BTreeSet<(u32, u32)>>,
    subjects: Arc<BTreeSet<(u32, u32)>>,
    attaches: BTreeSet<(u32, u32)>,
}

impl Relations {
    pub(super) fn family(&self, a: u32, b: u32) -> bool {
        a == b || self.subjects.contains(&(a, b)) || self.factions.get(&a).is_some_and(|faction| self.factions.get(&b) == Some(faction))
    }

    pub(super) fn shared(&self, a: u32, b: u32) -> bool {
        a == b || (!self.wars.contains(&(a, b)) && self.family(a, b))
    }

    pub(super) fn at_war(&self, a: u32, b: u32) -> bool {
        a != b && self.wars.contains(&(a, b))
    }

    pub(super) fn allied_for_fog(&self, source: u32, viewer: u32) -> bool {
        self.family(source, viewer) || self.attaches.contains(&(source, viewer))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Global {
    pub(super) links: Arc<BTreeMap<u32, u32>>,
    pub(super) raws: Arc<BTreeMap<u32, Vec<u32>>>,
    pub(super) natives: Arc<BTreeMap<u32, Vec<u32>>>,
    tags: Arc<BTreeMap<u32, String>>,
    pub(super) ours: u32,
    pub(super) resolution: Resolution,
    pub(super) viewer: Option<u32>,
    pub(super) fog: bool,
    pub(super) undecided: Option<&'static str>,
    pub(super) relations: Result<Relations, &'static str>,
    pub(super) fog_relations: Result<Relations, &'static str>,
    pub(super) shares: BTreeMap<u32, ()>,
    pub(super) wars: BTreeMap<u32, ()>,
    pub(super) unit_modifier: i64,
    pub(super) truncated_unit_intel: bool,
    pub(super) air_defines: Option<AirDefines>,
    pub(super) naval_defines: Option<NavalDefines>,
    pub(super) air_access: Option<BTreeMap<u32, ()>>,
    admitted: Option<BTreeSet<u32>>,
    pub(super) failed: Option<String>,
}

impl Default for Global {
    fn default() -> Self {
        Self {
            links: Arc::default(),
            raws: Arc::default(),
            natives: Arc::default(),
            tags: Arc::default(),
            ours: 0,
            resolution: Resolution::default(),
            viewer: None,
            fog: false,
            undecided: None,
            relations: Err("not read"),
            fog_relations: Err("not read"),
            shares: BTreeMap::new(),
            wars: BTreeMap::new(),
            unit_modifier: 0,
            truncated_unit_intel: false,
            air_defines: None,
            naval_defines: None,
            air_access: None,
            admitted: None,
            failed: None,
        }
    }
}

fn link(links: &BTreeMap<u32, u32>, raw: u32, what: &'static str) -> Result<u32, &'static str> {
    links.get(&raw).copied().ok_or(what)
}

impl Global {
    pub(super) fn unread(reason: &str) -> Self {
        Self { failed: Some(reason.into()), ..Self::default() }
    }

    pub(super) fn read(source: &Source<'_>, columns: &Columns, choice: &Choice, content: &Content) -> Self {
        let mut global = Self::default();
        for table in NAMED {
            if source.complete(table).is_none() {
                global.failed = Some(source.refusals.get(&table).cloned().unwrap_or_else(|| format!("{table:?} is not read")));
                return global;
            }
        }
        let (mut links, mut raws, mut natives, mut tags) = (BTreeMap::new(), BTreeMap::<u32, Vec<u32>>::new(), BTreeMap::<u32, Vec<u32>>::new(), BTreeMap::new());
        let mut unnamed = Vec::new();
        if let Some(rows) = source.complete(TableId::CountryLinks) {
            for slot in rows.rows() {
                if let (Some(key), Some(canonical)) = (rows.key(slot), rows.number(columns.link_slot, slot).value()) {
                    links.insert(key as u32, canonical);
                }
            }
        }
        for (&raw, &slot) in &links {
            raws.entry(slot).or_default().push(raw);
        }
        if let Some(countries) = source.complete(TableId::Countries) {
            for slot in countries.rows() {
                if let (Some(key), Some(canonical)) = (countries.key(slot), countries.number(columns.country_slot, slot).value()) {
                    natives.entry(canonical).or_default().push(key as u32);
                    let tag = match countries.named_text(columns.country_tag, slot) {
                        Ok(Cell::Value(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
                        Ok(_) => String::new(),
                        Err(id) => {
                            unnamed.push((canonical, id));
                            String::new()
                        }
                    };
                    tags.insert(canonical, tag);
                }
            }
            natives.values_mut().for_each(|natives| natives.sort_unstable());
        }
        (global.links, global.raws, global.natives, global.tags) = (Arc::new(links), Arc::new(raws), Arc::new(natives), Arc::new(tags));
        let recorded = source.complete(TableId::LocalPlayer).and_then(|player| {
            let mut rows = player.rows();
            let (slot, None) = (rows.next()?, rows.next()) else { return None };
            let country = player.number(columns.player_country, slot).value().unwrap_or(0);
            let observer = player.number(columns.player_observer, slot).value().unwrap_or(0);
            let raw = if (country as i32) > 0 { country } else { observer };
            ((raw as i32) > 0).then_some(raw).and_then(|raw| global.links.get(&raw).copied())
        });
        if let viewer_foundation::perspective::Country::Tag(chosen) = &choice.country
            && let Some(&(slot, id)) = unnamed.first()
            && !global.tags.values().any(|tag| tag.eq_ignore_ascii_case(chosen))
        {
            global.failed = Some(format!("the country chosen, {chosen}, cannot be found: country slot {slot}'s tag is string {id}, which is not named at the head"));
            return global;
        }
        let resolved = viewer_foundation::fold_tables::resolve(recorded, &global.tags, &choice.country);
        let Some((ours, resolution)) = resolved else {
            global.failed = Some(format!("hour {} records no country to view the map as", source.get(TableId::Countries).and_then(Table::head).unwrap_or(0)));
            return global;
        };
        global.ours = ours;
        global.resolution = resolution;
        global.viewer = global.natives.get(&ours).and_then(|natives| natives.first().copied());
        let unrecorded = [TableId::UnitIntel, TableId::CountryVisibility, TableId::CountryIntel, TableId::StateVisibility, TableId::RadarCoverage]
            .iter()
            .any(|&table| source.status(table) == Some(Status::NotRecorded));
        global.fog = choice.fog && !unrecorded;
        global.truncated_unit_intel = matches!(source.status(TableId::UnitIntel), Some(Status::Truncated { .. }));
        global.read_relations(source, columns);
        global.read_modifier(source, columns);
        global.read_defines(source, columns, content);
        global
    }

    fn read_relations(&mut self, source: &Source<'_>, columns: &Columns) {
        let ours = self.ours;
        self.relations = Self::relations(source, columns, &self.links);
        (self.shares, self.wars, self.undecided) = (BTreeMap::from([(ours, ())]), BTreeMap::new(), None);
        match &self.relations {
            Ok(relations) => {
                for &slot in self.tags.keys().filter(|&&slot| relations.shared(slot, ours)) {
                    self.shares.insert(slot, ());
                }
                for &(_, enemy) in relations.wars.range((ours, 0)..=(ours, u32::MAX)).filter(|&&(_, enemy)| enemy != ours) {
                    self.wars.insert(enemy, ());
                }
            }
            Err(reason) => self.undecided = Some(reason),
        }
        let links = &self.links;
        let link_rows = source.complete(TableId::RelationLinks);
        let bits = |rows: &Table, slot: u32| rows.number(columns.relation_links, slot).value().unwrap_or(0);
        let fog_relations = self.relations.clone().and_then(|mut relations| {
            let rows = link_rows.ok_or("RelationLinks")?;
            for slot in rows.rows() {
                let Some(key) = rows.key(slot) else { continue };
                if bits(rows, slot) & RelationLinkRow::ATTACHE != 0 {
                    let owner = *links.get(&((key >> 32) as u32)).ok_or("RelationLinks country")?;
                    relations.attaches.insert((owner, key as u32));
                }
            }
            Ok(relations)
        });
        self.fog_relations = fog_relations;
        if !matches!(source.status(TableId::UnitIntel), Some(Status::Truncated { .. } | Status::Complete)) {
            self.undecided.get_or_insert("UnitIntel");
        }
        if source.complete(TableId::CountryVisibility).is_none() {
            self.undecided.get_or_insert("CountryVisibility");
        }
        self.air_access = link_rows.map(|rows| {
            rows.rows()
                .filter(|&slot| bits(rows, slot) & RelationLinkRow::AIR_BASE_ACCESS != 0)
                .filter_map(|slot| rows.key(slot))
                .filter(|key| self.links.get(&((key >> 32) as u32)) == Some(&ours))
                .map(|key| (key as u32, ()))
                .collect()
        });
        self.admitted = self.read_admitted();
    }

    fn read_modifier(&mut self, source: &Source<'_>, columns: &Columns) {
        let Some(rows) = source.complete(TableId::CountryVisibility) else { return };
        self.unit_modifier = rows
            .rows()
            .find(|&slot| rows.key(slot).is_some_and(|key| self.links.get(&(key as u32)) == Some(&self.ours)))
            .and_then(|slot| rows.number(columns.unit_modifier, slot).value())
            .unwrap_or(0);
    }

    fn read_defines(&mut self, source: &Source<'_>, columns: &Columns, content: &Content) {
        let Ok(fog) = &content.fog else { return };
        let mask = source.complete(TableId::SimulationFeatures).and_then(|features| {
            let mut rows = features.rows();
            let (slot, None) = (rows.next()?, rows.next()) else { return None };
            features.number(columns.dlc_mask, slot).value()
        });
        self.air_defines = mask.map(|mask| AirDefines::of(&fog.defines, mask));
        self.naval_defines = mask.map(|mask| NavalDefines::of(&fog.defines, mask));
    }

    fn relations(source: &Source<'_>, columns: &Columns, links: &BTreeMap<u32, u32>) -> Result<Relations, &'static str> {
        let (mut factions, mut wars, mut subjects) = (BTreeMap::new(), BTreeSet::new(), BTreeSet::new());
        let rows = source.complete(TableId::FactionMembers).ok_or("FactionMembers")?;
        for slot in rows.rows() {
            let Some(key) = rows.key(slot) else { continue };
            let raw = key as u32;
            let country = if raw == 0 { 0 } else { link(links, raw, "FactionMembers country")? };
            factions.insert(country, rows.number(columns.faction, slot).value().unwrap_or(0));
        }
        let rows = source.complete(TableId::WarRelations).ok_or("WarRelations")?;
        for key in rows.rows().filter_map(|slot| rows.key(slot)) {
            let (a, b) = (link(links, (key >> 32) as u32, "WarRelations country")?, link(links, key as u32, "WarRelations country")?);
            wars.extend([(a, b), (b, a)]);
        }
        let rows = source.complete(TableId::Subjects).ok_or("Subjects")?;
        for key in rows.rows().filter_map(|slot| rows.key(slot)) {
            let (a, b) = (link(links, (key >> 32) as u32, "Subjects country")?, link(links, key as u32, "Subjects country")?);
            subjects.extend([(a, b), (b, a)]);
        }
        Ok(Relations { factions: Arc::new(factions), wars: Arc::new(wars), subjects: Arc::new(subjects), attaches: BTreeSet::new() })
    }

    fn read_admitted(&self) -> Option<BTreeSet<u32>> {
        let relations = self.fog_relations.as_ref().ok()?;
        Some(self.natives.keys().copied().filter(|&slot| slot == self.ours || relations.allied_for_fog(slot, self.ours)).collect())
    }

    pub(super) fn moved_terms(&self, other: &Self) -> Option<usize> {
        let parts: [&dyn Fn() -> bool; 4] = [
            &|| self.links != other.links,
            &|| self.natives != other.natives,
            &|| (self.ours, self.viewer, self.fog, self.failed.is_some()) != (other.ours, other.viewer, other.fog, other.failed.is_some()),
            &|| self.admitted != other.admitted || self.relations.as_ref().err() != other.relations.as_ref().err() || self.fog_relations.as_ref().err() != other.fog_relations.as_ref().err(),
        ];
        parts.iter().position(|moved| moved())
    }
}
