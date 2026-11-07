use super::*;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{Hour, Operator, Sink, World as _},
};
use replay_data::rows::{
    CountryLinkRow, CountryRow, DivisionLandInputsRow, DivisionManpowerContributor, DivisionRow,
    DivisionPanelStatsRow, Recorded, RowCodec, StateRow, tables,
};
use std::collections::{BTreeMap, BTreeSet};

const FORMAT: FormatVersion = FormatVersion::CURRENT;
const TABLES: [TableId; 3] = [TableId::Countries, TableId::CountryLinks, TableId::States];

/// Countries, links and states as an hour records them: by id, a country's slot and tag, a
/// link's slot, and a state's owner.
#[derive(Clone)]
struct Model {
    countries: BTreeMap<u32, (u32, Vec<u8>)>,
    links: BTreeMap<u32, u32>,
    states: BTreeMap<u32, u32>,
    /// Countries unavailable this hour.
    refused: bool,
    next: u32,
}

/// A deterministic draw below `below`.
fn draw(seed: &mut u64, below: u32) -> u32 {
    *seed = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    ((*seed >> 33) % u64::from(below.max(1))) as u32
}

fn pick<T: Copy>(seed: &mut u64, keys: impl Iterator<Item = T>) -> Option<T> {
    let keys: Vec<T> = keys.collect();
    (!keys.is_empty()).then(|| keys[draw(seed, keys.len() as u32) as usize])
}

impl Model {
    /// `countries` countries, each linked to its slot, every seventh slot named by a second
    /// raw id too, and `states` states owned round the countries, every eleventh by an alias
    /// and every thirteenth by a raw id no link names.
    fn new(countries: u32, states: u32) -> Self {
        let mut model = Self {
            countries: BTreeMap::new(),
            links: BTreeMap::new(),
            states: BTreeMap::new(),
            refused: false,
            next: 100_000,
        };
        for slot in 0..countries {
            model
                .countries
                .insert(1_000 + slot, (slot, format!("T{slot:04}").into_bytes()));
            model.links.insert(1_000 + slot, slot);
            if slot % 7 == 0 {
                model.links.insert(50_000 + slot, slot);
            }
        }
        for state in 0..states {
            let slot = state % countries.max(1);
            let owner = match state {
                _ if state % 13 == 0 => 99_999,
                _ if state % 11 == 0 && slot % 7 == 0 => 50_000 + slot,
                _ => 1_000 + slot,
            };
            model.states.insert(state + 1, owner);
        }
        model
    }

    /// One random edit of the countries, links or states.
    fn churn(&mut self, seed: &mut u64) {
        let slots = self.countries.len() as u32 + 4;
        match draw(seed, 10) {
            0 => {
                if let Some(id) = pick(seed, self.countries.keys().copied()) {
                    // Sometimes another country's tag: a tag held twice.
                    let tag = match pick(seed, self.countries.values().map(|(_, tag)| tag)) {
                        Some(tag) if draw(seed, 2) == 0 => tag.clone(),
                        _ => format!("D{:04}", draw(seed, 50)).into_bytes(),
                    };
                    self.countries.get_mut(&id).expect("a country").1 = tag;
                }
            }
            1 => {
                if let Some(id) = pick(seed, self.countries.keys().copied()) {
                    self.countries.get_mut(&id).expect("a country").0 = draw(seed, slots);
                }
            }
            2 => {
                if let Some(id) = pick(seed, self.countries.keys().copied()) {
                    self.countries.remove(&id);
                }
            }
            3 => {
                self.next += 1;
                let slot = draw(seed, slots);
                self.countries.insert(
                    self.next,
                    (slot, format!("N{:04}", self.next % 97).into_bytes()),
                );
                if draw(seed, 2) == 0 {
                    self.links.insert(self.next, slot);
                }
            }
            4 => {
                if let Some(raw) = pick(seed, self.links.keys().copied()) {
                    self.links.insert(raw, draw(seed, slots));
                }
            }
            5 => {
                if let Some(raw) = pick(seed, self.links.keys().copied()) {
                    self.links.remove(&raw);
                }
            }
            6 => {
                self.next += 1;
                self.links.insert(self.next, draw(seed, slots));
            }
            7 | 8 => {
                if let Some(state) = pick(seed, self.states.keys().copied()) {
                    let owner = match pick(seed, self.links.keys().copied()) {
                        Some(raw) if draw(seed, 5) != 0 => raw,
                        _ => 99_999,
                    };
                    self.states.insert(state, owner);
                }
            }
            _ => {
                if draw(seed, 2) == 0 {
                    if let Some(state) = pick(seed, self.states.keys().copied()) {
                        self.states.remove(&state);
                    }
                } else {
                    self.next += 1;
                    let owner = pick(seed, self.links.keys().copied()).unwrap_or(99_999);
                    self.states.insert(self.next, owner);
                }
            }
        }
    }

