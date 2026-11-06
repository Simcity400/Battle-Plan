//! The repairs view: each owner's repair ports by province, their queues and the ships queued,
//! and its dockyard and queue totals.
use super::*;

const PORT: usize = port::CELLS - 2;
const TOTALS: usize = repair_totals::CELLS - 2;

fn at(cell: usize) -> usize {
    cell - 2
}

/// A port as the hour holds it.
pub(super) struct Port {
    pub(super) owner: u32,
    pub(super) cells: Box<[Spelled]>,
    /// Its queue's ships in queue order, repeats kept.
    queued: Vec<u64>,
    /// Its province's state, where recorded.
    state: Option<u32>,
}

impl Port {
    /// Its queue's ships, each once, ascending.
    fn unique(&self) -> Vec<u64> {
        let mut ships = self.queued.clone();
        ships.sort_unstable();
        ships.dedup();
        ships
    }
}

/// An owner's queue: its entries, and its ships with how many entries each has.
#[derive(Default)]
struct Queued {
    entries: usize,
    ships: HashMap<u64, usize>,
}

#[derive(Default)]
pub(super) struct Repairs {
    ports: HashMap<u32, Port>,
    queued: HashMap<u32, Queued>,
    /// The provinces of bases whose country resolves to no owner.
    unowned: HashSet<u32>,
    /// The ports of each state.
    states: HashMap<u32, Vec<u32>>,
    /// Each owner's totals' refusals.
    refused: HashMap<u32, Vec<(u32, u64)>>,
}

/// The port of `province` at the hour, `held` as last projected; none where it has none.
fn project<F: Tables + ?Sized>(
    cx: &mut Cx<'_, '_, F>,
    province: u32,
    held: Option<&Port>,
) -> Option<Port> {
    let frame = cx.frame;
    let base = frame.get::<tables::NavalBases>(province)?;
    let owner = cx.owner(base.country)?;
    let repair = frame.get::<tables::NavalRepairBases>(province);
    let entries = frame.complete::<tables::NavalRepairQueue>().map(|rows| {
        let first = u128::from(province) << 32;
        frame.each(rows.range(first..=first | u128::from(u32::MAX)))
    });
    if repair.is_none() && entries.as_ref().is_none_or(Vec::is_empty) {
        return None;
    }
    let mut cells = vec![Spelled::Null; PORT];
    let state = frame
        .get::<tables::Provinces>(province)
        .map(|row| row.state);
    if let Some(state) = state {
        cells[at(port::STATE)] = Spelled::Integer(i128::from(state));
    }
    let name = Operand::Port(port_operand(frame, cx.countries, province));
    cells[at(port::NAME)] = Spelled::Token(label(
        held.map(|port| &port.cells[at(port::NAME)]),
        name,
    ));
    if let Some(repair) = repair {
        cells[at(port::PRIORITY)] = Spelled::Integer(i128::from(repair.priority));
        cells[at(port::DOCKYARDS)] = Spelled::Integer(i128::from(repair.allocated_dockyards));
        cells[at(port::POLICY)] = Spelled::Token(cx.shared.policy(true, repair.priority));
    }
    let mut queued = Vec::new();
    if let Some(entries) = entries {
        cells[at(port::QUEUE_COUNT)] = Spelled::Integer(entries.len() as i128);
        if !entries.is_empty() {
            cells[at(port::QUEUE)] = Spelled::List(
                entries
                    .iter()
                    .flat_map(|entry| {
                        [
                            Spelled::Integer(i128::from(entry.ordinal)),
                            Spelled::Integer(i128::from(entry.ship)),
                        ]
                    })
                    .collect(),
            );
        }
        queued.extend(entries.iter().map(|entry| entry.ship));
    }
    Some(Port {
        owner,
        cells: cells.into_boxed_slice(),
        queued,
        state,
    })
}

