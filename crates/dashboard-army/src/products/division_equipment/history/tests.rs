//! Division equipment's family on synthetic hours, walked by segments as preparation walks
//! them: an operator per segment on its own world, the emit stepping each restart's hours
//! before its reads are its own on the engine the segment before handed on. Where no content
//! is bound every need is refused and every design unclassified: the parts, sums and reads are
//! what is checked; with [`content`], each division's need is compared too.

use super::{
    emit::{self, Emit, SCHEMA},
    facts::{self, Hour, Projected, Projector},
};
use dashboard_series::{
    Blob, Identity, Publication, Snapshot, Span, Writer,
    family::{Directory, Scalar},
    memory::{Memory, publish},
};
use dashboard_text::catalogue::{Catalogue, tests::Folder};
use dashboard_world::World;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::{
    cadence::cadence_in,
    format::FormatVersion,
    rows::{
        CampaignRow, CountryAppearanceRow, CountryLinkRow, CountryLocalSubunitRow, CountryRow,
        DivisionEquipmentRow, DivisionLandInputsRow, DivisionManpowerContributor, DivisionRow,
        DivisionStatusRow, DivisionTemplateIconRow, DivisionTemplateRow, LexicalTokenRow, Recorded,
        RowCodec, StockpileRow, TableId, TemplateSettings, TemplateSubunit, VariantDefinitionRow,
    },
    scope::CaptureMode,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const FORMAT: FormatVersion = FormatVersion::CURRENT;
const BRANCH: u64 = 1;
const VARIANTS: [u64; 3] = [11, 12, 13];
const TEMPLATES: [u64; 2] = [101, 102];
/// The unclassified type's name in [`read`]'s names.
const UNCLASSIFIED: &str = "null";
/// [`content`]'s line, its one need, of `NEED` raw a line, and the designs' equipment.
const LINE: &str = "test_line";
const KIND: &str = "widgets";
const NEED: i128 = 10_000_000;
const DOCUMENT: &str = r#"
sub_units = { test_line = { type = infantry need = { widgets = 100 } } }
equipments = {
    widgets = { is_archetype = yes type = infantry }
    widgets_1 = { archetype = widgets }
}
"#;

/// Content of one line needing one equipment, which every design is a model of.
fn content() -> (Arc<Catalogue>, Folder) {
    let (cat, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        ("common/units/equipment/test.txt", DOCUMENT),
        ("common/units/test.txt", DOCUMENT),
    ]);
    (Arc::new(cat), folder)
}

/// Design `id`'s name, the first design's another when `renamed`.
fn design_name(id: u64, renamed: bool) -> String {
    if renamed && id == VARIANTS[0] {
        "Rifle renamed".to_owned()
    } else {
        format!("Rifle {id}")
    }
}

/// The lines template `template` holds: one in the first, two in the second.
fn lines(template: u64) -> i32 {
    if template == TEMPLATES[0] { 1 } else { 2 }
}

type Pools = BTreeMap<u64, Vec<(u64, i64)>>;