    fn write(&self, walk: &mut Synthetic, hour: i32) {
        let countries: Vec<CountryRow> = self
            .countries
            .iter()
            .map(|(&id, (slot, tag))| CountryRow {
                id,
                canonical_slot: *slot,
                tag: tag.clone(),
                // A balance the roster never reads, moved every hour.
                stability_raw: i64::from(hour),
                ..Default::default()
            })
            .collect();
        let links: Vec<CountryLinkRow> = self
            .links
            .iter()
            .map(|(&id, &canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect();
        let states: Vec<StateRow> = self
            .states
            .iter()
            .map(|(&id, &owner)| StateRow {
                id,
                owner,
                ..Default::default()
            })
            .collect();
        if self.refused {
            walk.table::<CountryRow>(
                TableId::Countries,
                Input::Unavailable {
                    reason: "synthetic refusal",
                    read: true,
                    sample: Sample::FromReadBit,
                },
            )
            .unwrap();
        } else {
            snapshot(walk, TableId::Countries, &countries);
        }
        snapshot(walk, TableId::CountryLinks, &links);
        snapshot(walk, TableId::States, &states);
    }
}

fn snapshot<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: &[R]) {
    walk.table(
        table,
        Input::Snapshot {
            rows,
            partial: None,
            read: true,
            sample: Sample::FromReadBit,
        },
    )
    .unwrap();
}

fn needs() -> Needs {
    Needs {
        countries: true,
        owners: true,
        divisions: false,
        naming: false,
        lexicon: false,
        flags: false,
        frame: vec![
            (
                TableId::Countries,
                Hold::Fields(
                    1 << replay::columns::field(TableId::Countries, "tag").unwrap()
                        | 1 << replay::columns::field(TableId::Countries, "canonical_slot")
                            .unwrap(),
                ),
            ),
            (TableId::CountryLinks, Hold::Whole),
        ],
    }
}

/// What a world holds of the countries and owners, in bytes rather than its own tag numbers.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    complete: (bool, bool, bool),
    countries: BTreeMap<u32, (u32, Vec<u8>, bool)>,
    links: BTreeMap<u32, u32>,
    aliases: BTreeMap<u32, Vec<u32>>,
    /// Each held tag's holders: id, slot and resolution, by id.
    holders: BTreeMap<Vec<u8>, Vec<(u32, u32, bool)>>,
    tagged: BTreeMap<Vec<u8>, Option<u32>>,
    owners: BTreeMap<u32, u32>,
    owned: BTreeMap<u32, u32>,
    unresolved: u32,
}

fn seen(world: &World, states: impl Iterator<Item = u32>) -> Seen {
    let roster = world.countries().unwrap();
    let owners = world.owners().unwrap();
    let mut slots: BTreeSet<u32> = roster
        .links()
        .iter()
        .map(|link| link.canonical_slot)
        .collect();
    slots.extend(roster.iter().map(|country| country.slot));
    let mut holders = BTreeMap::new();
    let mut tagged = BTreeMap::new();
    for tag in roster.held() {
        let bytes = roster.tag(tag).to_vec();
        holders.insert(
            bytes.clone(),
            roster
                .holders(tag)
                .map(|country| (country.id, country.slot, country.resolved))
                .collect(),
        );
        tagged.insert(
            bytes,
            match roster.tagged(tag) {
                Tagged::Found(country) => Some(country.id),
                Tagged::Unresolved => None,
                Tagged::Absent => panic!("a held tag is absent"),
            },
        );
    }
    Seen {
        complete: (
            roster.countries_complete(),
            roster.links_complete(),
            owners.complete(),
        ),
        countries: roster
            .iter()
            .map(|country| {
                (
                    country.id,
                    (
                        country.slot,
                        roster.tag(country.tag).to_vec(),
                        country.resolved,
                    ),
                )
            })
            .collect(),
        links: roster
            .links()
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect(),
        aliases: slots
            .iter()
            .filter(|&&slot| !roster.aliases(slot).is_empty())
            .map(|&slot| (slot, roster.aliases(slot).to_vec()))
            .collect(),
        holders,
        tagged,
        owners: states
            .filter_map(|state| owners.owner(state).map(|owner| (state, owner)))
            .collect(),
        owned: slots
            .iter()
            .filter(|&&slot| owners.owned(slot) > 0)
            .map(|&slot| (slot, owners.owned(slot)))
            .collect(),
        unresolved: owners.unresolved(),
    }
}