impl Repairs {
    /// Every port `frame` holds, into `projector`'s ships and rows.
    pub(super) fn build<F: Tables + ?Sized>(
        cx: &mut Cx<'_, '_, F>,
        ships: &mut Ships,
        dirty: &mut Dirty,
    ) -> Self {
        let mut repairs = Self::default();
        let provinces: Vec<u32> = cx
            .frame
            .complete::<tables::NavalBases>()
            .map(|bases| bases.keys().map(|province| province as u32).collect())
            .unwrap_or_default();
        for province in provinces {
            repairs.port(cx, ships, dirty, province);
        }
        repairs
    }

    /// The port of `province` projected again, and what that changes.
    fn port<F: Tables + ?Sized>(
        &mut self,
        cx: &mut Cx<'_, '_, F>,
        ships: &mut Ships,
        dirty: &mut Dirty,
        province: u32,
    ) {
        let unowned = cx
            .frame
            .get::<tables::NavalBases>(province)
            .is_some_and(|base| cx.owner(base.country).is_none());
        if unowned != self.unowned.contains(&province) {
            if unowned {
                self.unowned.insert(province);
            } else {
                self.unowned.remove(&province);
            }
            // Every owner's queue census reads whether every base resolves.
            dirty.totals_all = true;
        }
        let old = self.ports.remove(&province);
        let now = project(cx, province, old.as_ref());
        if let Some(old) = &old {
            if let Some(state) = old.state {
                remove(&mut self.states, state, province);
            }
            dirty.place(old.owner, layout::HOLDERS, u64::from(province));
            dirty.totals(old.owner);
            let queued = self.queued.entry(old.owner).or_default();
            queued.entries -= old.queued.len();
            for ship in &old.queued {
                if let Some(count) = queued.ships.get_mut(ship) {
                    *count -= 1;
                    if *count == 0 {
                        queued.ships.remove(ship);
                    }
                }
            }
            if queued.entries == 0 {
                self.queued.remove(&old.owner);
            }
        }
        if let Some(now) = &now {
            if let Some(state) = now.state {
                self.states.entry(state).or_default().push(province);
            }
            dirty.place(now.owner, layout::HOLDERS, u64::from(province));
            dirty.totals(now.owner);
            if !now.queued.is_empty() {
                let queued = self.queued.entry(now.owner).or_default();
                queued.entries += now.queued.len();
                for &ship in &now.queued {
                    *queued.ships.entry(ship).or_default() += 1;
                }
            }
        }
        let before = old.as_ref().map(|port| (port.owner, port.unique()));
        let after = now.as_ref().map(|port| (port.owner, port.unique()));
        ships.rehold(cx, dirty, u64::from(province), before, after);
        if let Some(now) = now {
            self.ports.insert(province, now);
        }
    }

    /// The hour's changes to the ports, through `touched`.
    pub(super) fn hour<F: Tables + ?Sized>(
        &mut self,
        cx: &mut Cx<'_, '_, F>,
        ships: &mut Ships,
        dirty: &mut Dirty,
        touched: &Touched,
    ) {
        let mut provinces = BTreeSet::new();
        for table in [
            TableId::NavalBases,
            TableId::NavalRepairBases,
            TableId::Provinces,
            TableId::ProvinceNames,
        ] {
            provinces.extend(keys(touched.of(table)).map(|key| key as u32));
        }
        provinces.extend(keys(touched.of(TableId::NavalRepairQueue)).map(|key| (key >> 32) as u32));
        for state in keys(touched.of(TableId::States)) {
            provinces.extend(self.states.get(&(state as u32)).into_iter().flatten());
        }
        for province in provinces {
            self.port(cx, ships, dirty, province);
        }
        for table in [TableId::NavalRepairCountries, TableId::CountryFactories] {
            for key in keys(touched.of(table)) {
                if let Some(owner) = cx.owner(key as u32) {
                    dirty.totals(owner);
                }
            }
        }
        // Whether every queued ship is recorded reads the ships it gains and loses.
        let ships_touched = touched.of(TableId::Ships);
        for (key, mask) in &ships_touched.written {
            if *mask == u64::MAX {
                ships.owners_of(*key as u64, |owner| dirty.totals(owner));
            }
        }
        for key in &ships_touched.deleted {
            ships.owners_of(*key as u64, |owner| dirty.totals(owner));
        }
    }