fn identity() -> Identity {
    Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

/// A deterministic draw below `below`.
fn draw(seed: &mut u64, below: u64) -> u64 {
    *seed = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    (*seed >> 33) % below.max(1)
}

/// Whether `table` is read at `hour`: at its cadence's slot, and every table at the
/// branch's first hour.
fn reads(table: TableId, hour: i32) -> bool {
    hour == 0 || cadence_in(table, FORMAT).is_refresh(hour)
}

/// What an hour holds: the countries, their divisions and what the next pool and stockpile
/// reads find.
#[derive(Clone)]
struct Model {
    countries: u32,
    divisions: BTreeMap<u64, DivisionRow>,
    maxima: BTreeMap<u64, i64>,
    manpower: BTreeMap<u64, i32>,
    pools: Pools,
    stock: BTreeMap<u32, Vec<(u64, i64)>>,
    /// Countries are unavailable this hour.
    hole: bool,
    /// The pool read this hour is refused.
    refused: bool,
    /// The first design is defined by another name.
    renamed: bool,
    next: u64,
}

/// What the daily pool and stockpile tables hold between their reads.
#[derive(Default)]
struct Carried {
    pools: Pools,
    stock: BTreeMap<u32, Vec<(u64, i64)>>,
}

impl Model {
    fn new(countries: u32, per: u64) -> Self {
        let mut model = Self {
            countries,
            divisions: BTreeMap::new(),
            maxima: BTreeMap::new(),
            manpower: BTreeMap::new(),
            pools: BTreeMap::new(),
            stock: BTreeMap::new(),
            hole: false,
            refused: false,
            renamed: false,
            next: 0,
        };
        for country in 1..=countries {
            for _ in 0..per {
                model.add(country);
            }
            model.stock.insert(
                country,
                vec![(VARIANTS[0], 7_000_000), (VARIANTS[2], 300_000)],
            );
        }
        model
    }

    fn add(&mut self, country: u32) {
        self.next += 1;
        let id = self.next;
        let n = id as usize;
        self.divisions.insert(
            id,
            DivisionRow {
                id,
                roster_owner: country,
                logical_owner: country,
                template: TEMPLATES[n % 2],
                hit_points: 100_000 * (1 + id as i64 % 9),
                // Every eleventh is a reveal copy, which no reply counts.
                reveal_skip: u8::from(id % 11 == 0),
                ..Default::default()
            },
        );
        self.maxima.insert(id, 1_000_000);
        self.manpower.insert(id, 900 + id as i32);
        // About a line's need: some divisions are short of it, others hold enough.
        self.pools.insert(
            id,
            vec![
                (VARIANTS[n % 3], 4_000_000 * (1 + id as i64 % 4)),
                (VARIANTS[(n + 1) % 3], 2_000_000),
            ],
        );
    }

    fn churn(&mut self, seed: &mut u64) {
        if self.divisions.is_empty() {
            self.add(1 + draw(seed, u64::from(self.countries)) as u32);
            return;
        }
        let ids: Vec<u64> = self.divisions.keys().copied().collect();
        let picked = ids[draw(seed, ids.len() as u64) as usize];
        let country = 1 + draw(seed, u64::from(self.countries)) as u32;
        match draw(seed, 8) {
            0 | 1 => {
                let row = self.divisions.get_mut(&picked).expect("a division");
                row.hit_points = 100_000 * (1 + draw(seed, 9) as i64);
            }
            2 => {
                let variant = VARIANTS[draw(seed, 3) as usize];
                let amount = 1_000_000 * (1 + picked as i64 % 25);
                self.pools.insert(picked, vec![(variant, amount)]);
            }
            3 => {
                let row = self.divisions.get_mut(&picked).expect("a division");
                row.roster_owner = country;
            }
            4 => self.add(country),
            5 => {
                self.divisions.remove(&picked);
                self.maxima.remove(&picked);
                self.manpower.remove(&picked);
                self.pools.remove(&picked);
            }
            6 => {
                self.manpower.insert(picked, draw(seed, 5_000) as i32);
            }
            _ => {
                let amount = 100_000 * draw(seed, 50) as i64;
                self.stock.insert(country, vec![(VARIANTS[1], amount)]);
            }
        }
    }

    /// The hour's rows, the daily tables as their last read found them.
    fn write(&self, walk: &mut Synthetic, hour: i32, carried: &mut Carried) {
        let refused = self.refused && reads(TableId::DivisionEquipment, hour);
        if reads(TableId::DivisionEquipment, hour) && !refused {
            carried.pools = self.pools.clone();
        }
        if reads(TableId::Stockpiles, hour) {
            carried.stock = self.stock.clone();
        }
        let countries: Vec<CountryRow> = (1..=self.countries)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("C{id:04}").into_bytes(),
                ..Default::default()
            })
            .collect();
        let links: Vec<CountryLinkRow> = (1..=self.countries)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect();
        let tokens = [
            LexicalTokenRow {
                token: 1,
                literal: LINE.as_bytes().to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: KIND.as_bytes().to_vec(),
            },
            LexicalTokenRow {
                token: 3,
                literal: b"widgets_1".to_vec(),
            },
        ];
        let templates: Vec<DivisionTemplateRow> = TEMPLATES
            .iter()
            .map(|&template| DivisionTemplateRow {
                template,
                owner: 1,
                name: format!("Template {template}").into_bytes(),
                ..Default::default()
            })
            .collect();
        let icons: Vec<DivisionTemplateIconRow> = TEMPLATES
            .iter()
            .map(|&template| DivisionTemplateIconRow {
                template,
                country: 1,
                subunits: vec![TemplateSubunit {
                    token: 1,
                    local: 0,
                    count: lines(template),
                }],
                settings: Recorded::Value(Some(Arc::new(TemplateSettings {
                    total_manpower: 1_000,
                    ..Default::default()
                }))),
                ..Default::default()
            })
            .collect();
        let variants: Vec<VariantDefinitionRow> = VARIANTS
            .iter()
            .map(|&id| VariantDefinitionRow {
                id,
                equipment_token: 3,
                creator_link: 1,
                origin_link: 1,
                display_name: design_name(id, self.renamed).into_bytes(),
                ..Default::default()
            })
            .collect();
        let divisions: Vec<DivisionRow> = self.divisions.values().cloned().collect();
        let status: Vec<DivisionStatusRow> = self
            .maxima
            .iter()
            .map(|(&division, &maximum_hit_points)| DivisionStatusRow {
                division,
                maximum_hit_points,
                ..Default::default()
            })
            .collect();
        let land: Vec<DivisionLandInputsRow> = self
            .manpower
            .iter()
            .map(|(&division, &amount)| DivisionLandInputsRow {
                division,
                manpower: vec![DivisionManpowerContributor { country: 1, amount }],
                ..Default::default()
            })
            .collect();
        let pools: Vec<DivisionEquipmentRow> = carried
            .pools
            .iter()
            .flat_map(|(&division, rows)| {
                rows.iter()
                    .enumerate()
                    .map(move |(ordinal, &(variant, amount))| DivisionEquipmentRow {
                        division,
                        ordinal: ordinal as u32,
                        variant_handle: variant,
                        amount_raw: amount,
                        ..Default::default()
                    })
            })
            .collect();
        let stock: Vec<StockpileRow> = carried
            .stock
            .iter()
            .flat_map(|(&country, rows)| {
                rows.iter()
                    .enumerate()
                    .map(move |(ordinal, &(variant, amount))| StockpileRow {
                        country,
                        ordinal: ordinal as u32,
                        variant_handle: variant,
                        amount_raw: amount,
                    })
            })
            .collect();
        let campaign = [CampaignRow {
            id: b"campaign".to_vec(),
        }];
        let countries = (!self.hole).then_some(&countries[..]);
        put(walk, hour, TableId::Campaign, Some(&campaign[..]));
        put(walk, hour, TableId::Countries, countries);
        put(walk, hour, TableId::CountryLinks, Some(&links[..]));
        put::<CountryAppearanceRow>(walk, hour, TableId::CountryAppearance, Some(&[][..]));
        put(walk, hour, TableId::LexicalTokens, Some(&tokens[..]));
        put::<CountryLocalSubunitRow>(walk, hour, TableId::CountryLocalSubunits, Some(&[][..]));
        put(walk, hour, TableId::DivisionTemplates, Some(&templates[..]));
        put(walk, hour, TableId::DivisionTemplateIcons, Some(&icons[..]));
        put(walk, hour, TableId::VariantDefinitions, Some(&variants[..]));
        put(walk, hour, TableId::Divisions, Some(&divisions[..]));
        put(walk, hour, TableId::DivisionStatus, Some(&status[..]));
        put(walk, hour, TableId::DivisionLandInputs, Some(&land[..]));
        put::<DivisionEquipmentRow>(walk, hour, TableId::NewDivisionEquipment, Some(&[][..]));
        put(
            walk,
            hour,
            TableId::DivisionEquipment,
            (!refused).then_some(&pools[..]),
        );
        put(walk, hour, TableId::Stockpiles, Some(&stock[..]));
    }
}