/// The model's countries and owners as a world must hold them.
fn expected(model: &Model) -> Seen {
    let resolved = |id: u32, slot: u32| model.links.get(&id) == Some(&slot);
    let countries: BTreeMap<u32, (u32, Vec<u8>, bool)> = if model.refused {
        BTreeMap::new()
    } else {
        model
            .countries
            .iter()
            .map(|(&id, (slot, tag))| (id, (*slot, tag.clone(), resolved(id, *slot))))
            .collect()
    };
    let mut holders: BTreeMap<Vec<u8>, Vec<(u32, u32, bool)>> = BTreeMap::new();
    for (&id, (slot, tag, resolved)) in &countries {
        holders
            .entry(tag.clone())
            .or_default()
            .push((id, *slot, *resolved));
    }
    let tagged = holders
        .iter()
        .map(|(tag, held)| {
            (
                tag.clone(),
                held.iter().find(|holder| holder.2).map(|holder| holder.0),
            )
        })
        .collect();
    let mut aliases: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&raw, &slot) in &model.links {
        aliases.entry(slot).or_default().push(raw);
    }
    let mut owned: BTreeMap<u32, u32> = BTreeMap::new();
    let mut unresolved = 0;
    for owner in model.states.values() {
        match model.links.get(owner) {
            Some(&slot) => *owned.entry(slot).or_default() += 1,
            None => unresolved += 1,
        }
    }
    Seen {
        complete: (!model.refused, true, true),
        countries,
        links: model.links.clone(),
        aliases,
        holders,
        tagged,
        owners: model.states.clone(),
        owned,
        unresolved,
    }
}

/// What changed from `before` to `after`: ids, tags, links, slots reaching or leaving
/// ownership, and states given another owner.
type Moves = (Vec<u32>, Vec<Vec<u8>>, Vec<u32>, Vec<u32>, Vec<u32>);

fn moves(before: &Seen, after: &Seen) -> Moves {
    fn differ<K: Ord + Clone, V: PartialEq>(
        before: &BTreeMap<K, V>,
        after: &BTreeMap<K, V>,
    ) -> Vec<K> {
        let keys: BTreeSet<&K> = before.keys().chain(after.keys()).collect();
        keys.into_iter()
            .filter(|key| before.get(*key) != after.get(*key))
            .cloned()
            .collect()
    }
    let owning = |seen: &Seen| -> BTreeMap<u32, bool> {
        seen.owned.keys().map(|&slot| (slot, true)).collect()
    };
    (
        differ(&before.countries, &after.countries),
        differ(&before.holders, &after.holders),
        differ(&before.links, &after.links),
        differ(&owning(before), &owning(after)),
        differ(&before.owners, &after.owners),
    )
}

fn reported(world: &World) -> Moves {
    let roster = world.countries().unwrap();
    let owners = world.owners().unwrap();
    let mut tags: Vec<Vec<u8>> = roster
        .changed_tags()
        .iter()
        .map(|&tag| roster.tag(tag).to_vec())
        .collect();
    tags.sort_unstable();
    (
        roster.changed_ids().to_vec(),
        tags,
        roster.changed_links().to_vec(),
        owners.reached().to_vec(),
        owners.moved().to_vec(),
    )
}

/// Over churned hours, with segments starting every ninth hour and the countries unavailable
/// for an hour, the world kept from changed rows holds what one built at the hour from
/// scratch holds and the recorded tables say, reports exactly what changed, and its frame
/// holds the row decoder's rows in the fields it watches.
#[test]
fn the_world_kept_from_changes_holds_what_a_rebuild_holds() {
    let mut model = Model::new(40, 90);
    let mut seed = 7;
    let mut walk = Synthetic::new(&TABLES, 1, 0).unwrap();
    let mut world = World::new(&needs(), FORMAT, CaptureMode::Full).unwrap();
    let mut before: Option<Seen> = None;
    for hour in 0..120 {
        if hour > 0 {
            for _ in 0..1 + draw(&mut seed, 4) {
                model.churn(&mut seed);
            }
        }
        model.refused = hour == 50;
        let opens = hour % 9 == 0;
        if opens {
            world = World::new(&needs(), FORMAT, CaptureMode::Full).unwrap();
        }
        model.write(&mut walk, hour);
        walk.end(|state, manifest, frame, _| {
            world.hour(state, manifest).unwrap();
            let mut scratch = World::new(&needs(), FORMAT, CaptureMode::Full).unwrap();
            scratch.hour(state, manifest).unwrap();
            let states = || model.states.keys().copied();
            let now = seen(&world, states());
            assert_eq!(now, seen(&scratch, states()), "hour {hour}");
            assert_eq!(now, expected(&model), "hour {hour}");
            let roster = world.countries().unwrap();
            assert_eq!(world.first(), opens, "hour {hour}");
            let rebuilt = opens || hour == 50 || hour == 51;
            assert_eq!(roster.rebuilt(), rebuilt, "hour {hour}");
            if let Some(before) = before.as_ref().filter(|_| !rebuilt) {
                assert_eq!(reported(&world), moves(before, &now), "hour {hour}");
            }
            let shared = world.identity(state).unwrap();
            assert_eq!(
                shared.complete::<tables::CountryLinks>().ok(),
                frame.complete::<tables::CountryLinks>().ok(),
                "hour {hour}"
            );
            let named = |rows: &[CountryRow]| -> Vec<(u32, u32, Vec<u8>)> {
                rows.iter()
                    .map(|row| (row.id, row.canonical_slot, row.tag.clone()))
                    .collect()
            };
            assert_eq!(
                shared.complete::<tables::Countries>().ok().map(named),
                frame.complete::<tables::Countries>().ok().map(named),
                "hour {hour}"
            );
            before = Some(now);
        })
        .unwrap();
    }
}

