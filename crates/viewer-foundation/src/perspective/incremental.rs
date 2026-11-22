use super::{members::Members, values::Values, *};
use replay_data::rows::{Recorded, RowCodec, TableSchema};
use replay_frame::frame::ViewIdentity;
use std::collections::{BTreeMap, BTreeSet};

type Input = (usize, u128);
type Group = (usize, u8, u128);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Dep {
    Row(Input),
    Group(Group),
    Clock,
    ZeroBoundary,
    FactionValidation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Job {
    Source(Input),
    Share(u32),
    Level(u64),
    Intel(u32),
    Controller(u32),
    Invasion(u64),
    Access(u32),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Order(u8, u32, u32, u8, [u128; 4]);
#[derive(Clone, Copy)]
enum Action {
    Set,
    Max(u8),
    Add(i32, bool),
}
struct Operation {
    province: u32,
    order: Order,
    action: Action,
}
struct Work {
    stage: u16,
    order: Order,
    deps: BTreeSet<Dep>,
    ops: Vec<Operation>,
    error: Option<&'static str>,
}
impl Work {
    fn new() -> Self {
        Self {
            stage: 60,
            order: Order(0, 0, 0, 0, [0; 4]),
            deps: BTreeSet::new(),
            ops: Vec::new(),
            error: None,
        }
    }
    fn dep(&mut self, dep: Dep) {
        self.deps.insert(dep);
    }
    fn op(&mut self, op: Operation) -> io::Result<()> {
        self.ops
            .try_reserve(1)
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        self.ops.push(op);
        Ok(())
    }
}
struct RetainedWork {
    stage: u16,
    order: Order,
    deps: Vec<Dep>,
    ops: Vec<Operation>,
}
impl Work {
    fn retain(self) -> io::Result<RetainedWork> {
        let mut deps = Vec::new();
        deps.try_reserve_exact(self.deps.len())
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        deps.extend(self.deps);
        Ok(RetainedWork {
            stage: self.stage,
            order: self.order,
            deps,
            ops: self.ops,
        })
    }
}
struct Indexed {
    groups: Vec<Group>,
}
struct Index {
    rows: BTreeMap<Input, Indexed>,
    groups: BTreeMap<Group, Members<u128>>,
    links: BTreeMap<u32, u32>,
    invalid_factions: BTreeMap<u32, &'static str>,
}
impl Index {
    fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
            groups: BTreeMap::new(),
            links: BTreeMap::new(),
            invalid_factions: BTreeMap::new(),
        }
    }
    fn remove(&mut self, input: Input) {
        if input.0 == TableId::CountryLinks.index() {
            self.links.remove(&(input.1 as u32));
        }
        if let Some(row) = self.rows.remove(&input) {
            for group in row.groups {
                let members = self.groups.get_mut(&group).unwrap();
                members.remove(&input.1);
                if members.is_empty() {
                    self.groups.remove(&group);
                }
            }
        }
    }
    fn install(&mut self, frame: &DecodedFrame, input: Input) -> io::Result<()> {
        self.remove(input);
        if input.0 == TableId::FactionMembers.index() || input.0 == TableId::CountryLinks.index() {
            let raw = input.1 as u32;
            self.invalid_factions.remove(&raw);
            if let Some(faction) = row::<tables::FactionMembers>(frame, raw as u128)? {
                let reason =
                    if raw != 0 && row::<tables::CountryLinks>(frame, raw as u128)?.is_none() {
                        Some("visibility country link")
                    } else if faction.faction == 0 {
                        Some("FactionMembers has null faction identity")
                    } else {
                        None
                    };
                if let Some(reason) = reason {
                    self.invalid_factions.insert(raw, reason);
                }
            }
        }
        let Some(groups) = memberships(frame, input)? else {
            return Ok(());
        };
        for &group in &groups {
            self.groups.entry(group).or_default().insert(input.1);
        }
        if input.0 == TableId::CountryLinks.index() {
            let r = row::<tables::CountryLinks>(frame, input.1)?.unwrap();
            self.links.insert(r.id, r.canonical_slot);
        }
        self.rows.insert(input, Indexed { groups });
        Ok(())
    }
}
#[derive(Debug)]
enum Failure {
    Gap(&'static str),
    Resource(io::Error),
}
impl From<io::Error> for Failure {
    fn from(e: io::Error) -> Self {
        Self::Resource(e)
    }
}
impl From<&'static str> for Failure {
    fn from(e: &'static str) -> Self {
        Self::Gap(e)
    }
}
type Result<T> = std::result::Result<T, Failure>;
fn row<T: TableSchema>(frame: &DecodedFrame, key: u128) -> io::Result<Option<&T::Row>> {
    let rows = rows::<T>(frame)?.rows();
    Ok(rows
        .binary_search_by_key(&key, RowCodec::key)
        .ok()
        .map(|index| &rows[index]))
}
struct Query<'a> {
    frame: &'a DecodedFrame,
    index: &'a Index,
    work: &'a mut Work,
    content: &'a FogContent,
    ours: u32,
}
impl<'a> Query<'a> {
    fn row<T: TableSchema>(&mut self, key: u128) -> Result<Option<&'a T::Row>> {
        self.work.dep(Dep::Row((T::ID.index(), key)));
        Ok(row::<T>(self.frame, key)?)
    }
    fn group(&mut self, table: TableId, dim: u8, value: u128) -> Result<Vec<u128>> {
        let group = (table.index(), dim, value);
        self.work.dep(Dep::Group(group));
        Ok(self
            .index
            .groups
            .get(&group)
            .into_iter()
            .flatten()
            .copied()
            .collect())
    }
    fn canonical(&mut self, raw: u32) -> Result<u32> {
        if raw == 0 {
            return Ok(0);
        };
        self.row::<tables::CountryLinks>(raw as u128)?
            .map(|r| r.canonical_slot)
            .ok_or(Failure::Gap("visibility country link"))
    }
    fn equal(&mut self, a: u32, b: u32) -> Result<bool> {
        Ok(a == b || (a != 0 && b != 0 && self.canonical(a)? == self.canonical(b)?))
    }
    fn raws(&mut self, slot: u32) -> Result<Vec<u128>> {
        self.group(TableId::CountryLinks, 1, slot as u128)
    }
    fn countries(&mut self, slot: u32) -> Result<Vec<(u32, u32)>> {
        let keys = self.group(TableId::Countries, 1, slot as u128)?;
        Ok(keys
            .into_iter()
            .take(if slot == self.ours { 1 } else { usize::MAX })
            .map(|raw| (slot, raw as u32))
            .collect())
    }
    fn faction(&mut self, slot: u32) -> Result<Option<u64>> {
        let mut out = None;
        let mut raws = self.raws(slot)?;
        raws.retain(|&raw| raw != 0);
        if slot == 0 {
            raws.insert(0, 0);
        }
        for raw in raws {
            if let Some(r) = self.row::<tables::FactionMembers>(raw)? {
                out = Some(r.faction)
            }
        }
        Ok(out)
    }
    fn pair(&mut self, table: TableId, a: u32, b: u32) -> Result<bool> {
        let (a, b) = (self.raws(a)?, self.raws(b)?);
        for raw_a in a {
            for &raw_b in &b {
                for key in [(raw_a << 32) | raw_b, (raw_b << 32) | raw_a] {
                    let present = match table {
                        TableId::Subjects => self.row::<tables::Subjects>(key)?.is_some(),
                        TableId::WarRelations => self.row::<tables::WarRelations>(key)?.is_some(),
                        _ => unreachable!(),
                    };
                    if present {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
    fn family(&mut self, a: u32, b: u32) -> Result<bool> {
        if a == b {
            return Ok(true);
        };
        if self.pair(TableId::Subjects, a, b)? {
            return Ok(true);
        };
        let faction = self.faction(a)?;
        Ok(faction.is_some() && faction == self.faction(b)?)
    }
    fn sharing(&mut self, a: u32, b: u32, attache: bool) -> Result<bool> {
        if !attache && a != b && self.pair(TableId::WarRelations, a, b)? {
            return Ok(false);
        }
        if self.family(a, b)? {
            return Ok(true);
        }
        if attache {
            for raw in self.raws(a)? {
                if let Some(r) = self.row::<tables::RelationLinks>((raw << 32) | b as u128)? {
                    if r.links & RelationLinkRow::ATTACHE != 0 {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
    fn cutoff(&mut self, a: u32, b: u32) -> Result<i32> {
        let defines = &self.content.defines;
        if defines.map_cutoffs.len() < 2 {
            return Ok(0);
        }
        let (owner, target) = (self.canonical(a)?, self.canonical(b)?);
        let shared = if owner == 0 {
            b == 0 || self.reverse_zero(target)?
        } else if target == 0 {
            self.forward_zero(owner)?
        } else {
            self.sharing(owner, target, false)?
        };
        let fraction = if shared {
            100_000
        } else {
            let army = if owner == 0 {
                self.zero_army(b, target)?
            } else {
                let mut army = 0;
                for raw in self.raws(owner)? {
                    if let Some(r) =
                        self.row::<tables::CountryIntel>((raw << 32) | target as u128)?
                    {
                        army = r.army;
                        break;
                    }
                }
                if !matches!(
                    *self.frame.tables[TableId::CountryIntel.index()],
                    TableView::Complete(_)
                ) {
                    return Err("CountryIntel".into());
                }
                army
            };
            replay_derive::visibility::intel_fraction(army, defines.intel_maximums[1])?
        };
        let mut result = 0;
        for pair in defines.map_cutoffs.chunks_exact(2) {
            result = super::super::visibility::rounded(pair[1]);
            if pair[0] > fraction {
                break;
            }
        }
        Ok(result)
    }
    fn order(&self, country: (u32, u32), phase: u8, key: [u128; 4]) -> Order {
        Order(
            u8::from(country.0 != self.ours),
            country.0,
            country.1,
            phase,
            key,
        )
    }
    fn op(&mut self, p: u32, order: Order, action: Action) -> Result<()> {
        if self.row::<tables::Provinces>(p as u128)?.is_none() {
            return Err("visibility revealed province".into());
        };
        self.work.op(Operation {
            province: p,
            order,
            action,
        })?;
        Ok(())
    }
    fn neighbors(
        &mut self,
        p: u32,
        base: i32,
        country: (u32, u32),
        phase: u8,
        key: [u128; 4],
    ) -> Result<()> {
        self.work.order = self.order(country, phase, key);
        let source = self
            .row::<tables::Provinces>(p as u128)?
            .ok_or("visibility source province")?
            .controller;
        let neighbors = self
            .content
            .topology
            .neighbors
            .get(&p)
            .ok_or("visibility province topology")?;
        for &next in neighbors {
            if !self.content.topology.land.contains(&next) {
                continue;
            };
            let other = self
                .row::<tables::Provinces>(next as u128)?
                .ok_or("visibility neighbour controller")?
                .controller;
            if self.equal(other, source)? {
                continue;
            };
            let value = base.wrapping_add(self.cutoff(source, other)?).clamp(0, 255) as u8;
            self.op(next, self.order(country, phase, key), Action::Max(value))?;
        }
        Ok(())
    }
    fn reveal(
        &mut self,
        p: u32,
        base: i32,
        country: (u32, u32),
        phase: u8,
        key: [u128; 4],
    ) -> Result<()> {
        self.work.order = self.order(country, phase, key);
        self.op(p, self.order(country, phase, key), Action::Set)?;
        self.neighbors(p, base, country, phase, key)
    }
}
fn tag_hash(tag: &[u8]) -> u128 {
    tag.iter().fold(0xcbf29ce484222325u128, |hash, b| {
        (hash ^ b.to_ascii_lowercase() as u128).wrapping_mul(0x100000001b3)
    })
}
fn memberships(frame: &DecodedFrame, (table, key): Input) -> io::Result<Option<Vec<Group>>> {
    let table = TableId::ALL[table];
    Ok(match table {
        TableId::CountryLinks => row::<tables::CountryLinks>(frame, key)?.map(|r| {
            let _ = r;
            vec![
                (table.index(), 0, 0),
                (table.index(), 1, r.canonical_slot as u128),
            ]
        }),
        TableId::Countries => row::<tables::Countries>(frame, key)?.map(|r| {
            let _ = r;
            vec![
                (table.index(), 0, 0),
                (table.index(), 1, r.canonical_slot as u128),
                (
                    table.index(),
                    2,
                    tag_hash(String::from_utf8_lossy(&r.tag).as_bytes()),
                ),
            ]
        }),
        TableId::LocalPlayer => row::<tables::LocalPlayer>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0)]
        }),
        TableId::FactionMembers => row::<tables::FactionMembers>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0), (table.index(), 1, r.faction as u128)]
        }),
        TableId::WarRelations => row::<tables::WarRelations>(frame, key)?.map(|r| {
            let _ = r;
            vec![
                (table.index(), 0, 0),
                (table.index(), 1, r.first_country as u128),
                (table.index(), 1, r.second_country as u128),
            ]
        }),
        TableId::RelationLinks => row::<tables::RelationLinks>(frame, key)?.map(|r| {
            let _ = r;
            vec![
                (table.index(), 0, 0),
                (table.index(), 1, r.country as u128),
                (table.index(), 2, r.target_index as u128),
            ]
        }),
        TableId::Subjects => row::<tables::Subjects>(frame, key)?.map(|r| {
            let _ = r;
            vec![
                (table.index(), 0, 0),
                (table.index(), 1, r.overlord as u128),
                (table.index(), 1, r.subject as u128),
            ]
        }),
        TableId::UnitIntel => row::<tables::UnitIntel>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0), (table.index(), 1, r.unit as u128)]
        }),
        TableId::CountryVisibility => row::<tables::CountryVisibility>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0), (table.index(), 1, r.country as u128)]
        }),
        TableId::CountryIntel => row::<tables::CountryIntel>(frame, key)?.map(|r| {
            let _ = r;
            {
                let mut groups = vec![
                    (table.index(), 0, 0),
                    (table.index(), 1, r.target_index as u128),
                ];
                if r.owner_country == 0 {
                    groups.push((table.index(), 2, 0));
                }
                groups
            }
        }),
        TableId::CountryCounterInputs => {
            row::<tables::CountryCounterInputs>(frame, key)?.map(|r| {
                let _ = r;
                vec![(table.index(), 0, 0), (table.index(), 1, r.country as u128)]
            })
        }
        TableId::ReservedCountryNaming => {
            row::<tables::ReservedCountryNaming>(frame, key)?.map(|r| {
                let _ = r;
                vec![(table.index(), 0, 0)]
            })
        }
        TableId::Provinces => row::<tables::Provinces>(frame, key)?.map(|r| {
            let mut groups = vec![
                (table.index(), 0, 0),
                (table.index(), 1, r.controller as u128),
            ];
            if r.state != 0 {
                groups.push((table.index(), 2, r.state as u128));
            }
            groups
        }),
        TableId::Divisions => row::<tables::Divisions>(frame, key)?.map(|r| {
            let _ = r;
            vec![
                (table.index(), 0, 0),
                (table.index(), 1, r.roster_owner as u128),
            ]
        }),
        TableId::DivisionMovement => row::<tables::DivisionMovement>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0)]
        }),
        TableId::RadarCoverage => row::<tables::RadarCoverage>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0), (table.index(), 1, r.country as u128)]
        }),
        TableId::SimulationFeatures => row::<tables::SimulationFeatures>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0)]
        }),
        TableId::StateVisibility => row::<tables::StateVisibility>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0)]
        }),
        TableId::States => row::<tables::States>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0), (table.index(), 1, r.owner as u128)]
        }),
        TableId::Fleets => row::<tables::Fleets>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0), (table.index(), 1, r.owner as u128)]
        }),
        TableId::TaskForces => row::<tables::TaskForces>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0), (table.index(), 1, r.fleet as u128)]
        }),
        TableId::StationedUnits => row::<tables::StationedUnits>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0), (table.index(), 1, r.theatre as u128)]
        }),
        TableId::LandTheatres => row::<tables::LandTheatres>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0), (table.index(), 1, r.owner as u128)]
        }),
        TableId::TheatreAreas => row::<tables::TheatreAreas>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0), (table.index(), 1, r.theatre as u128)]
        }),
        TableId::VolunteerHosts => row::<tables::VolunteerHosts>(frame, key)?.map(|r| {
            let _ = r;
            vec![(table.index(), 0, 0), (table.index(), 1, r.country as u128)]
        }),
        _ => None,
    })
}
fn all_keys(
    frame: &DecodedFrame,
    table: TableId,
    mut visit: impl FnMut(u128) -> io::Result<()>,
) -> io::Result<()> {
    match table {
        TableId::CountryLinks => {
            for r in rows::<tables::CountryLinks>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::Countries => {
            for r in rows::<tables::Countries>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::LocalPlayer => {
            for r in rows::<tables::LocalPlayer>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::FactionMembers => {
            for r in rows::<tables::FactionMembers>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::WarRelations => {
            for r in rows::<tables::WarRelations>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::RelationLinks => {
            for r in rows::<tables::RelationLinks>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::Subjects => {
            for r in rows::<tables::Subjects>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::UnitIntel => {
            for r in rows::<tables::UnitIntel>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::CountryVisibility => {
            for r in rows::<tables::CountryVisibility>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::CountryIntel => {
            for r in rows::<tables::CountryIntel>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::CountryCounterInputs => {
            for r in rows::<tables::CountryCounterInputs>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::ReservedCountryNaming => {
            for r in rows::<tables::ReservedCountryNaming>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::Provinces => {
            for r in rows::<tables::Provinces>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::Divisions => {
            for r in rows::<tables::Divisions>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::DivisionMovement => {
            for r in rows::<tables::DivisionMovement>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::RadarCoverage => {
            for r in rows::<tables::RadarCoverage>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::SimulationFeatures => {
            for r in rows::<tables::SimulationFeatures>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::StateVisibility => {
            for r in rows::<tables::StateVisibility>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::States => {
            for r in rows::<tables::States>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::Fleets => {
            for r in rows::<tables::Fleets>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::TaskForces => {
            for r in rows::<tables::TaskForces>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::StationedUnits => {
            for r in rows::<tables::StationedUnits>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::LandTheatres => {
            for r in rows::<tables::LandTheatres>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::TheatreAreas => {
            for r in rows::<tables::TheatreAreas>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        TableId::VolunteerHosts => {
            for r in rows::<tables::VolunteerHosts>(frame)?.rows() {
                visit(r.key())?;
            }
        }
        _ => {}
    }
    Ok(())
}

impl Query<'_> {
    fn owned_countries(&mut self, raw: u32) -> Result<Vec<(u32, u32)>> {
        let slot = self.canonical(raw)?;
        if slot == 0 {
            Ok(Vec::new())
        } else {
            self.countries(slot)
        }
    }
    fn admitted(&mut self, country: u32) -> Result<bool> {
        Ok(country == self.ours || self.sharing(country, self.ours, true)?)
    }
    fn validate(&mut self, (table, key): Input) -> Result<()> {
        let table = TableId::ALL[table];
        match table {
            TableId::FactionMembers => {
                self.work.stage = 0;
                if let Some(r) = self.row::<tables::FactionMembers>(key)? {
                    if r.country != 0 {
                        self.row::<tables::CountryLinks>(r.country as u128)?
                            .ok_or("FactionMembers country")?;
                    }
                }
            }
            TableId::WarRelations => {
                self.work.stage = 1;
                if let Some(r) = self.row::<tables::WarRelations>(key)? {
                    for raw in [r.first_country, r.second_country] {
                        self.row::<tables::CountryLinks>(raw as u128)?
                            .ok_or("WarRelations country")?;
                    }
                }
            }
            TableId::Subjects => {
                self.work.stage = 2;
                if let Some(r) = self.row::<tables::Subjects>(key)? {
                    for raw in [r.overlord, r.subject] {
                        self.row::<tables::CountryLinks>(raw as u128)?
                            .ok_or("Subjects country")?;
                    }
                }
            }
            TableId::RelationLinks => {
                self.work.stage = 3;
                if let Some(r) = self.row::<tables::RelationLinks>(key)? {
                    if r.links & RelationLinkRow::ATTACHE != 0 {
                        self.row::<tables::CountryLinks>(r.country as u128)?
                            .ok_or("RelationLinks country")?;
                    }
                }
            }
            TableId::Provinces => {
                if let Some(r) = self.row::<tables::Provinces>(key)? {
                    self.work.stage = 10;
                    if !self.content.topology.neighbors.contains_key(&r.id) {
                        return Err("visibility province topology".into());
                    }
                    self.work.stage = 11;
                    self.canonical(r.controller)?;
                }
            }
            TableId::States => {
                if let Some(r) = self.row::<tables::States>(key)? {
                    self.work.stage = 12;
                    self.canonical(r.owner)?;
                    self.canonical(r.controller)?;
                }
            }
            TableId::Divisions => {
                if let Some(r) = self.row::<tables::Divisions>(key)? {
                    self.work.stage = 13;
                    self.canonical(r.roster_owner)?;
                }
            }
            TableId::Fleets => {
                if let Some(r) = self.row::<tables::Fleets>(key)? {
                    self.work.stage = 14;
                    self.canonical(r.owner)?;
                }
            }
            TableId::LandTheatres => {
                if let Some(r) = self.row::<tables::LandTheatres>(key)? {
                    if r.volunteers_theatre > 0 {
                        self.work.stage = 15;
                        self.canonical(r.owner)?;
                        self.work.stage = 16;
                        self.canonical(r.volunteers_theatre as u32)?;
                    }
                }
            }
            TableId::VolunteerHosts => {
                if let Some(r) = self.row::<tables::VolunteerHosts>(key)? {
                    self.work.stage = 17;
                    self.canonical(r.country)?;
                }
            }
            TableId::RadarCoverage => {
                if let Some(r) = self.row::<tables::RadarCoverage>(key)? {
                    self.work.stage = 18;
                    self.canonical(r.country)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn source(&mut self, (table, key): Input) -> Result<()> {
        match TableId::ALL[table] {
            TableId::Provinces => {
                if let Some(r) = self.row::<tables::Provinces>(key)? {
                    if !self.content.topology.neighbors.contains_key(&r.id) {
                        return Err("visibility province topology".into());
                    }
                    for c in self.owned_countries(r.controller)? {
                        if self.admitted(c.0)? {
                            self.reveal(r.id, 150, c, 0, [key, 0, 0, 0])?
                        }
                    }
                }
            }
            TableId::States => {
                if let Some(r) = self.row::<tables::States>(key)? {
                    let owner = self.canonical(r.owner)?;
                    let controller = self.canonical(r.controller)?;
                    if owner != controller && owner != 0 {
                        for c in self.countries(owner)? {
                            if !self.admitted(c.0)? {
                                continue;
                            }
                            let Some(network) =
                                self.row::<tables::StateVisibility>(r.id as u128)?
                            else {
                                continue;
                            };
                            self.work.order = self.order(c, 2, [key, 0, 0, 0]);
                            let term = super::super::visibility::network_term(
                                self.cutoff(c.1, r.controller)?,
                                network.network_modifier,
                            );
                            for p in self.group(TableId::Provinces, 2, r.id as u128)? {
                                self.op(
                                    p as u32,
                                    self.order(c, 2, [key, p, 0, 0]),
                                    Action::Add(
                                        term,
                                        self.content.topology.land.contains(&(p as u32)),
                                    ),
                                )?;
                            }
                        }
                    }
                }
            }
            TableId::Divisions => {
                if let Some(r) = self.row::<tables::Divisions>(key)? {
                    for c in self.owned_countries(r.roster_owner)? {
                        if r.reveal_skip == 0 && r.province != 0 && self.admitted(c.0)? {
                            self.reveal(r.province, 250, c, 3, [key, 0, 0, 0])?
                        }
                    }
                }
            }
            TableId::TaskForces => {
                if let Some(r) = self.row::<tables::TaskForces>(key)? {
                    if r.province != 0 {
                        if let Some(fleet) = self.row::<tables::Fleets>(r.fleet as u128)? {
                            for c in self.owned_countries(fleet.owner)? {
                                if self.admitted(c.0)? {
                                    self.neighbors(
                                        r.province,
                                        250,
                                        c,
                                        4,
                                        [r.fleet as u128, key, 0, 0],
                                    )?
                                }
                            }
                        }
                    }
                }
            }
            TableId::Fleets => {
                if let Some(r) = self.row::<tables::Fleets>(key)? {
                    self.canonical(r.owner)?;
                }
            }
            TableId::LandTheatres => {
                if let Some(r) = self.row::<tables::LandTheatres>(key)? {
                    if r.volunteers_theatre > 0 {
                        self.canonical(r.owner)?;
                        self.canonical(r.volunteers_theatre as u32)?;
                    }
                }
            }
            TableId::VolunteerHosts => {
                if let Some(r) = self.row::<tables::VolunteerHosts>(key)? {
                    if self.canonical(r.country)? == self.ours && self.ours != 0 {
                        let host = self.canonical(r.host)?;
                        if host == 0 {
                            return Ok(());
                        }
                        let mut provinces = Vec::new();
                        for raw in self.raws(host)?.into_iter().filter(|&raw| raw != 0) {
                            let group = self.group(TableId::Provinces, 1, raw)?;
                            provinces.extend(group);
                        }
                        provinces.sort_unstable();
                        for p in provinces {
                            let c = self
                                .countries(self.ours)?
                                .into_iter()
                                .next()
                                .ok_or("visibility viewing country")?;
                            self.reveal(p as u32, 150, c, 1, [1, key, p, 0])?;
                        }
                    }
                }
            }
            TableId::TheatreAreas => {
                if let Some(r) = self.row::<tables::TheatreAreas>(key)? {
                    if let Some(theatre) = self.row::<tables::LandTheatres>(r.theatre as u128)? {
                        if theatre.volunteers_theatre > 0
                            && self.canonical(theatre.owner)? == self.ours
                            && self.ours != 0
                        {
                            let mut stopped = false;
                            for area in self.group(TableId::TheatreAreas, 1, r.theatre as u128)? {
                                let area = self.row::<tables::TheatreAreas>(area)?.unwrap();
                                if area.ordinal > r.ordinal {
                                    break;
                                }
                                if area.provinces.is_empty() {
                                    stopped = true;
                                    break;
                                }
                            }
                            if !stopped {
                                let c = self
                                    .countries(self.ours)?
                                    .into_iter()
                                    .next()
                                    .ok_or("visibility viewing country")?;
                                for (i, &p) in r.provinces.iter().enumerate() {
                                    self.reveal(
                                        p,
                                        150,
                                        c,
                                        1,
                                        [0, r.theatre as u128, r.ordinal as u128, i as u128],
                                    )?;
                                }
                            }
                        }
                    }
                }
            }
            TableId::StationedUnits => {
                if let Some(r) = self.row::<tables::StationedUnits>(key)? {
                    if let Some(theatre) = self.row::<tables::LandTheatres>(r.theatre as u128)? {
                        let country = self.canonical(theatre.owner)?;
                        if country != 0 && !self.admitted(country)? {
                            if self.countries(country)?.is_empty() {
                                return Ok(());
                            }
                            let mut first = None;
                            for raw in self.raws(country)? {
                                for t in self.group(TableId::LandTheatres, 1, raw)? {
                                    let t = self.row::<tables::LandTheatres>(t)?.unwrap();
                                    if t.volunteers_theatre > 0
                                        && self.canonical(t.volunteers_theatre as u32)? == self.ours
                                        && first.is_none_or(|(ordinal, id)| {
                                            (t.ordinal, t.id) < (ordinal, id)
                                        })
                                    {
                                        first = Some((t.ordinal, t.id));
                                    }
                                }
                            }
                            if first.map(|(_, id)| id) == Some(theatre.id) {
                                let c = self
                                    .countries(country)?
                                    .into_iter()
                                    .next()
                                    .ok_or("visibility viewing country")?;
                                self.work.order = self.order(c, 0, [key, 0, 0, 0]);
                                let (owner, province) = if let Some(d) =
                                    self.row::<tables::Divisions>(r.unit as u128)?
                                {
                                    (d.logical_owner, d.province)
                                } else if let Some(t) =
                                    self.row::<tables::TaskForces>(r.unit as u128)?
                                {
                                    (t.logical_owner, t.province)
                                } else {
                                    return Err("StationedUnits unit".into());
                                };
                                if self.canonical(owner)? == self.ours && province != 0 {
                                    for c in self.countries(country)? {
                                        self.reveal(province, 250, c, 0, [key, 0, 0, 0])?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            TableId::RadarCoverage => {
                if let Some(r) = self.row::<tables::RadarCoverage>(key)? {
                    for c in self.owned_countries(r.country)? {
                        if self.admitted(c.0)? {
                            for (i, &p) in r.provinces.iter().enumerate() {
                                self.work.order =
                                    self.order(c, 5, [r.ordinal as u128, key, i as u128, 0]);
                                let controller = self
                                    .row::<tables::Provinces>(p as u128)?
                                    .ok_or("RadarCoverage province")?
                                    .controller;
                                if !self.equal(controller, c.1)? {
                                    let term = self
                                        .content
                                        .defines
                                        .radar_effect
                                        .wrapping_add(self.cutoff(c.1, controller)?);
                                    self.op(
                                        p,
                                        self.order(c, 5, [r.ordinal as u128, key, i as u128, 0]),
                                        Action::Add(term, self.content.topology.land.contains(&p)),
                                    )?;
                                }
                            }
                        }
                    }
                }
            }
            TableId::FactionMembers => {
                if let Some(r) = self.row::<tables::FactionMembers>(key)? {
                    if r.country != 0 {
                        self.row::<tables::CountryLinks>(r.country as u128)?
                            .ok_or("FactionMembers country")?;
                    }
                }
            }
            TableId::WarRelations => {
                if let Some(r) = self.row::<tables::WarRelations>(key)? {
                    for raw in [r.first_country, r.second_country] {
                        self.row::<tables::CountryLinks>(raw as u128)?
                            .ok_or("WarRelations country")?;
                    }
                }
            }
            TableId::Subjects => {
                if let Some(r) = self.row::<tables::Subjects>(key)? {
                    for raw in [r.overlord, r.subject] {
                        self.row::<tables::CountryLinks>(raw as u128)?
                            .ok_or("Subjects country")?;
                    }
                }
            }
            TableId::RelationLinks => {
                if let Some(r) = self.row::<tables::RelationLinks>(key)? {
                    if r.links & RelationLinkRow::ATTACHE != 0 {
                        self.row::<tables::CountryLinks>(r.country as u128)?
                            .ok_or("RelationLinks country")?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn meta(&mut self, job: Job, vision: &mut Vision) -> Result<()> {
        match job {
            Job::Share(slot) => {
                let exists = !self.countries(slot)?.is_empty();
                let shared =
                    slot == self.ours || (exists && self.sharing(slot, self.ours, false)?);
                set(&mut vision.shares, slot, shared.then_some(()));
                let war = slot != self.ours && self.pair(TableId::WarRelations, self.ours, slot)?;
                set(&mut vision.wars, slot, war.then_some(()));
            }
            Job::Level(unit) => {
                let mut level = None;
                for key in self.group(TableId::UnitIntel, 1, unit as u128)? {
                    let r = self.row::<tables::UnitIntel>(key)?.unwrap();
                    if r.viewer == self.ours {
                        level = Some(r.level);
                        break;
                    }
                }
                set(&mut vision.levels, unit, level);
            }
            Job::Intel(target) => {
                let mut lanes = None;
                for raw in self.raws(self.ours)? {
                    if let Some(r) =
                        self.row::<tables::CountryIntel>((raw << 32) | target as u128)?
                    {
                        lanes = Some((r.air, r.navy));
                        break;
                    }
                }
                if let Some(air) = &mut vision.air_intel {
                    set(air, target, lanes.map(|v| v.0));
                }
                if let Some(navy) = &mut vision.navy_intel {
                    set(navy, target, lanes.map(|v| v.1));
                }
            }
            Job::Controller(province) => {
                let controller =
                    match self
                        .row::<tables::Provinces>(province as u128)?
                        .filter(|_| {
                            matches!(
                                *self.frame.tables[TableId::Provinces.index()],
                                TableView::Complete(_)
                            )
                        }) {
                        Some(p) => self
                            .row::<tables::CountryLinks>(p.controller as u128)?
                            .map(|r| r.canonical_slot),
                        None => None,
                    };
                set(&mut vision.province_controllers, province, controller);
            }
            Job::Invasion(unit) => {
                let mut target = None;
                if let Some(d) = self.row::<tables::Divisions>(unit as u128)? {
                    if self.content.topology.sea.contains(&d.province) {
                        let movement = self
                            .row::<tables::DivisionMovement>(unit as u128)?
                            .ok_or("DivisionMovement division")?;
                        if let Some(&next) = movement.path.first() {
                            if self.content.topology.land.contains(&next) {
                                let p = self
                                    .row::<tables::Provinces>(next as u128)?
                                    .ok_or("invasion target controller")?;
                                let controller = self
                                    .row::<tables::CountryLinks>(p.controller as u128)?
                                    .ok_or("invasion target controller")?
                                    .canonical_slot;
                                let owner = self
                                    .row::<tables::CountryLinks>(d.logical_owner as u128)?
                                    .ok_or("invasion division owner")?
                                    .canonical_slot;
                                if controller != owner
                                    && self.pair(TableId::WarRelations, controller, owner)?
                                {
                                    target = Some(next)
                                }
                            }
                        }
                    }
                }
                set(&mut vision.sight_provinces, unit, target);
            }
            Job::Access(target) => {
                let mut yes = false;
                for raw in self.raws(self.ours)? {
                    if let Some(r) =
                        self.row::<tables::RelationLinks>((raw << 32) | target as u128)?
                    {
                        yes |= r.links & RelationLinkRow::AIR_BASE_ACCESS != 0;
                    }
                }
                if let Some(access) = &mut vision.air_access {
                    set(access, target, yes.then_some(()));
                }
            }
            Job::Source(_) => unreachable!(),
        }
        Ok(())
    }
}
fn set<K: values::Key, V: Clone + Eq>(map: &mut Values<K, V>, key: K, value: Option<V>) {
    if let Some(value) = value {
        map.insert(key, value)
    } else {
        map.remove(&key)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorkStats {
    pub jobs: usize,
    pub provinces: usize,
    pub cold: bool,
}
enum EmptyView {
    Complete(TableId),
    Truncated(TableId, u32),
    Unavailable { reason: String },
    NotRecorded(CaptureMode),
}
struct InputView {
    identity: ViewIdentity,
    allocation: usize,
    empty: Option<EmptyView>,
}
impl InputView {
    fn new(view: &replay_frame::frame::SharedView) -> io::Result<Self> {
        let empty = match &**view {
            TableView::Complete(rows) if rows.is_empty() => Some(EmptyView::Complete(rows.table())),
            TableView::Truncated { rows, missing } if rows.is_empty() => {
                Some(EmptyView::Truncated(rows.table(), *missing))
            }
            TableView::Unavailable { reason } => {
                let mut owned = String::new();
                owned
                    .try_reserve_exact(reason.len())
                    .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
                owned.push_str(reason);
                Some(EmptyView::Unavailable { reason: owned })
            }
            TableView::NotRecorded { mode } => Some(EmptyView::NotRecorded(*mode)),
            _ => None,
        };
        Ok(Self {
            identity: view.identity(),
            allocation: view.as_ptr() as usize,
            empty,
        })
    }
    fn is(&self, view: &replay_frame::frame::SharedView) -> bool {
        self.allocation == view.as_ptr() as usize || self.empty_matches(view)
    }
    fn empty_matches(&self, view: &replay_frame::frame::SharedView) -> bool {
        match (&self.empty, &**view) {
            (Some(EmptyView::Complete(table)), TableView::Complete(rows)) => {
                rows.table() == *table && rows.is_empty()
            }
            (Some(EmptyView::Truncated(table, old)), TableView::Truncated { rows, missing }) => {
                rows.table() == *table && rows.is_empty() && old == missing
            }
            (
                Some(EmptyView::Unavailable { reason: old, .. }),
                TableView::Unavailable { reason },
            ) => old == reason,
            (Some(EmptyView::NotRecorded(old)), TableView::NotRecorded { mode }) => old == mode,
            _ => false,
        }
    }
    fn refresh(&mut self, view: &replay_frame::frame::SharedView) -> io::Result<()> {
        if !self.empty_matches(view) {
            self.empty = Self::new(view)?.empty;
        }
        self.identity = view.identity();
        self.allocation = view.as_ptr() as usize;
        Ok(())
    }
}
fn fog_enabled(frame: &DecodedFrame, choice: &Choice) -> bool {
    choice.fog
        && FOG_TABLES
            .iter()
            .all(|table| !matches!(*frame.tables[table.index()], TableView::NotRecorded { .. }))
}
fn active(table: TableId, fog: bool) -> bool {
    fog || !matches!(
        table,
        TableId::States
            | TableId::StateVisibility
            | TableId::Divisions
            | TableId::DivisionMovement
            | TableId::Fleets
            | TableId::TaskForces
            | TableId::LandTheatres
            | TableId::TheatreAreas
            | TableId::StationedUnits
            | TableId::VolunteerHosts
            | TableId::RadarCoverage
    )
}
fn source_job(table: TableId, fog: bool) -> bool {
    matches!(
        table,
        TableId::FactionMembers | TableId::WarRelations | TableId::Subjects
    ) || (fog
        && matches!(
            table,
            TableId::RelationLinks
                | TableId::Provinces
                | TableId::States
                | TableId::Divisions
                | TableId::TaskForces
                | TableId::Fleets
                | TableId::LandTheatres
                | TableId::VolunteerHosts
                | TableId::TheatreAreas
                | TableId::StationedUnits
                | TableId::RadarCoverage
        ))
}

pub struct Fold {
    pub last: WorkStats,
    branch: u64,
    choice: Choice,
    views: Vec<InputView>,
    fog: bool,
    clock: (Option<bool>, Option<bool>),
    index: Index,
    jobs: BTreeMap<Job, RetainedWork>,
    dependents: BTreeMap<Dep, Members<Job>>,
    operations: BTreeMap<u32, BTreeMap<(Order, Job, usize), Action>>,
    errors: BTreeMap<(u16, Order, Job), &'static str>,
    value: Arc<Vision>,
    computed: Arc<Vision>,
}
impl Fold {
    pub fn advance(
        state: &mut Option<Self>,
        frame: &DecodedFrame,
        choice: &Choice,
        content: std::result::Result<&FogContent, &'static str>,
    ) -> io::Result<Arc<Vision>> {
        let _ = core::<tables::CountryLinks>(frame)?;
        let _ = core::<tables::Countries>(frame)?;
        let fog = fog_enabled(frame, choice);
        let changes = replay_frame::frame::HourInput::new(frame).changes();
        if let Some(fold) = state.as_mut() {
            if fold.branch == frame.branch
                && fold.choice == *choice
                && fold.fog == fog
                && fold.clock == clock(frame)
                && VISION_TABLES.iter().zip(&fold.views).all(|(&table, old)| {
                    !active(table, fog)
                        || old.is(&frame.tables[table.index()])
                        || matches!(
                            changes.since(table, Some(&old.identity), fields(table)),
                            replay_frame::frame::TableChanges::Unchanged
                        )
                })
            {
                for (old, &table) in fold.views.iter_mut().zip(VISION_TABLES) {
                    let view = &frame.tables[table.index()];
                    old.identity = view.identity();
                    old.allocation = view.as_ptr() as usize;
                }
                fold.last = WorkStats::default();
                return Ok(Arc::clone(&fold.value));
            }
        }
        let mut previous = state.take();
        let result = (|| {
            if previous.as_ref().is_some_and(|fold| {
                fold.branch != frame.branch
                    || fold.choice != *choice
                    || fold.fog != fog
                    || VISION_TABLES.iter().zip(&fold.views).any(|(&table, old)| {
                        active(table, fog)
                            && !old.is(&frame.tables[table.index()])
                            && matches!(
                                changes.since(table, Some(&old.identity), fields(table)),
                                replay_frame::frame::TableChanges::Discontinuity(_)
                            )
                    })
            }) {
                previous = None;
            }
            let mut fold = if let Some(fold) = previous.take() {
                fold
            } else {
                Self::cold(frame, choice, fog)?
            };
            let placeholder = FogContent {
                topology: visibility::Topology::default(),
                defines: visibility::Defines {
                    intel_maximums: [0; 4],
                    radar_effect: 0,
                    map_cutoffs: Vec::new(),
                    air_icon_threshold: 0,
                    air_detail_thresholds: [0; 4],
                    air_plane_fuzzy_range: 0,
                    naval_detail_thresholds: [0; 5],
                },
            };
            let (content, content_gap) = match content {
                Ok(content) => (content, None),
                Err(reason) => (&placeholder, Some(reason)),
            };
            let cold = fold.jobs.is_empty();
            let old_invalid = fold
                .index
                .invalid_factions
                .first_key_value()
                .map(|(&key, &reason)| (key, reason));
            let old_zero = fold
                .index
                .groups
                .get(&(TableId::CountryIntel.index(), 2, 0))
                .and_then(Members::last)
                .copied();
            let mut queue = BTreeSet::new();
            for (&table, old) in VISION_TABLES.iter().zip(&fold.views) {
                let view = &frame.tables[table.index()];
                if !active(table, fog) || old.is(view) {
                    continue;
                }
                let facts = match changes.since(table, Some(&old.identity), fields(table)) {
                    replay_frame::frame::TableChanges::Unchanged => continue,
                    replay_frame::frame::TableChanges::Rows(facts) => facts,
                    replay_frame::frame::TableChanges::Discontinuity(_) => {
                        unreachable!("cold fold uses current identities")
                    }
                };
                for key in facts
                    .changed()
                    .iter()
                    .filter(|change| change.inserted || change.fields & fields(table) != 0)
                    .map(|change| change.key)
                    .chain(facts.deleted().iter().copied())
                {
                    let input = (table.index(), key);
                    fold.enqueue(Dep::Row(input), &mut queue);
                    if let Some(indexed) = fold.index.rows.get(&input) {
                        for &group in &indexed.groups {
                            fold.enqueue(Dep::Group(group), &mut queue);
                        }
                    }
                    fold.index.install(frame, input)?;
                    if let Some(indexed) = fold.index.rows.get(&input) {
                        for &group in &indexed.groups {
                            fold.enqueue(Dep::Group(group), &mut queue);
                        }
                    }
                    fold.enqueue(Dep::Group((table.index(), 0, 0)), &mut queue);
                    if source_job(table, fog) {
                        queue.insert(Job::Source(input));
                    }
                    seed_jobs(frame, input, &mut queue)?;
                }
            }
            if old_invalid
                != fold
                    .index
                    .invalid_factions
                    .first_key_value()
                    .map(|(&key, &reason)| (key, reason))
            {
                fold.enqueue(Dep::FactionValidation, &mut queue);
            }
            if old_zero
                != fold
                    .index
                    .groups
                    .get(&(TableId::CountryIntel.index(), 2, 0))
                    .and_then(Members::last)
                    .copied()
            {
                fold.enqueue(Dep::ZeroBoundary, &mut queue);
            }
            if fold.clock != clock(frame) {
                fold.enqueue(Dep::Clock, &mut queue);
            }
            if fold.jobs.is_empty() {
                for &input in fold.index.rows.keys() {
                    if source_job(TableId::ALL[input.0], fog) {
                        queue.insert(Job::Source(input));
                    }
                    seed_jobs(frame, input, &mut queue)?;
                }
            }
            let (ours, resolution) = resolve(frame, &fold.index, choice)?;
            if ours != fold.value.ours && !fold.jobs.is_empty() {
                drop(fold);
                let mut replacement = None;
                let value = Self::advance(
                    &mut replacement,
                    frame,
                    choice,
                    content_gap.map_or(Ok(content), Err),
                )?;
                *state = replacement;
                return Ok(value);
            }
            let mut vision = (*fold.computed).clone();
            vision.ours = ours;
            vision.resolution = resolution;
            vision.fog = fog;
            vision.air_intel = matches!(
                *frame.tables[TableId::CountryIntel.index()],
                TableView::Complete(_)
            )
            .then(|| vision.air_intel.take().unwrap_or_default());
            vision.navy_intel = matches!(
                *frame.tables[TableId::CountryIntel.index()],
                TableView::Complete(_)
            )
            .then(|| vision.navy_intel.take().unwrap_or_default());
            vision.air_access = matches!(
                *frame.tables[TableId::RelationLinks.index()],
                TableView::Complete(_)
            )
            .then(|| vision.air_access.take().unwrap_or_default());
            queue.insert(Job::Share(ours));
            let mut dirty_provinces = BTreeSet::new();
            let mut executed = 0;
            while let Some(job) = queue.pop_first() {
                executed += 1;
                if let Job::Source((table, key)) = job {
                    if table == TableId::Provinces.index() {
                        dirty_provinces.insert(key as u32);
                    }
                }
                let mut work = Work::new();
                {
                    let mut query = Query {
                        frame,
                        index: &fold.index,
                        work: &mut work,
                        content,
                        ours,
                    };
                    let result = match job {
                        Job::Source(input) => query.validate(input).and_then(|()| {
                            query.work.stage = 40;
                            query.source(input)
                        }),
                        _ => {
                            if matches!(job, Job::Invasion(_)) {
                                query.work.stage = 50;
                            }
                            query.meta(job, &mut vision)
                        }
                    };
                    match result {
                        Ok(()) => {}
                        Err(Failure::Gap(reason)) => {
                            work.error = Some(reason);
                            work.ops.clear();
                        }
                        Err(Failure::Resource(error)) => return Err(error),
                    }
                }
                if let Some(old) = fold.jobs.remove(&job) {
                    fold.errors.remove(&(old.stage, old.order, job));
                    for dep in old.deps {
                        let jobs = fold.dependents.get_mut(&dep).unwrap();
                        jobs.remove(&job);
                        if jobs.is_empty() {
                            fold.dependents.remove(&dep);
                        }
                    }
                    for (i, op) in old.ops.iter().enumerate() {
                        dirty_provinces.insert(op.province);
                        let ops = fold.operations.get_mut(&op.province).unwrap();
                        ops.remove(&(op.order, job, i));
                        if ops.is_empty() {
                            fold.operations.remove(&op.province);
                        }
                    }
                }
                if let Some(reason) = work.error {
                    fold.errors.insert((work.stage, work.order, job), reason);
                }
                for &dep in &work.deps {
                    fold.dependents.entry(dep).or_default().insert(job);
                }
                for (i, op) in work.ops.iter().enumerate() {
                    dirty_provinces.insert(op.province);
                    fold.operations
                        .entry(op.province)
                        .or_default()
                        .insert((op.order, job, i), op.action);
                }
                let retained = match job {
                    Job::Source(input) => fold.index.rows.contains_key(&input),
                    Job::Level(unit) => fold.index.groups.contains_key(&(
                        TableId::UnitIntel.index(),
                        1,
                        unit as u128,
                    )),
                    Job::Intel(target) => fold.index.groups.contains_key(&(
                        TableId::CountryIntel.index(),
                        1,
                        target as u128,
                    )),
                    Job::Controller(province) => fold
                        .index
                        .rows
                        .contains_key(&(TableId::Provinces.index(), province as u128)),
                    Job::Invasion(unit) => fold
                        .index
                        .rows
                        .contains_key(&(TableId::Divisions.index(), unit as u128)),
                    Job::Access(target) => fold.index.groups.contains_key(&(
                        TableId::RelationLinks.index(),
                        2,
                        target as u128,
                    )),
                    Job::Share(slot) => {
                        slot == ours
                            || fold.index.groups.contains_key(&(
                                TableId::Countries.index(),
                                1,
                                slot as u128,
                            ))
                            || fold.index.groups.contains_key(&(
                                TableId::CountryLinks.index(),
                                1,
                                slot as u128,
                            ))
                    }
                };
                if retained {
                    fold.jobs.insert(job, work.retain()?);
                } else {
                    for dep in &work.deps {
                        if let Some(dependents) = fold.dependents.get_mut(dep) {
                            dependents.remove(&job);
                            if dependents.is_empty() {
                                fold.dependents.remove(dep);
                            }
                        }
                    }
                }
            }
            vision.relations_gap = relations_gap(frame, &fold.errors);
            vision.undecided = vision.relations_gap;
            if matches!(
                *frame.tables[TableId::UnitIntel.index()],
                TableView::Unavailable { .. } | TableView::NotRecorded { .. }
            ) {
                vision.undecided.get_or_insert("UnitIntel");
            }
            if frame.complete::<tables::CountryVisibility>().is_err() {
                vision.undecided.get_or_insert("CountryVisibility");
            }
            vision.truncated.clear();
            for (table, name) in [
                (TableId::UnitIntel, "UnitIntel"),
                (TableId::CountryIntel, "CountryIntel"),
            ] {
                if matches!(*frame.tables[table.index()], TableView::Truncated { .. }) {
                    vision.truncated.push(name);
                }
            }
            let mut scalar = Work::new();
            let mut query = Query {
                frame,
                index: &fold.index,
                work: &mut scalar,
                content,
                ours,
            };
            vision.unit_modifier = if frame.complete::<tables::CountryVisibility>().is_err() {
                0
            } else {
                match query.raws(ours).and_then(|raws| {
                    for raw in raws {
                        if let Some(r) = query.row::<tables::CountryVisibility>(raw)? {
                            return Ok(r.unit_modifier);
                        }
                    }
                    Ok(0)
                }) {
                    Ok(value) => value,
                    Err(Failure::Resource(error)) => return Err(error),
                    Err(_) => 0,
                }
            };
            vision.air_defines = content_gap
                .is_none()
                .then(|| AirDefines::read(frame, &content.defines).ok())
                .flatten();
            vision.naval_defines = content_gap
                .is_none()
                .then(|| NavalDefines::read(frame, &content.defines).ok())
                .flatten();
            let gap = content_gap
                .or(vision.relations_gap)
                .or_else(|| visibility_gap(frame, &fold.index, &fold.errors, ours));
            {
                if cold && fog {
                    dirty_provinces.extend(
                        fold.index
                            .groups
                            .get(&(TableId::Provinces.index(), 0, 0))
                            .into_iter()
                            .flatten()
                            .map(|&p| p as u32),
                    );
                }
                fold.last = WorkStats {
                    jobs: executed,
                    provinces: dirty_provinces.len(),
                    cold,
                };

                let mut values = (*vision.visibility).clone();
                for province in dirty_provinces {
                    if !fold
                        .index
                        .rows
                        .contains_key(&(TableId::Provinces.index(), province as u128))
                    {
                        values.remove(&province);
                        continue;
                    }
                    let mut byte = 0u8;
                    for action in fold
                        .operations
                        .get(&province)
                        .into_iter()
                        .flat_map(|ops| ops.values())
                    {
                        byte = match *action {
                            Action::Set => 255,
                            Action::Max(value) => byte.max(value),
                            Action::Add(term, land) => {
                                if land {
                                    visibility::saturate(byte, term)
                                } else {
                                    0
                                }
                            }
                        };
                    }
                    values.insert(province, byte);
                }
                if !values.same_root(&vision.visibility) {
                    vision.visibility = Arc::new(values);
                }
            }
            vision.visibility_gap = gap;
            fold.computed = Arc::new(vision.clone());
            if gap.is_some() || !vision.fog {
                vision.visibility = Arc::new(Values::default());
                vision.sight_provinces = Values::default();
            }
            if !vision.fog {
                vision.visibility_gap = None;
            }
            if vision.relations_gap.is_some() {
                vision.shares = Values::from_iter([(ours, ())]);
                vision.wars = Values::default();
            }
            fold.clock = clock(frame);
            for (identity, &table) in fold.views.iter_mut().zip(VISION_TABLES) {
                identity.refresh(&frame.tables[table.index()])?;
            }
            let value = Arc::new(vision);
            fold.value = Arc::clone(&value);
            *state = Some(fold);
            Ok(value)
        })();
        result
    }
    fn cold(frame: &DecodedFrame, choice: &Choice, fog: bool) -> io::Result<Self> {
        let mut index = Index::new();
        for &table in VISION_TABLES.iter().filter(|&&table| active(table, fog)) {
            all_keys(frame, table, |key| {
                index.install(frame, (table.index(), key))
            })?;
        }
        let mut views = Vec::new();
        views
            .try_reserve_exact(VISION_TABLES.len())
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        for &table in VISION_TABLES {
            views.push(InputView::new(&frame.tables[table.index()])?);
        }
        Ok(Self {
            last: WorkStats::default(),
            branch: frame.branch,
            choice: choice.clone(),
            views,
            fog,
            clock: clock(frame),
            index,
            jobs: BTreeMap::new(),
            dependents: BTreeMap::new(),
            operations: BTreeMap::new(),
            errors: BTreeMap::new(),
            value: Arc::new(Vision::default()),
            computed: Arc::new(Vision::default()),
        })
    }
    fn enqueue(&self, dep: Dep, jobs: &mut BTreeSet<Job>) {
        jobs.extend(self.dependents.get(&dep).into_iter().flatten().copied());
    }
}

fn clock(frame: &DecodedFrame) -> (Option<bool>, Option<bool>) {
    let Ok([row]) = frame.complete::<tables::ReservedCountryNaming>() else {
        return (None, None);
    };
    (
        match &row.country_intel_sample {
            Recorded::Value(Some(sample)) => Some(sample.sample_hour > frame.hour),
            _ => None,
        },
        match &row.reverse_visibility {
            Recorded::Value(Some(packet)) => Some(packet.sample_hour.fresh_at(frame.hour)),
            _ => None,
        },
    )
}
fn seed_jobs(
    frame: &DecodedFrame,
    (table, key): Input,
    queue: &mut BTreeSet<Job>,
) -> io::Result<()> {
    match TableId::ALL[table] {
        TableId::Countries => {
            if let Some(r) = row::<tables::Countries>(frame, key)? {
                queue.insert(Job::Share(r.canonical_slot));
            }
        }
        TableId::CountryLinks => {
            if let Some(r) = row::<tables::CountryLinks>(frame, key)? {
                queue.insert(Job::Share(r.canonical_slot));
            }
        }
        TableId::UnitIntel => {
            queue.insert(Job::Level((key >> 32) as u64));
        }
        TableId::CountryIntel => {
            queue.insert(Job::Intel(key as u32));
        }
        TableId::Provinces => {
            queue.insert(Job::Controller(key as u32));
        }
        TableId::Divisions => {
            queue.insert(Job::Invasion(key as u64));
        }
        TableId::RelationLinks => {
            queue.insert(Job::Access(key as u32));
        }
        _ => {}
    }
    Ok(())
}
fn resolve(frame: &DecodedFrame, index: &Index, choice: &Choice) -> io::Result<(u32, Resolution)> {
    let _ = core::<tables::CountryLinks>(frame)?;
    let _ = core::<tables::Countries>(frame)?;
    let default = || -> io::Result<(u32, bool)> {
        if let Ok([player]) = frame.complete::<tables::LocalPlayer>() {
            if let Some(slot) = player
                .viewing_country()
                .and_then(|raw| index.links.get(&raw))
            {
                return Ok((*slot, true));
            }
        }
        index
            .groups
            .range((TableId::Countries.index(), 1, 0)..=(TableId::Countries.index(), 1, u128::MAX))
            .next()
            .map(|(&(.., slot), _)| (slot as u32, false))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("hour {} records no country to view the map as", frame.hour),
                )
            })
    };
    Ok(match &choice.country {
        Country::Recorded => {
            let (slot, recorded) = default()?;
            (
                slot,
                if recorded {
                    Resolution::Recorded
                } else {
                    Resolution::NoPlayer
                },
            )
        }
        Country::Tag(tag) => {
            let mut selected = None;
            for &key in index
                .groups
                .get(&(TableId::Countries.index(), 2, tag_hash(tag.as_bytes())))
                .into_iter()
                .flatten()
            {
                let country = row::<tables::Countries>(frame, key)?.unwrap();
                let last = *index.groups[&(
                    TableId::Countries.index(),
                    1,
                    country.canonical_slot as u128,
                )]
                    .last()
                    .unwrap();
                let actual = row::<tables::Countries>(frame, last)?.unwrap();
                if String::from_utf8_lossy(&actual.tag).eq_ignore_ascii_case(tag) {
                    selected = Some(selected.map_or(country.canonical_slot, |slot: u32| {
                        slot.min(country.canonical_slot)
                    }));
                }
            }
            match selected {
                Some(slot) => (slot, Resolution::Requested),
                None => (default()?.0, Resolution::Absent(tag.clone())),
            }
        }
        Country::Slot { slot, .. } => {
            let held = index
                .groups
                .get(&(TableId::Countries.index(), 1, *slot as u128))
                .is_some_and(|keys| !keys.is_empty());
            if held {
                (*slot, Resolution::Requested)
            } else {
                (default()?.0, Resolution::Absent(choice.country.to_string()))
            }
        }
    })
}
fn first_error(
    errors: &BTreeMap<(u16, Order, Job), &'static str>,
    stage: u16,
) -> Option<&'static str> {
    errors
        .range(
            (stage, Order(0, 0, 0, 0, [0; 4]), Job::Source((0, 0)))
                ..=(
                    stage,
                    Order(255, u32::MAX, u32::MAX, 255, [u128::MAX; 4]),
                    Job::Access(u32::MAX),
                ),
        )
        .next()
        .map(|(_, &reason)| reason)
}
fn complete_gap(frame: &DecodedFrame, table: TableId) -> Option<&'static str> {
    (!matches!(*frame.tables[table.index()], TableView::Complete(_))).then(|| match table {
        TableId::FactionMembers => "FactionMembers",
        TableId::WarRelations => "WarRelations",
        TableId::Subjects => "Subjects",
        TableId::RelationLinks => "RelationLinks",
        TableId::Countries => "Countries",
        TableId::Provinces => "Provinces",
        TableId::Divisions => "Divisions",
        TableId::TaskForces => "TaskForces",
        TableId::StateVisibility => "StateVisibility",
        TableId::States => "States",
        TableId::Fleets => "Fleets",
        TableId::LandTheatres => "LandTheatres",
        TableId::TheatreAreas => "TheatreAreas",
        TableId::StationedUnits => "StationedUnits",
        TableId::VolunteerHosts => "VolunteerHosts",
        TableId::RadarCoverage => "RadarCoverage",
        TableId::DivisionMovement => "DivisionMovement",
        _ => unreachable!(),
    })
}
fn relations_gap(
    frame: &DecodedFrame,
    errors: &BTreeMap<(u16, Order, Job), &'static str>,
) -> Option<&'static str> {
    for (table, stage) in [
        (TableId::FactionMembers, 0),
        (TableId::WarRelations, 1),
        (TableId::Subjects, 2),
    ] {
        if let Some(reason) = complete_gap(frame, table).or_else(|| first_error(errors, stage)) {
            return Some(reason);
        }
    }
    None
}
fn visibility_gap(
    frame: &DecodedFrame,
    index: &Index,
    errors: &BTreeMap<(u16, Order, Job), &'static str>,
    ours: u32,
) -> Option<&'static str> {
    macro_rules! whole {
        ($table:ident) => {
            if let Some(reason) = complete_gap(frame, TableId::$table) {
                return Some(reason);
            }
        };
    }
    macro_rules! bad {
        ($stage:expr) => {
            if let Some(reason) = first_error(errors, $stage) {
                return Some(reason);
            }
        };
    }
    whole!(RelationLinks);
    bad!(3);
    whole!(Countries);
    whole!(Provinces);
    whole!(Divisions);
    whole!(TaskForces);
    whole!(StateVisibility);
    bad!(10);
    bad!(11);
    whole!(States);
    bad!(12);
    bad!(13);
    whole!(Fleets);
    bad!(14);
    whole!(LandTheatres);
    bad!(15);
    bad!(16);
    whole!(TheatreAreas);
    whole!(StationedUnits);
    whole!(VolunteerHosts);
    bad!(17);
    whole!(RadarCoverage);
    bad!(18);
    if !index
        .groups
        .contains_key(&(TableId::Countries.index(), 1, ours as u128))
    {
        return Some("visibility viewing country");
    }
    bad!(40);
    whole!(DivisionMovement);
    bad!(50);
    None
}

impl<'a> Query<'a> {
    fn reserved(&mut self) -> Result<&'a replay_data::rows::ReservedCountryNamingRow> {
        self.work
            .dep(Dep::Row((TableId::ReservedCountryNaming.index(), 0)));
        let [row] = self
            .frame
            .complete::<tables::ReservedCountryNaming>()
            .map_err(|_| "ReservedCountryNaming")?
        else {
            return Err("ReservedCountryNaming singleton".into());
        };
        Ok(row)
    }
    fn zero_faction(&mut self, target: u32) -> Result<Option<u64>> {
        self.work.dep(Dep::FactionValidation);
        if !matches!(
            *self.frame.tables[TableId::FactionMembers.index()],
            TableView::Complete(_)
        ) {
            return Err("FactionMembers".into());
        }
        let invalid = self
            .index
            .invalid_factions
            .first_key_value()
            .map(|(&key, &reason)| (key, reason));
        let mut raws = self.raws(target)?;
        raws.retain(|&raw| raw != 0);
        if target == 0 {
            raws.insert(0, 0);
        }
        let mut faction = None;
        for raw in raws {
            if let Some((bad, reason)) = invalid {
                if bad as u128 <= raw {
                    return Err(reason.into());
                }
            }
            if let Some(row) = self.row::<tables::FactionMembers>(raw)? {
                if faction.is_some_and(|previous| previous != row.faction) {
                    return Err("FactionMembers has inconsistent faction identity".into());
                }
                faction = Some(row.faction);
            }
        }
        if let Some((_, reason)) = invalid {
            return Err(reason.into());
        }
        Ok(faction)
    }
    fn subject(
        &mut self,
        subject: &replay_data::rows::ZeroSubjectInputs,
        target: std::result::Result<u32, &'static str>,
    ) -> Result<bool> {
        if let Some(parties) = subject.parties {
            for raw in parties.into_iter().flatten() {
                self.work
                    .dep(Dep::Row((TableId::CountryLinks.index(), raw as u128)));
            }
        }
        if let Ok(raw) = target {
            self.work
                .dep(Dep::Row((TableId::CountryLinks.index(), raw as u128)));
        }
        Ok(replay_derive::visibility::map_intel_subject(
            subject,
            &self.index.links,
            target,
        )?)
    }
    fn reverse_zero(&mut self, target: u32) -> Result<bool> {
        let reserved = self.reserved()?;
        self.work.dep(Dep::Clock);
        let packet = match &reserved.reverse_visibility {
            Recorded::NotRecorded => {
                return Err("ReservedCountryNaming.reverse_visibility not recorded".into());
            }
            Recorded::Value(None) => {
                return Err("ReservedCountryNaming.reverse_visibility capture refused".into());
            }
            Recorded::Value(Some(packet)) => packet,
        };
        if !packet.sample_hour.fresh_at(self.frame.hour) {
            return Err("ReservedCountryNaming.reverse_visibility sample hour mismatch".into());
        }
        let cell = packet
            .cells
            .get(target as usize)
            .and_then(Option::as_ref)
            .ok_or("ReservedCountryNaming.reverse_visibility cell capture refused")?;
        if replay_derive::visibility::active_war(
            cell.war
                .ok_or("ReservedCountryNaming.reverse_visibility war capture refused")?,
        )? {
            return Ok(false);
        }
        let faction = replay_derive::visibility::reserved_faction(reserved)?;
        if faction != 0 && (target == 0 || self.zero_faction(target)? == Some(faction)) {
            return Ok(true);
        }
        let subject = cell
            .subject
            .as_ref()
            .ok_or("ReservedCountryNaming.reverse_visibility subject capture refused")?;
        self.subject(
            subject,
            cell.target
                .ok_or("ReservedCountryNaming.reverse_visibility target capture refused"),
        )
    }
    fn forward_zero(&mut self, owner: u32) -> Result<bool> {
        if !matches!(
            *self.frame.tables[TableId::CountryCounterInputs.index()],
            TableView::Complete(_)
        ) {
            return Err("CountryCounterInputs".into());
        }
        let mut row = None;
        for raw in self.raws(owner)? {
            if let Some(next) = self.row::<tables::CountryCounterInputs>(raw)? {
                if row.is_some() {
                    return Err("CountryCounterInputs visibility owner duplicated".into());
                }
                row = Some(next)
            }
        }
        let row = row.ok_or("CountryCounterInputs visibility owner missing")?;
        if !matches!(row.forward_visibility, Recorded::NotRecorded) {
            let cell = match &row.forward_visibility {
                Recorded::NotRecorded => unreachable!(),
                Recorded::Value(None) => {
                    return Err("CountryCounterInputs.forward_visibility capture refused".into());
                }
                Recorded::Value(Some(cell)) => cell,
            };
            if replay_derive::visibility::active_war(
                cell.war
                    .ok_or("CountryCounterInputs.forward_visibility war capture refused")?,
            )? {
                return Ok(false);
            }
            let faction = (|| {
                let Some(faction) = self.zero_faction(owner)? else {
                    return Ok(false);
                };
                Ok(replay_derive::visibility::reserved_faction(self.reserved()?)? == faction)
            })();
            if matches!(faction, Ok(true)) {
                return Ok(true);
            }
            let faction = match faction {
                Err(Failure::Resource(error)) => return Err(Failure::Resource(error)),
                Err(Failure::Gap(reason)) => Err(reason),
                Ok(value) => Ok(value),
            };
            let subject = (|| {
                let subject = cell
                    .subject
                    .as_ref()
                    .ok_or("CountryCounterInputs.forward_visibility subject capture refused")?;
                self.subject(
                    subject,
                    cell.target
                        .ok_or("CountryCounterInputs.forward_visibility target capture refused"),
                )
            })();
            if matches!(subject, Ok(true)) {
                return Ok(true);
            }
            if matches!(subject, Err(Failure::Resource(_))) {
                return subject;
            }
            faction?;
            return subject;
        }
        let war = match row.zero_war {
            Recorded::NotRecorded => {
                return Err("CountryCounterInputs.zero_war not recorded".into());
            }
            Recorded::Value(None) => {
                return Err("CountryCounterInputs.zero_war capture refused".into());
            }
            Recorded::Value(Some(war)) => war,
        };
        if replay_derive::visibility::active_war(war)? {
            return Ok(false);
        }
        if let Some(faction) = self.zero_faction(owner)? {
            if replay_derive::visibility::reserved_faction(self.reserved()?)? == faction {
                return Ok(true);
            }
        }
        let subject = match &row.zero_subject {
            Recorded::NotRecorded => {
                return Err("CountryCounterInputs.zero_subject not recorded".into());
            }
            Recorded::Value(None) => {
                return Err("CountryCounterInputs.zero_subject capture refused".into());
            }
            Recorded::Value(Some(subject)) => subject,
        };
        self.subject(subject, Ok(0))
    }
    fn zero_army(&mut self, raw_target: u32, target: u32) -> Result<i64> {
        if raw_target as i32 <= 0 {
            return Ok(0);
        }
        if target > i32::MAX as u32 {
            return Err("CountryIntel owner0 target outside native index range".into());
        }
        let reserved = self.reserved()?;
        self.work.dep(Dep::Clock);
        let sample = match &reserved.country_intel_sample {
            Recorded::NotRecorded => {
                return Err("ReservedCountryNaming.country_intel_sample not recorded".into());
            }
            Recorded::Value(None) => {
                return Err("ReservedCountryNaming.country_intel_sample capture refused".into());
            }
            Recorded::Value(Some(sample)) => sample,
        };
        if sample.sample_hour > self.frame.hour {
            return Err("ReservedCountryNaming.country_intel_sample is later than frame".into());
        }
        if sample.vector_count > i32::MAX as u32 {
            return Err(
                "ReservedCountryNaming.country_intel_sample count outside native range".into(),
            );
        }
        if !matches!(
            *self.frame.tables[TableId::CountryIntel.index()],
            TableView::Complete(_)
        ) {
            return Err("CountryIntel".into());
        }
        self.work.dep(Dep::ZeroBoundary);
        let last = self
            .index
            .groups
            .get(&(TableId::CountryIntel.index(), 2, 0))
            .and_then(Members::last)
            .copied();
        if last.is_some_and(|key| key >= sample.vector_count as u128) {
            return Err("CountryIntel owner0 keys outside admitted vector or duplicated".into());
        }
        if sample.vector_count > 0 && last != Some(sample.vector_count as u128 - 1) {
            return Err("CountryIntel owner0 final sentinel missing".into());
        }
        if target >= sample.vector_count {
            return Ok(0);
        }
        Ok(self
            .row::<tables::CountryIntel>(target as u128)?
            .map_or(0, |row| row.army))
    }
}

pub fn fields(table: TableId) -> u64 {
    static MASKS: std::sync::OnceLock<[u64; replay_data::rows::TABLE_COUNT]> =
        std::sync::OnceLock::new();
    MASKS.get_or_init(|| std::array::from_fn(|index| field_mask(TableId::ALL[index])))
        [table.index()]
}
fn field_mask(table: TableId) -> u64 {
    let names: &[&str] = match table {
        TableId::Countries => &["canonical_slot", "tag"],
        TableId::CountryCounterInputs => &["forward_visibility", "zero_war", "zero_subject"],
        TableId::ReservedCountryNaming => {
            &["faction", "reverse_visibility", "country_intel_sample"]
        }
        TableId::StateVisibility => &["network_modifier"],
        TableId::LandTheatres => &["owner", "volunteers_theatre", "ordinal"],
        TableId::Provinces => &["state", "controller"],
        TableId::States => &["owner", "controller"],
        TableId::Divisions => &["logical_owner", "roster_owner", "province", "reveal_skip"],
        TableId::DivisionMovement => &["path"],
        TableId::Fleets => &["owner"],
        TableId::TaskForces => &["fleet", "province", "logical_owner"],
        TableId::CountryVisibility => &["unit_modifier"],
        TableId::CountryIntel => &["army", "air", "navy"],
        TableId::SimulationFeatures => &["dlc_mask"],
        _ => return u64::MAX,
    };
    assert!(
        names.iter().all(
            |name| (0..table.schema().len()).any(|field| table.field_name(field) == Some(name))
        ),
        "missing Vision operand column for {table:?}"
    );
    (0..table.schema().len())
        .filter(|&field| {
            table
                .field_name(field)
                .is_some_and(|name| names.contains(&name))
        })
        .fold(0, |mask, field| mask | (1u64 << field))
}