/// `rows` as `table`'s rows this hour, or the table unavailable.
fn put<R: RowCodec>(walk: &mut Synthetic, hour: i32, table: TableId, rows: Option<&[R]>) {
    let (read, sample) = (reads(table, hour), Sample::FromReadBit);
    let input = match rows {
        Some(rows) => Input::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        None => Input::Unavailable {
            reason: "synthetic refusal",
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

/// The model's hours from hour 0, a segment starting there and at each of `starts`, each
/// walked by an operator and a world of its own; their items in walk order.
fn project(models: &[Model], starts: &[i32]) -> Vec<Hour> {
    project_with(models, starts, None)
}

/// [`project`] reading content `cat`.
fn project_with(models: &[Model], starts: &[i32], cat: Option<Arc<Catalogue>>) -> Vec<Hour> {
    let mut walk = Synthetic::new(facts::TABLES, BRANCH, 0).unwrap();
    let needs = facts::needs().unwrap();
    let mut held: Option<World> = None;
    let mut projector: Option<Projector> = None;
    let mut carried = Carried::default();
    let mut out = Vec::new();
    for (ordinal, model) in models.iter().enumerate() {
        let hour = ordinal as i32;
        let opens = hour == 0 || starts.contains(&hour);
        if opens {
            if let Some(mut done) = projector.take() {
                done.end(&mut out).unwrap();
            }
            let mut operator =
                Projector::within(FORMAT, (vec![(BRANCH, 0)], Vec::new()), cat.clone()).unwrap();
            let start = match hour {
                0 => Start::Run { continues: false },
                _ => Start::Restart(walk.state()),
            };
            assert!(operator.starts_at(start));
            operator.start(start, BRANCH, hour).unwrap();
            projector = Some(operator);
        }
        model.write(&mut walk, hour, &mut carried);
        let operator = projector.as_mut().expect("a started operator");
        walk.end(|state, manifest, _, _| {
            let world = World::at(
                &mut held,
                opens,
                &needs,
                (FORMAT, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            operator
                .hour(
                    &segments::Hour {
                        branch: BRANCH,
                        hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts: opens,
                        opens,
                        state,
                        manifest,
                        world,
                    },
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
    }
    if let Some(mut done) = projector.take() {
        done.end(&mut out).unwrap();
    }
    // As its emit is handed them: written to the spool and read back.
    replay_spool::respool(out).unwrap()
}

/// The hours segments led: each a prefix of its segment, before its reads are its own.
fn leads(items: &[Hour]) -> BTreeSet<i32> {
    items
        .iter()
        .filter(|item| matches!(item.projected, Projected::Lead(_)))
        .map(|item| item.hour)
        .collect()
}

/// A publication of projected hours, cut after each of `cuts`' hours; at `resume`, one of
/// them, the emit is saved and goes on from the publication.
struct Published {
    pages: Memory,
    root: u64,
}

fn published(hours: Vec<Hour>, cuts: &[i32], resume: Option<i32>) -> Published {
    published_with(hours, cuts, resume, None)
}

/// [`published`] reading content `cat`.
fn published_with(
    hours: Vec<Hour>,
    cuts: &[i32],
    resume: Option<i32>,
    cat: Option<Arc<Catalogue>>,
) -> Published {
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(identity(), first).unwrap();
    let mut emit = Emit::new(cat.clone());
    let mut root = 0;
    let mut hours = hours.into_iter().peekable();
    for (generation, &cut) in (1..).zip(cuts) {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        root = publish(
            &mut writer,
            &mut pages,
            Publication {
                generation,
                segment: generation - 1,
                accepted: Span::new(first, cut).unwrap(),
                reached: cut,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        if resume == Some(cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(cat.clone(), &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Published { pages, root }
}

/// One value's runs: first and last hour, value as read, and whether it starts anew.
type Runs = Vec<(i32, i32, String, bool)>;

/// `key`'s runs, one going on across a cut kept as one.
fn runs_of(snapshot: &mut Snapshot<'_, Memory>, key: u64) -> Runs {
    let mut runs: Runs = Vec::new();
    for run in dashboard_base::prepared::engine::runs(snapshot, key, 0, 1 << 20).unwrap() {
        let value = run.value.to_string();
        match runs.last_mut() {
            Some(last) if last.1 + 1 == run.span.first && last.2 == value && !run.break_before => {
                last.1 = run.span.last;
            }
            _ => runs.push((run.span.first, run.span.last, value, run.break_before)),
        }
    }
    runs
}

/// What a publication holds: every value's runs by country, attribution, measure and
/// named dimension, each operand read out of the catalogue (a design without the hour of
/// the read that first named it), so two numberings of one history read alike.
fn read(published: &Published) -> BTreeMap<String, Runs> {
    let mut snapshot =
        Snapshot::open(published.pages.clone(), published.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let text = |scalar: Option<&Scalar>| match scalar {
        Some(Scalar::Text(text)) => String::from_utf8(text.to_vec()).unwrap(),
        Some(Scalar::Null) => "null".to_owned(),
        other => panic!("not an operand: {other:?}"),
    };
    let design = |number: u64| {
        let mut raw: serde_json::Value =
            serde_json::from_str(&text(directory.scalar(emit::DESIGNS + number))).unwrap();
        raw["first_hour"] = serde_json::Value::Null;
        raw.to_string()
    };
    let mut held = BTreeMap::new();
    for (measure, name) in [
        (emit::KNOWN, "known"),
        (emit::POOL, "pool"),
        (emit::STOCK, "stock"),
    ] {
        if let Some(key) = directory.key(emit::FAMILY, measure, None) {
            held.insert(format!("*|{name}"), runs_of(&mut snapshot, key));
        }
    }
    for (actor, described) in directory.actors.iter().enumerate() {
        let tag = String::from_utf8(described.name.to_vec()).unwrap();
        let actor = actor as u64;
        for measure in 0..2 * emit::SIDE {
            let (side, code) = (measure / emit::SIDE, measure % emit::SIDE);
            let mut keys: Vec<(String, u64)> = Vec::new();
            if let Some(key) = directory.key(actor, measure, None) {
                keys.push((String::new(), key));
            }
            for &(dimension, key) in directory.dimensions(actor, measure) {
                let named = match code {
                    _ if (emit::TYPE..emit::VARIANT).contains(&code) => {
                        text(directory.scalar(emit::KINDS + dimension))
                    }
                    emit::VARIANT | emit::DESIGN => {
                        let pair = directory.scalar(emit::PAIRS + dimension).unwrap();
                        let [kind, handle] = pair.list() else {
                            panic!("a kind and design pair");
                        };
                        let kind = kind.integer().unwrap() as u64;
                        format!(
                            "{}|{}",
                            text(directory.scalar(emit::KINDS + kind)),
                            handle.integer().unwrap()
                        )
                    }
                    emit::NOTE => text(directory.scalar(emit::NOTES + dimension)),
                    _ => dimension.to_string(),
                };
                keys.push((named, key));
            }
            let operands = match code {
                emit::DESIGN | emit::STOCK_DESIGN => Some(emit::DESIGNS),
                _ if code == emit::TEMPLATE + 9 => Some(emit::NAMINGS),
                _ if code == emit::TEMPLATE + 10 => Some(emit::ARTWORKS),
                _ => None,
            };
            for (named, key) in keys {
                let mut runs = runs_of(&mut snapshot, key);
                if let Some(range) = operands {
                    for run in &mut runs {
                        let number: u64 = run.2.parse().unwrap();
                        run.2 = match range {
                            emit::DESIGNS => design(number),
                            range => text(directory.scalar(range + number)),
                        };
                    }
                }
                if !runs.is_empty() {
                    held.insert(format!("{tag}|{side}|{code}|{named}"), runs);
                }
            }
        }
    }
    held
}

/// The value `name` holds at `hour`; none where it holds none.
fn at(held: &BTreeMap<String, Runs>, name: &str, hour: i32) -> Option<String> {
    held.get(name)?
        .iter()
        .find(|run| run.0 <= hour && hour <= run.1)
        .map(|run| run.2.clone())
}

/// `hours` hours of `countries` countries with `per` divisions each at first, a few rows
/// changing every hour.
fn models(countries: u32, per: u64, hours: usize, seed: &mut u64) -> Vec<Model> {
    let mut model = Model::new(countries, per);
    let mut models = Vec::with_capacity(hours);
    for hour in 0..hours {
        if hour > 0 {
            for _ in 0..1 + draw(seed, 3) {
                model.churn(seed);
            }
        }
        models.push(model.clone());
    }
    models
}

/// The pools the replies stand on at `hour`: the last read's, for a day; none before a
/// read, past a day or from a refused read.
fn pooled(models: &[Model], hour: usize) -> Option<&Pools> {
    let read = (hour.saturating_sub(23)..=hour)
        .rev()
        .find(|&at| reads(TableId::DivisionEquipment, at as i32))?;
    (!models[read].refused).then(|| &models[read].pools)
}

/// What one country's owner reply holds at an hour, from the model: its divisions, the sum
/// of its divisions' pools (none while no read stands or none holds any) and each
/// template's hit points.
struct Expected {
    divisions: usize,
    held: Option<i128>,
    hp: BTreeMap<u64, i128>,
}

fn expected(model: &Model, pools: Option<&Pools>, country: u32) -> Expected {
    let members: Vec<&DivisionRow> = model
        .divisions
        .values()
        .filter(|row| row.reveal_skip == 0 && row.roster_owner == country)
        .collect();
    let mut hp = BTreeMap::new();
    for row in &members {
        *hp.entry(row.template).or_default() += i128::from(row.hit_points);
    }
    let held = pools.and_then(|pools| {
        let amounts: Vec<i128> = members
            .iter()
            .flat_map(|row| pools.get(&row.id).into_iter().flatten())
            .map(|&(_, amount)| i128::from(amount))
            .collect();
        (!amounts.is_empty()).then(|| amounts.iter().sum())
    });
    Expected {
        divisions: members.len(),
        held,
        hp,
    }
}

/// The owner reply's divisions, unknown comparisons, unclassified holdings and template hit
/// points at `hour`, as the model has them.
fn assert_sums(held: &BTreeMap<String, Runs>, models: &[Model], hour: usize, country: u32) {
    let tag = format!("C{country:04}");
    let value = |name: String| at(held, &format!("{tag}|0|{name}"), hour as i32);
    let expect = expected(&models[hour], pooled(models, hour), country);
    let divisions = Some(expect.divisions.to_string());
    assert_eq!(
        value(format!("{}|", emit::DIVISIONS)),
        divisions,
        "{tag} at {hour}"
    );
    // No content: no division's need is known.
    assert_eq!(
        value(format!("{}|", emit::UNKNOWN)),
        divisions,
        "{tag} at {hour}"
    );
    assert_eq!(
        value(format!("{}|{UNCLASSIFIED}", emit::TYPE)),
        expect.held.map(|held| held.to_string()),
        "{tag} holds at {hour}"
    );
    for template in TEMPLATES {
        assert_eq!(
            value(format!("{}|{template}", emit::TEMPLATE + 5)),
            expect.hp.get(&template).map(i128::to_string),
            "{tag} template {template} hit points at {hour}"
        );
    }
}

/// Whether a designs read stands at `hour`: one made within the day.
fn designs_read(hour: usize) -> bool {
    designs_read_at(hour).is_some()
}

/// The hour of the designs read standing at `hour`.
fn designs_read_at(hour: usize) -> Option<usize> {
    (hour.saturating_sub(23)..=hour)
        .rev()
        .find(|&at| reads(TableId::VariantDefinitions, at as i32))
}

/// What one country's owner reply compares at an hour with [`content`] bound, from the model:
/// its divisions' need, capped holdings and shortfall of the one equipment, and how many fall
/// short of it, of how many; none while no pool or designs read stands or it has no division.
fn compared(
    model: &Model,
    pools: Option<&Pools>,
    designs: bool,
    country: u32,
) -> Option<[i128; 5]> {
    let pools = pools.filter(|_| designs)?;
    let mut sums = None;
    for row in model
        .divisions
        .values()
        .filter(|row| row.reveal_skip == 0 && row.roster_owner == country)
    {
        let need = NEED * i128::from(lines(row.template));
        let held: i128 = pools
            .get(&row.id)
            .into_iter()
            .flatten()
            .map(|&(_, amount)| i128::from(amount))
            .sum();
        let sums = sums.get_or_insert([0; 5]);
        sums[0] += need;
        sums[1] += held.clamp(0, need);
        sums[2] += (need - held).max(0);
        sums[3] += i128::from(held < need);
        sums[4] += 1;
    }
    sums
}

/// With content bound, each division's need is compared with its pool: a pool changing by its
/// amounts places its division again by its part's difference, and the replies stay their
/// divisions' parts, which segments placing every division anew at their restarts agree with.
#[test]
fn a_division_placed_again_moves_its_parts_difference_alone() {
    let (cat, _folder) = content();
    let mut seed = 13;
    let hours = models(6, 8, 120, &mut seed);
    let whole = read(
        &published(
            project_with(&hours, &[], Some(Arc::clone(&cat))),
            &[119],
            None,
        ),
    );
    let stitched = read(
        &published(
            project_with(&hours, &[30, 60, 90], Some(cat)),
            &[119],
            None,
        ),
    );
    assert_eq!(stitched, whole);
    let (mut short, mut full) = (false, false);
    for hour in 0..hours.len() {
        for country in 1..=6 {
            let tag = format!("C{country:04}");
            let value = |code: u64, named: &str| {
                at(&whole, &format!("{tag}|0|{code}|{named}"), hour as i32)
            };
            let expect = compared(
                &hours[hour],
                pooled(&hours, hour),
                designs_read(hour),
                country,
            );
            for (at, code) in [emit::TYPE + 1, emit::TYPE + 2, emit::TYPE + 3]
                .into_iter()
                .enumerate()
            {
                assert_eq!(
                    value(code, KIND),
                    expect.map(|sums| sums[at].to_string()),
                    "{tag} measure {code} at {hour}"
                );
            }
            let shorts = expect.map_or(0, |sums| sums[3]);
            assert_eq!(
                value(emit::SHORT, ""),
                Some(shorts.to_string()),
                "{tag} short at {hour}"
            );
            if let Some(sums) = expect {
                short |= sums[3] > 0;
                full |= sums[3] < sums[4];
            }
        }
    }
    assert!(
        short && full,
        "some divisions fall short and some hold enough"
    );
}

/// A design defined anew between two designs reads is named, at every hour, as the designs
/// read standing then defines it; segments restarting inside that change and an emit saved
/// and loaded with the engine it leads with go on as the one walk.
#[test]
fn a_design_is_named_as_the_designs_read_standing_defines_it() {
    let (cat, _folder) = content();
    let mut seed = 17;
    let mut hours = models(5, 6, 110, &mut seed);
    for model in &mut hours[45..] {
        model.renamed = true;
    }
    let project = |starts: &[i32]| project_with(&hours, starts, Some(Arc::clone(&cat)));
    let publish = |starts: &[i32], cuts: &[i32], resume| {
        let items = project(starts);
        read(
            &published_with(items, cuts, resume, Some(Arc::clone(&cat))),
        )
    };
    let whole = publish(&[], &[109], None);
    assert_eq!(publish(&[50, 80], &[109], None), whole);
    assert_eq!(
        publish(&[60], &[61, 109], Some(61)),
        publish(&[60], &[109], None)
    );
    let mut names = BTreeSet::new();
    for (name, runs) in &whole {
        let Some(design) = name
            .split('|')
            .nth(2)
            .filter(|code| *code == emit::DESIGN.to_string())
            .map(|_| name.rsplit('|').next().unwrap_or_default())
        else {
            continue;
        };
        let handle: u64 = design.parse().unwrap();
        for run in runs {
            let raw: serde_json::Value = serde_json::from_str(&run.2).unwrap();
            let named = raw["display_name"].as_str().unwrap().to_owned();
            for hour in run.0..=run.1 {
                let read = designs_read_at(hour as usize).expect("a designs read stands");
                let expected = design_name(handle, hours[read].renamed);
                assert_eq!(named, expected, "{name} at {hour}");
            }
            names.insert(named);
        }
    }
    assert!(names.contains("Rifle renamed") && names.contains(&design_name(VARIANTS[0], false)));
}

/// Segments restarting at 30, 60 and 90 hours are stitched by the emit into the publication
/// one walk makes: each restart's hours before its reads are its own are stepped on the
/// engine the segment before handed on, and the rest it answers. The replies' sums are their
/// divisions' parts at every hour through pool, stockpile and designs reads, moves, new and
/// deleted divisions, and holes in a lead and in a segment's own hours.
#[test]
fn segments_stitched_by_the_emit_are_one_walk() {
    let mut seed = 5;
    let mut hours = models(6, 8, 120, &mut seed);
    hours[50].hole = true;
    hours[85].hole = true;
    let whole = read(
        &published(project(&hours, &[]), &[119], None),
    );
    let items = project(&hours, &[30, 60, 90]);
    let led = leads(&items);
    assert!((0..30).all(|hour| !led.contains(&hour)));
    for (start, end) in [(30, 59), (60, 89), (90, 119)] {
        let leading = (start..=end).take_while(|hour| led.contains(hour)).count() as i32;
        // A restart leads until its reads are its own, under two days, then answers.
        assert!(leading > 0 && leading < 48, "{start} leads {leading} hours");
        assert!(start + leading <= end, "{start} answers its own hours");
        assert!((start + leading..=end).all(|hour| !led.contains(&hour)));
    }
    let stitched = read(&published(items, &[119], None));
    assert_eq!(stitched, whole);
    for hour in [3, 29, 30, 31, 46, 49, 51, 56, 60, 76, 86, 100, 119] {
        for country in 1..=6 {
            assert_sums(&stitched, &hours, hour, country);
        }
    }
}

/// An emit saved at a cut, before, inside or after a restart's lead, with the engine the
/// lead steps, and loaded on the publication goes on as the emit that was never saved.
#[test]
fn a_cut_and_resume_go_on_as_one_publication() {
    let mut seed = 9;
    let hours = models(5, 6, 110, &mut seed);
    let whole = read(
        &published(project(&hours, &[60]), &[109], None),
    );
    for (cuts, resume) in [
        (&[59, 109][..], 59),
        (&[61, 109][..], 61),
        (&[40, 59, 80, 109][..], 80),
    ] {
        let resumed = read(
            &published(project(&hours, &[60]), cuts, Some(resume)),
        );
        assert_eq!(resumed, whole, "resumed at {resume}");
    }
    for hour in [59, 60, 61, 80, 109] {
        for country in 1..=5 {
            assert_sums(&whole, &hours, hour, country);
        }
    }
}

/// 1,500 countries of five divisions each, restarting at 20 and 41 hours: the stitched
/// publication is the one walk's, the replies' sums are their divisions', and an hour whose
/// rows did not change gives no value.
#[test]
fn fifteen_hundred_countries_stitch_as_one_walk_and_quiet_hours_give_nothing() {
    let mut seed = 21;
    let mut hours = models(1_500, 5, 64, &mut seed);
    // Hour 40 holds hour 39's rows and reads no daily table.
    hours[40] = hours[39].clone();
    let whole = read(
        &published(project(&hours, &[]), &[63], None),
    );
    let items = project(&hours, &[20, 41]);
    let quiet = items.iter().find(|item| item.hour == 40).expect("hour 40");
    let Projected::Hour(output) = &quiet.projected else {
        panic!("hour 40 is answered by its own segment");
    };
    assert!(output.changes.is_empty(), "a quiet hour gives nothing");
    let stitched = read(&published(items, &[63], None));
    assert_eq!(stitched, whole);
    for hour in [20, 40, 63] {
        for country in [1, 750, 1_500] {
            assert_sums(&stitched, &hours, hour, country);
        }
    }
}

/// A hole ends every value, which starts anew after it; a refused pool read leaves the
/// pools unread, so the holdings they sum are absent until the next read, never zero.
#[test]
fn a_hole_and_a_refused_pool_read_show_nothing_never_zero() {
    let mut seed = 3;
    let mut hours = models(3, 4, 110, &mut seed);
    hours[50].hole = true;
    // The first pool read past 60, inside the restart's lead; the next is a day later.
    let refused = (61..110)
        .find(|&hour| reads(TableId::DivisionEquipment, hour as i32))
        .expect("a pool read past 60");
    hours[refused].refused = true;
    let held = read(
        &published(project(&hours, &[60]), &[109], None),
    );
    for (name, runs) in &held {
        assert!(
            runs.iter().all(|run| !(run.0 <= 50 && 50 <= run.1)),
            "{name} holds a value in the hole"
        );
    }
    let divisions = &held[&format!("C0001|0|{}|", emit::DIVISIONS)];
    assert!(divisions.iter().any(|run| run.1 == 49));
    let after = divisions
        .iter()
        .find(|run| run.0 == 51)
        .expect("a run from 51");
    assert!(after.3, "the run after a hole starts anew");
    for country in 1..=3 {
        let holdings = format!("C{country:04}|0|{}|{UNCLASSIFIED}", emit::TYPE);
        for hour in refused..refused + 24 {
            assert_eq!(pooled(&hours, hour), None);
            assert_eq!(
                at(&held, &holdings, hour as i32),
                None,
                "{holdings} at {hour}"
            );
        }
    }
    for hour in [49, 51, refused - 1, refused, refused + 24, 109] {
        for country in 1..=3 {
            assert_sums(&held, &hours, hour, country);
        }
    }
}

/// Every field and value an hour's changes hold reads back as it was pushed, at the widest
/// tags, kinds, handles, notes, operands and numbers.
#[test]
fn encoded_changes_read_back_every_field_and_value() {
    use super::engine::{Change, Changes, Field, MEASURES, PARTS, Value};
    let fields = [
        Field::State,
        Field::Divisions,
        Field::Short,
        Field::Unknown,
        Field::Refused,
        Field::Type(u32::MAX, MEASURES[MEASURES.len() - 1]),
        Field::Variant(0, u64::MAX),
        Field::Design(300, 1 << 40),
        Field::Template(u64::MAX, PARTS[PARTS.len() - 1]),
        Field::Note(u32::MAX),
        Field::StockDesign(0),
    ];
    let values = [
        None,
        Some(Value::Number(i128::MIN)),
        Some(Value::Number(i128::MAX)),
        Some(Value::Number(-1)),
        Some(Value::Number(1 << 70)),
        Some(Value::Operand(u32::MAX)),
    ];
    let mut changes = Changes::default();
    let mut pushed = Vec::new();
    for (at, field) in fields.into_iter().enumerate() {
        for value in values {
            let change = Change {
                tag: [0, 127, 128, u32::MAX][at % 4],
                side: at as u8,
                field,
                value,
            };
            changes.push(change);
            pushed.push(change);
        }
    }
    let read: Vec<Change> = changes.iter().collect::<std::io::Result<_>>().unwrap();
    assert_eq!(read, pushed);
}