/// An hour of what an operator read of its world.
#[derive(replay_spool::Spooled)]
struct Read(Vec<u8>);

/// Reads each country's tag, identity and states owned.
struct Owning;

impl Owning {
    fn needs() -> Needs {
        Needs {
            countries: true,
            owners: true,
            ..Needs::default()
        }
    }
}

impl Operator<World> for Owning {
    type Output = Read;

    fn hour(&mut self, hour: &Hour<'_, World>, out: &mut dyn Sink<Read>) -> io::Result<()> {
        let (roster, owners) = (hour.world.countries()?, hour.world.owners()?);
        let mut read = Vec::new();
        for country in roster.iter() {
            read.extend_from_slice(roster.tag(country.tag));
            read.extend_from_slice(&country.id.to_le_bytes());
            read.extend_from_slice(&country.slot.to_le_bytes());
            read.push(u8::from(country.resolved));
            read.extend_from_slice(&owners.owned(country.slot).to_le_bytes());
        }
        read.extend_from_slice(&owners.unresolved().to_le_bytes());
        out.push(Read(read))
    }
}

/// Reads States from the frame, and from its fifth hour CountryLinks too, which it never
/// declared: the frame widens to it.
struct Framing;

impl Framing {
    fn needs() -> Needs {
        Needs {
            frame: vec![(TableId::States, Hold::Whole)],
            ..Needs::default()
        }
    }
}

impl Operator<World> for Framing {
    type Output = Read;

    fn hour(&mut self, hour: &Hour<'_, World>, out: &mut dyn Sink<Read>) -> io::Result<()> {
        let widened = hour.hour % 20 >= 5;
        let tables: &[TableId] = if widened {
            &[TableId::States, TableId::CountryLinks]
        } else {
            &[TableId::States]
        };
        let frame = hour.world.frame(hour.state, tables)?;
        let mut read = Vec::new();
        for state in frame.complete::<tables::States>()? {
            read.extend_from_slice(&state.id.to_le_bytes());
            read.extend_from_slice(&state.owner.to_le_bytes());
        }
        if widened {
            for link in frame.complete::<tables::CountryLinks>()? {
                read.extend_from_slice(&link.id.to_le_bytes());
                read.extend_from_slice(&link.canonical_slot.to_le_bytes());
            }
        }
        out.push(Read(read))
    }
}

/// Hour `hour` of a walk seeing `world`.
fn walked<'a>(
    (hour, starts, state, manifest): (i32, bool, &'a State, &'a Manifest<'a>),
    world: &'a World,
) -> Hour<'a, World> {
    Hour {
        branch: 1,
        hour,
        run: 0,
        ordinal: hour as u32,
        starts,
        opens: hour % 20 == 0,
        state,
        manifest,
        world,
    }
}

/// Two operators reading one world see what each sees of a world of its own: over churned
/// hours, with segments starting every twentieth hour.
#[test]
fn operators_sharing_a_world_read_what_each_reads_alone() {
    let mut model = Model::new(25, 60);
    let mut seed = 11;
    let mut walk = Synthetic::new(&TABLES, 1, 0).unwrap();
    let union = Needs::union([&Owning::needs(), &Framing::needs()]);
    let make = |needs: &Needs| World::new(needs, FORMAT, CaptureMode::Full).unwrap();
    let (mut shared, mut alone) = (
        make(&union),
        [make(&Owning::needs()), make(&Framing::needs())],
    );
    let (mut together, mut apart): ([Vec<Read>; 2], [Vec<Read>; 2]) = Default::default();
    for hour in 0..80 {
        if hour > 0 {
            model.churn(&mut seed);
            model.churn(&mut seed);
        }
        if hour % 20 == 0 {
            shared = make(&union);
            alone = [make(&Owning::needs()), make(&Framing::needs())];
        }
        model.write(&mut walk, hour);
        walk.end(|state, manifest, _, starts| {
            let at = (hour, starts, state, manifest);
            shared.hour(state, manifest).unwrap();
            Owning.hour(&walked(at, &shared), &mut together[0]).unwrap();
            Framing
                .hour(&walked(at, &shared), &mut together[1])
                .unwrap();
            for world in &mut alone {
                world.hour(state, manifest).unwrap();
            }
            Owning.hour(&walked(at, &alone[0]), &mut apart[0]).unwrap();
            Framing.hour(&walked(at, &alone[1]), &mut apart[1]).unwrap();
        })
        .unwrap();
    }
    for (together, apart) in together.iter().zip(&apart) {
        assert_eq!(together.len(), 80);
        assert!(together.iter().zip(apart).all(|(one, two)| one.0 == two.0));
    }
}