    /// `owner`'s totals row at the hour, and the refusals it counts there.
    pub(super) fn totals<F: Tables + ?Sized>(
        &mut self,
        cx: &mut Cx<'_, '_, F>,
        dirty: &mut Dirty,
        owner: u32,
    ) -> Option<Box<[Spelled]>> {
        let frame = cx.frame;
        let mut cells = vec![Spelled::Null; TOTALS];
        let mut refused = Vec::new();
        // The last of an owner's rows holds each of its values.
        if let Some(rows) = frame.complete::<tables::NavalRepairCountries>()
            && let Some(row) = cx
                .raws(owner)
                .iter()
                .rev()
                .find_map(|&raw| frame.found(rows.get(u128::from(raw))))
        {
            cells[at(repair_totals::USED_DOCKYARDS)] =
                Spelled::Integer(i128::from(row.num_used_dockyards));
            cells[at(repair_totals::REPAIR_LIMIT)] =
                Spelled::Integer(i128::from(row.max_allowed_dockyards_to_repair));
            cells[at(repair_totals::BUILDING_REPAIR_LIMIT)] =
                Spelled::Integer(i128::from(row.max_factories_for_repair));
        }
        if let Some(rows) = frame.complete::<tables::CountryFactories>()
            && let Some(row) = cx
                .raws(owner)
                .iter()
                .rev()
                .find_map(|&raw| frame.found(rows.get(u128::from(raw))))
        {
            cells[at(repair_totals::DOCKYARDS_RAW)] =
                Spelled::Integer(i128::from(row.naval_total_raw));
            cells[at(repair_totals::DOCKYARDS)] =
                Spelled::Integer(i128::from(row.naval_total_raw / 100_000));
        }
        if frame.complete::<tables::NavalBases>().is_none() {
            refused.push((cx.reasons.number("Repair port ownership not recorded"), 1));
        } else {
            let complete = self.unowned.is_empty()
                && frame.complete::<tables::NavalRepairQueue>().is_some();
            cells[at(repair_totals::QUEUE_CENSUS_COMPLETE)] =
                Spelled::Integer(i128::from(complete));
            if !complete {
                refused.push((
                    cx.reasons.number(
                        "Repair queue or port ownership not recorded; empty queue cannot be established",
                    ),
                    1,
                ));
            } else {
                let queued = self.queued.get(&owner);
                let entries = queued.map_or(0, |queued| queued.entries);
                let distinct = queued.map_or(0, |queued| queued.ships.len());
                cells[at(repair_totals::QUEUE_ENTRIES)] = Spelled::Integer(entries as i128);
                cells[at(repair_totals::DISTINCT_QUEUED_REFERENCES)] =
                    Spelled::Integer(distinct as i128);
                if let Some(rows) = frame.complete::<tables::Ships>()
                    && queued.is_none_or(|queued| {
                        queued
                            .ships
                            .keys()
                            .all(|&ship| rows.contains(u128::from(ship)))
                    })
                {
                    cells[at(repair_totals::DISTINCT_QUEUED_SHIPS)] =
                        Spelled::Integer(distinct as i128);
                }
                if entries > 0 {
                    refused.push((
                        cx.reasons.number(
                            "Repair rate and ETA unavailable: the exact repair rate is not recorded",
                        ),
                        entries as u64,
                    ));
                }
            }
        }
        let old = self.refused.remove(&owner).unwrap_or_default();
        cx.refusals.shift(dirty, owner, &old, &refused);
        if !refused.is_empty() {
            self.refused.insert(owner, refused);
        }
        cells
            .iter()
            .any(|cell| *cell != Spelled::Null)
            .then(|| cells.into_boxed_slice())
    }

    pub(super) fn holder(&self, owner: u32, key: u64) -> Option<&[Spelled]> {
        self.ports
            .get(&u32::try_from(key).ok()?)
            .filter(|port| port.owner == owner)
            .map(|port| &*port.cells)
    }
}