/// With 1,500 countries, their links and 4,000 states, the first hour reads every row, and each
/// later hour reads only the rows it changed: a quiet hour, none.
#[test]
fn work_per_hour_follows_the_changed_rows_of_1500_countries() {
    let mut model = Model::new(1_500, 4_000);
    let mut walk = Synthetic::new(&TABLES, 1, 0).unwrap();
    let mut world = World::new(&needs(), FORMAT, CaptureMode::Full).unwrap();
    let mut read = 0;
    for hour in 0..24_u32 {
        // Hour h retags h % 4 countries and moves h % 3 states: a quiet hour every twelfth.
        let edits = (hour % 4, hour % 3);
        for at in 0..edits.0 {
            let id = 1_000 + (hour * 37 + at * 101) % 1_500;
            model.countries.get_mut(&id).expect("a country").1 =
                format!("R{hour:02}{at}").into_bytes();
        }
        for at in 0..edits.1 {
            let state = 1 + (hour * 53 + at * 211) % 4_000;
            let mut owner = 1_000 + (hour * 7 + at) % 1_500;
            if model.states.get(&state) == Some(&owner) {
                owner = 1_000 + (owner - 999) % 1_500;
            }
            model.states.insert(state, owner);
        }
        model.write(&mut walk, hour as i32);
        walk.end(|state, manifest, _, _| {
            world.hour(state, manifest).unwrap();
            let reads = world.countries().unwrap().reads + world.owners().unwrap().reads;
            let hour_reads = reads - read;
            read = reads;
            if hour == 0 {
                assert_eq!(hour_reads, 1_500 + model.links.len() + 4_000);
            } else {
                // Every country's balance moved; only the edited rows were read.
                assert_eq!(hour_reads, (edits.0 + edits.1) as usize, "hour {hour}");
                assert_eq!(
                    world.countries().unwrap().changed_ids().len(),
                    edits.0 as usize,
                    "hour {hour}"
                );
            }
        })
        .unwrap();
    }
}

/// Each country's flag inputs, in bytes: cosmetic tag, original reference and tag, ruling
/// party and its token.
type Flagged = (
    Option<(Vec<u8>, Recorded<Option<i32>>, Recorded<Option<Vec<u8>>>)>,
    Option<Option<u32>>,
    Option<Vec<u8>>,
);

fn flagged(world: &World, ids: impl Iterator<Item = u32>) -> BTreeMap<u32, Flagged> {
    let roster = world.countries().unwrap();
    let flags = world.flags().unwrap();
    let text = |tag| roster.tag(tag).to_vec();
    ids.map(|id| {
        let inputs = flags.inputs(id);
        (
            id,
            (
                inputs.appearance.map(|appearance| {
                    (
                        text(appearance.cosmetic),
                        appearance.reference,
                        match appearance.original {
                            Recorded::Value(original) => Recorded::Value(original.map(text)),
                            Recorded::NotRecorded => Recorded::NotRecorded,
                        },
                    )
                }),
                inputs.ruling,
                inputs.group.map(text),
            ),
        )
    })
    .collect()
}

/// Over churned cosmetic tags, original tags, ruling parties and party tokens, each country's
/// flag inputs kept from changed rows equal those read from scratch and the rows recorded,
/// and the countries reported changed are exactly those whose inputs changed.
#[test]
fn flag_inputs_kept_from_changes_hold_what_a_rebuild_holds() {
    use replay_data::rows::{CountryAppearanceRow, PoliticalPartyRow, PoliticsRow};
    let tables = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryAppearance,
        TableId::Politics,
        TableId::PoliticalParties,
    ];
    let needs = Needs {
        flags: true,
        ..Needs::default()
    };
    let ids: Vec<u32> = (1..=30).collect();
    let mut cosmetic: BTreeMap<u32, (Vec<u8>, i32)> =
        ids.iter().map(|&id| (id, (Vec::new(), 0))).collect();
    let mut ruling: BTreeMap<u32, Option<u32>> = ids.iter().map(|&id| (id, Some(1))).collect();
    let mut tokens: BTreeMap<(u32, u32), Vec<u8>> = ids
        .iter()
        .flat_map(|&id| [((id, 1), b"left".to_vec()), ((id, 2), b"right".to_vec())])
        .collect();
    let mut seed = 5;
    let mut walk = Synthetic::new(&tables, 1, 0).unwrap();
    let mut world = World::new(&needs, FORMAT, CaptureMode::Full).unwrap();
    let mut before: Option<BTreeMap<u32, Flagged>> = None;
    for hour in 0..90 {
        if hour > 0 {
            for _ in 0..1 + draw(&mut seed, 3) {
                let id = 1 + draw(&mut seed, 30);
                match draw(&mut seed, 6) {
                    0 => {
                        cosmetic.get_mut(&id).unwrap().0 =
                            format!("C{}", draw(&mut seed, 4)).into_bytes()
                    }
                    1 => cosmetic.get_mut(&id).unwrap().1 = draw(&mut seed, 3) as i32 - 1,
                    2 => {
                        ruling.insert(
                            id,
                            [None, Some(1), Some(2), Some(3)][draw(&mut seed, 4) as usize],
                        );
                    }
                    3 => {
                        let party = 1 + draw(&mut seed, 3);
                        tokens.insert((id, party), format!("g{}", draw(&mut seed, 3)).into_bytes());
                    }
                    4 => {
                        tokens.remove(&(id, 1 + draw(&mut seed, 3)));
                    }
                    _ => {
                        if ruling.remove(&id).is_none() {
                            ruling.insert(id, Some(2));
                        }
                    }
                }
            }
        }
        if hour % 30 == 0 {
            world = World::new(&needs, FORMAT, CaptureMode::Full).unwrap();
        }
        let countries: Vec<CountryRow> = ids
            .iter()
            .map(|&id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("F{id:02}").into_bytes(),
                ..Default::default()
            })
            .collect();
        let links: Vec<CountryLinkRow> = ids
            .iter()
            .map(|&id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect();
        let appearances: Vec<CountryAppearanceRow> = cosmetic
            .iter()
            .map(|(&country, (tag, reference))| CountryAppearanceRow {
                country,
                cosmetic_tag: tag.clone(),
                original_tag_reference: Recorded::Value(Some(*reference)),
                original_tag: Recorded::Value(Some(format!("O{reference}").into_bytes())),
                // A colour no flag reads, moved every hour.
                red: hour as u32,
                ..Default::default()
            })
            .collect();
        let politics: Vec<PoliticsRow> = ruling
            .iter()
            .map(|(&country, &ruling_party)| PoliticsRow {
                country,
                ruling_party,
                ..Default::default()
            })
            .collect();
        let parties: Vec<PoliticalPartyRow> = tokens
            .iter()
            .map(|(&(country, id), token)| PoliticalPartyRow {
                country,
                id,
                token: token.clone(),
                popularity: i64::from(hour),
                ..Default::default()
            })
            .collect();
        snapshot(&mut walk, TableId::Countries, &countries);
        snapshot(&mut walk, TableId::CountryLinks, &links);
        snapshot(&mut walk, TableId::CountryAppearance, &appearances);
        snapshot(&mut walk, TableId::Politics, &politics);
        snapshot(&mut walk, TableId::PoliticalParties, &parties);
        walk.end(|state, manifest, _, _| {
            world.hour(state, manifest).unwrap();
            let mut scratch = World::new(&needs, FORMAT, CaptureMode::Full).unwrap();
            scratch.hour(state, manifest).unwrap();
            let now = flagged(&world, ids.iter().copied());
            assert_eq!(now, flagged(&scratch, ids.iter().copied()), "hour {hour}");
            for (id, (appearance, ruled, group)) in &now {
                let (tag, reference) = &cosmetic[id];
                assert_eq!(
                    appearance.as_ref().map(|held| &held.0),
                    Some(tag),
                    "hour {hour}"
                );
                assert_eq!(
                    appearance.as_ref().map(|held| held.1),
                    Some(Recorded::Value(Some(*reference)))
                );
                assert_eq!(*ruled, ruling.get(id).copied(), "hour {hour}");
                let party = ruling.get(id).copied().flatten();
                assert_eq!(
                    *group,
                    party.and_then(|party| tokens.get(&(*id, party)).cloned())
                );
            }
            let flags = world.flags().unwrap();
            assert_eq!(flags.rebuilt(), hour % 30 == 0, "hour {hour}");
            if let Some(before) = before.as_ref().filter(|_| !flags.rebuilt()) {
                let moved: Vec<u32> = ids
                    .iter()
                    .copied()
                    .filter(|id| before[id] != now[id])
                    .collect();
                assert_eq!(flags.changed(), moved.as_slice(), "hour {hour}");
            }
            before = Some(now);
        })
        .unwrap();
    }
}

/// Divisions, their status and land inputs as an hour records them, by division.
struct Army {
    divisions: BTreeMap<u64, DivisionRow>,
    status: BTreeMap<u64, i64>,
    land: BTreeMap<u64, Vec<i32>>,
    /// Divisions, DivisionPanelStats and DivisionLandInputs unavailable this hour.
    refused: [bool; 3],
    next: u64,
}

impl Army {
    fn new(count: u64) -> Self {
        let mut army = Self {
            divisions: BTreeMap::new(),
            status: BTreeMap::new(),
            land: BTreeMap::new(),
            refused: [false; 3],
            next: count,
        };
        for id in 1..=count {
            army.add(id);
        }
        army
    }

    fn add(&mut self, id: u64) {
        self.divisions.insert(
            id,
            DivisionRow {
                id,
                roster_owner: (id % 7) as u32,
                logical_owner: (id % 5) as u32,
                expeditionary_owner: if id % 11 == 0 { 3 } else { 0 },
                template: 100 + id % 4,
                // Not a whole tenth: the stepped column keeps the tenth toward zero.
                hit_points: 123_456 + id as i64,
                organisation: 1,
                reveal_skip: u8::from(id % 13 == 0),
                ..Default::default()
            },
        );
        self.status.insert(id, 1_234_567);
        if id % 3 != 0 {
            self.land.insert(id, vec![i32::MAX, id as i32]);
        }
    }

    fn churn(&mut self, seed: &mut u64) {
        match draw(seed, 8) {
            0 => {
                if let Some(id) = pick(seed, self.divisions.keys().copied()) {
                    self.divisions.get_mut(&id).expect("a division").hit_points -= 54_321;
                }
            }
            1 => {
                if let Some(id) = pick(seed, self.divisions.keys().copied()) {
                    self.divisions
                        .get_mut(&id)
                        .expect("a division")
                        .roster_owner = draw(seed, 9);
                }
            }
            2 => {
                if let Some(id) = pick(seed, self.divisions.keys().copied()) {
                    self.divisions.remove(&id);
                    self.status.remove(&id);
                    self.land.remove(&id);
                }
            }
            3 => {
                self.next += 1;
                self.add(self.next);
            }
            4 => {
                if let Some(id) = pick(seed, self.status.keys().copied()) {
                    self.status
                        .insert(id, 2_000_000 + i64::from(draw(seed, 99_999)));
                }
            }
            5 => {
                if let Some(id) = pick(seed, self.land.keys().copied()) {
                    self.land.insert(id, vec![draw(seed, 1_000) as i32, -7]);
                }
            }
            // Organisation, which nothing reads.
            _ => {
                if let Some(id) = pick(seed, self.divisions.keys().copied()) {
                    self.divisions
                        .get_mut(&id)
                        .expect("a division")
                        .organisation += 1;
                }
            }
        }
    }

    fn write(&self, walk: &mut Synthetic) {
        let refuse = |walk: &mut Synthetic, table| {
            walk.table::<DivisionRow>(
                table,
                Input::Unavailable {
                    reason: "synthetic refusal",
                    read: true,
                    sample: Sample::FromReadBit,
                },
            )
            .unwrap()
        };
        let divisions: Vec<DivisionRow> = self.divisions.values().cloned().collect();
        let status: Vec<DivisionPanelStatsRow> = self
            .status
            .iter()
            .map(|(&division, &maximum_hit_points)| {
                let mut row = DivisionPanelStatsRow {
                    division,
                    ..Default::default()
                };
                row.panel_stats[0] = Some(maximum_hit_points);
                row
            })
            .collect();
        let land: Vec<DivisionLandInputsRow> = self
            .land
            .iter()
            .map(|(&division, amounts)| DivisionLandInputsRow {
                division,
                manpower: amounts
                    .iter()
                    .map(|&amount| DivisionManpowerContributor { country: 1, amount })
                    .collect(),
                ..Default::default()
            })
            .collect();
        if self.refused[0] {
            refuse(walk, TableId::Divisions);
        } else {
            snapshot(walk, TableId::Divisions, &divisions);
        }
        if self.refused[1] {
            refuse(walk, TableId::DivisionPanelStats);
        } else {
            snapshot(walk, TableId::DivisionPanelStats, &status);
        }
        if self.refused[2] {
            refuse(walk, TableId::DivisionLandInputs);
        } else {
            snapshot(walk, TableId::DivisionLandInputs, &land);
        }
    }
}

/// What a world holds of the divisions: each table's values, or none while it is not held.
#[derive(Debug, PartialEq, Eq)]
struct Armed {
    divisions: Option<BTreeMap<u64, Division>>,
    maxima: Option<BTreeMap<u64, Option<i64>>>,
    manpower: Option<BTreeMap<u64, i32>>,
}

fn held<T: Copy + PartialEq>(kept: &Kept<T>) -> Option<BTreeMap<u64, T>> {
    kept.holds()
        .then(|| kept.iter().map(|(id, value)| (id, *value)).collect())
}

fn armed(world: &World) -> Armed {
    let divisions = world.divisions().unwrap();
    Armed {
        divisions: held(divisions.divisions()),
        maxima: held(divisions.maxima()),
        manpower: held(divisions.manpower()),
    }
}

/// The same from the row decoder's frame.
fn decoded(frame: &DecodedFrame) -> Armed {
    Armed {
        divisions: frame.complete::<tables::Divisions>().ok().map(|rows| {
            rows.iter()
                .map(|row| {
                    let division = Division {
                        id: row.id,
                        roster_owner: row.roster_owner,
                        logical_owner: row.logical_owner,
                        expeditionary_owner: row.expeditionary_owner,
                        template: row.template,
                        hit_points: row.hit_points,
                        reveal_skip: row.reveal_skip,
                    };
                    (row.id, division)
                })
                .collect()
        }),
        maxima: frame.complete::<tables::DivisionPanelStats>().ok().map(|rows| {
            rows.iter()
                .map(|row| (row.division, row.maximum_hit_points()))
                .collect()
        }),
        manpower: frame
            .complete::<tables::DivisionLandInputs>()
            .ok()
            .map(|rows| {
                rows.iter()
                    .map(|row| {
                        let sum = row
                            .manpower
                            .iter()
                            .fold(0_i32, |sum, part| sum.wrapping_add(part.amount));
                        (row.division, sum)
                    })
                    .collect()
            }),
    }
}

/// What takes `before` to `after`: each changed value.
type Changed<T> = BTreeMap<u64, (Option<T>, Option<T>)>;

fn changed<T: Copy + PartialEq>(
    before: &Option<BTreeMap<u64, T>>,
    after: &Option<BTreeMap<u64, T>>,
) -> Changed<T> {
    let (Some(before), Some(after)) = (before, after) else {
        return BTreeMap::new();
    };
    before
        .keys()
        .chain(after.keys())
        .filter(|id| before.get(id) != after.get(id))
        .map(|&id| (id, (before.get(&id).copied(), after.get(&id).copied())))
        .collect()
}

fn reported_edits<T: Copy + PartialEq>(kept: &Kept<T>) -> Changed<T> {
    kept.edits()
        .iter()
        .map(|&(id, before, after)| (id, (before, after)))
        .collect()
}

/// Over churned hours of 1,500 divisions, segments opening every tenth hour and each table
/// unavailable for an hour, the divisions kept from changed rows are those the row decoder
/// reads, stepped hit points in native units; the edits are exactly what changed in the fields
/// read, organisation never; and a table is read whole again once it is held again.
#[test]
fn divisions_kept_from_changes_are_the_row_decoder_s() {
    let tables = [
        TableId::Divisions,
        TableId::DivisionPanelStats,
        TableId::DivisionLandInputs,
    ];
    let needs = Needs {
        divisions: true,
        ..Needs::default()
    };
    let mut army = Army::new(1_500);
    let mut seed = 11;
    let mut walk = Synthetic::new(&tables, 1, 0).unwrap();
    let mut world = World::new(&needs, FORMAT, CaptureMode::Full).unwrap();
    let mut before: Option<Armed> = None;
    for hour in 0..80 {
        if hour > 0 {
            for _ in 0..1 + draw(&mut seed, 6) {
                army.churn(&mut seed);
            }
        }
        army.refused = [hour == 30, hour == 40, hour == 50];
        let opens = hour % 10 == 0;
        if opens {
            world = World::new(&needs, FORMAT, CaptureMode::Full).unwrap();
        }
        army.write(&mut walk);
        walk.end(|state, manifest, frame, _| {
            world.hour(state, manifest).unwrap();
            let now = armed(&world);
            assert_eq!(now, decoded(frame), "hour {hour}");
            let parts = world.divisions().unwrap();
            let again = [hour == 31, hour == 41, hour == 51];
            let wholes = [
                parts.divisions().whole(),
                parts.maxima().whole(),
                parts.manpower().whole(),
            ];
            for (at, whole) in wholes.into_iter().enumerate() {
                let held = !army.refused[at];
                assert_eq!(
                    whole,
                    held && (opens || again[at]),
                    "hour {hour} table {at}"
                );
            }
            if let Some(before) = before.as_ref().filter(|_| !opens) {
                if !wholes[0] {
                    assert_eq!(
                        reported_edits(parts.divisions()),
                        changed(&before.divisions, &now.divisions),
                        "hour {hour}"
                    );
                }
                if !wholes[1] {
                    assert_eq!(
                        reported_edits(parts.maxima()),
                        changed(&before.maxima, &now.maxima),
                        "hour {hour}"
                    );
                }
                if !wholes[2] {
                    assert_eq!(
                        reported_edits(parts.manpower()),
                        changed(&before.manpower, &now.manpower),
                        "hour {hour}"
                    );
                }
            }
            if let Some(division) = now.divisions.as_ref().and_then(|held| held.get(&1)) {
                assert_eq!(division.hit_points % 10_000, 0, "hour {hour}");
            }
            before = Some(now);
        })
        .unwrap();
    }
}
