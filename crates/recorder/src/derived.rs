//! Offline proof that playback's derivations reproduce what a recording holds,
//! province by province and hour by hour, with the recording's own content
//! stack: the day/night clocks of `WeatherProvinces` ([`replay::day_night`]),
//! its temperatures ([`replay::temperature`], [`temperature`]) and the
//! selected army's attack operands against the native ones a recording made
//! with the collector's `attack_witness` diagnostic holds ([`attack`]), and the
//! power-balance description trees against their native witnesses
//! ([`replay::power_balance_derive`], [`power_balance`]). A recording from a
//! `naval_tree_oracle` collector also checks the naval hover trees' join and
//! carry against their fresh native samples ([`naval_trees`]). Every recorded
//! value is compared; every mismatch is counted and the first ones are
//! listed with their province or record and hour.
//!
//! `--census` counts, for every table, how many rows and which fields each
//! hour's delta changes. `--sizes` writes the table again with the derived
//! fields held constant and attributes its compressed bytes as `sizes` does.
//! `--weather-factor <steps>` writes DivisionStatus again with its
//! `weather_factor` at each step and counts the attack list entries that
//! would print differently ([`weather_factor`]).

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use installed_content::ResolveOptions;
use replay::{
    container::{Replay, Section},
    day_night::{self, Clock, HourOperands, ProvinceClock},
    frame::{DecodedFrame, Frame, FrameDecoder, FrameEncoder, TABLE_COUNT, TableUpdate, TableView},
    identity::Verification,
    rows::{
        ClockState, DayNightClock, ProvinceTemperature, Recorded, TableId, TemperatureRecord,
        tables,
    },
    spool::Record,
    temperature::State,
};

mod attack;
mod naval_trees;
mod power_balance;
mod temperature;
mod weather_factor;

pub const USAGE: &str = "recorder derived <campaign.battleplan> [--game-dir <directory>] [--user-dir <directory>] [--census] [--sizes] [--weather-factor <raw step>[,<raw step>...]]";

/// Mismatches listed by name; all of them are counted.
const LISTED: usize = 40;

pub fn command(args: Vec<OsString>) -> Result<(), String> {
    let mut path = None;
    let mut game_dir = None;
    let mut user_dir = None;
    let mut census = false;
    let mut sizes = false;
    let mut weather_steps = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--game-dir") => game_dir = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
            Some("--user-dir") => user_dir = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
            Some("--census") => census = true,
            Some("--sizes") => sizes = true,
            Some("--weather-factor") => {
                weather_steps = weather_factor_steps(args.next().ok_or(USAGE)?.to_str())?;
            }
            _ if path.is_none() => path = Some(PathBuf::from(arg)),
            _ => return Err(USAGE.into()),
        }
    }
    let path = path.ok_or(USAGE)?;
    run(&path, game_dir, user_dir, census, sizes, &weather_steps).map_err(|error| error.to_string())
}

/// Comma-separated fixed5 raw steps, each a divisor of one above one
/// (`replay::display::Step::fixed5`); `default` is [`weather_factor::DEFAULT_STEPS`].
fn weather_factor_steps(list: Option<&str>) -> Result<Vec<i64>, String> {
    let list = list.ok_or(USAGE)?;
    if list == "default" {
        return Ok(weather_factor::DEFAULT_STEPS.to_vec());
    }
    list.split(',')
        .map(|step| match step.trim().parse::<i64>() {
            Ok(raw) if raw > 1 && 100_000 % raw == 0 => Ok(raw),
            _ => Err(format!(
                "weather_factor step {step:?} is not a fixed5 raw divisor of 100000 above 1"
            )),
        })
        .collect()
}

/// The recording's content facts the derivations read.
struct Content {
    defines: day_night::Defines,
    /// Day/night geometry constants by province id.
    clocks: BTreeMap<u32, ProvinceClock>,
    /// What the hourly temperature pass reads.
    temperature: Result<replay::temperature::Content, String>,
    /// Each terrain category's own adjuster attack, in database order.
    terrains: Result<Vec<i64>, String>,
    /// Province to strategic region, for the weather_factor what-if.
    regions: Result<BTreeMap<u32, u32>, String>,
    /// The verified stack, for lazily read databases (static modifiers).
    layers: installed_content::ResolvedLayers,
}

fn content(
    replay: &Replay,
    game_dir: Option<PathBuf>,
    user_dir: Option<PathBuf>,
) -> io::Result<Content> {
    let session = replay
        .sessions
        .first()
        .ok_or_else(|| io::Error::other("recording has no session"))?;
    if replay
        .sessions
        .iter()
        .any(|other| other.identity != session.identity)
    {
        return Err(io::Error::other(
            "sessions record different content stacks; check them separately",
        ));
    }
    if !matches!(session.identity.verification, Verification::Verified { .. }) {
        return Err(io::Error::other(
            "the session's content is not verified; derivations need the recorded stack",
        ));
    }
    let options = ResolveOptions {
        game_dir: match game_dir {
            Some(directory) => directory,
            None => crate::oracle::default_game_directory()?,
        },
        user_dir: match user_dir {
            Some(directory) => directory,
            None => crate::recording::resolve_user_directory().map_err(io::Error::other)?,
        },
        cache_dir: PathBuf::from(
            std::env::var_os("LOCALAPPDATA")
                .ok_or_else(|| io::Error::other("LOCALAPPDATA is not set"))?,
        )
        .join("HOI4Replay")
        .join("recorder"),
    };
    let resolved = installed_content::resolve(&session.identity, &options)?;
    let defines = installed_content::defines::read(&resolved.layers)?;
    let defines = day_night::Defines::read(
        |section, name| defines.int32(section, name),
        |section, name| defines.fixed5(section, name),
    )?;
    let geometry = installed_content::province_geometry::ProvinceGeometry::read(&resolved.layers)?;
    let map = [geometry.width, geometry.height];
    let clocks = geometry
        .centroids
        .iter()
        .map(|(&id, &centroid)| (id, ProvinceClock::new(centroid, map, &defines)))
        .collect();
    println!(
        "content: {} provinces, map {}x{}, defines {:?}",
        geometry.centroids.len(),
        map[0],
        map[1],
        defines
    );
    let temperature =
        installed_content::weather::temperature::read(&resolved.layers).map_err(|error| error.to_string());
    let terrains = installed_content::terrain::read_definitions(&resolved.layers)
        .map(|terrain| {
            terrain
                .categories
                .iter()
                .map(|category| category.common_adjuster.attack)
                .collect()
        })
        .map_err(|error| error.to_string());
    let regions = installed_content::regions::read(&resolved.layers)
        .map_err(|error| error.to_string())
        .and_then(|database| {
            let mut regions = BTreeMap::new();
            for (id, region) in database.regions {
                for province in region.provinces {
                    if regions
                        .insert(province, id)
                        .is_some_and(|previous| previous != id)
                    {
                        return Err(format!(
                            "province {province} is in more than one strategic region"
                        ));
                    }
                }
            }
            Ok(regions)
        });
    Ok(Content {
        defines,
        clocks,
        temperature,
        terrains,
        regions,
        layers: resolved.layers,
    })
}

#[derive(Default)]
struct ClockStats {
    /// Rows holding a recorded clock (held from format 102, every row in 101).
    rows: u64,
    /// Rows recording only that the native writer updated them.
    recorded_updated: u64,
    /// Updated rows whose derivation cannot run on this content.
    updated_underivable: u64,
    /// Rows the native writer updated this hour: slot > 0 and both gates proved clear.
    updated: u64,
    hour_equal: u64,
    night_equal: u64,
    mismatches: u64,
    listed: Vec<String>,
    /// Slot 0 pairs, which no update writes.
    slot_zero: BTreeMap<(i32, i64), u64>,
    /// Rows whose hour does not prove both gates clear, by recorded flags.
    held: BTreeMap<String, u64>,
    held_equal: u64,
    not_recorded: u64,
    no_content: u64,
    hours: u64,
    unavailable_hours: u64,
}

/// Rows and fields each table's deltas change.
struct Census {
    hours: u64,
    rows: Vec<u64>,
    fields: Vec<[u64; 64]>,
}

impl Census {
    fn new() -> Self {
        Self {
            hours: 0,
            rows: vec![0; TABLE_COUNT],
            fields: vec![[0; 64]; TABLE_COUNT],
        }
    }

    fn observe(&mut self, frame: &Frame) {
        self.hours += 1;
        for (index, update) in frame.tables.iter().enumerate() {
            if let TableUpdate::Delta { changed, .. } = update {
                self.rows[index] += changed.len() as u64;
                for row in changed {
                    for (bit, count) in self.fields[index].iter_mut().enumerate() {
                        *count += (row.mask >> bit) & 1;
                    }
                }
            }
        }
    }

    fn print(&self) {
        let mut tables: Vec<_> = TableId::ALL
            .into_iter()
            .filter(|table| self.rows[table.index()] != 0)
            .collect();
        tables.sort_by_key(|table| std::cmp::Reverse(self.rows[table.index()]));
        println!(
            "census: changed rows per hour and changed fields per hour (field index=count/h), {} hours",
            self.hours
        );
        for table in tables.into_iter().take(40) {
            let index = table.index();
            let fields: Vec<String> = self.fields[index]
                .iter()
                .enumerate()
                .filter(|(_, count)| **count != 0)
                .map(|(field, count)| format!("{field}={:.1}", *count as f64 / self.hours as f64))
                .collect();
            println!(
                "  {table:?} rows/h={:.1} {}",
                self.rows[index] as f64 / self.hours as f64,
                fields.join(" ")
            );
        }
    }
}

/// The weather simulation's gates as the frame recorded them.
fn gates(frame: &DecodedFrame) -> Result<[u8; 2], String> {
    match &*frame.tables[TableId::SimulationFeatures.index()] {
        TableView::Complete(rows) => match rows
            .rows::<tables::SimulationFeatures>()
            .and_then(|rows| rows.first())
            .map(|row| &row.weather_suspend)
        {
            Some(Recorded::Value(Some(flags))) => Ok(*flags),
            Some(Recorded::Value(None)) => Err("suspension read refused".into()),
            Some(Recorded::NotRecorded) => Err("suspension not recorded".into()),
            None => Err("simulation features row absent".into()),
        },
        _ => Err("simulation features unavailable".into()),
    }
}

/// The clock the native writer left in `province` at this frame, when it ran.
fn derived_clock(
    content: &Content,
    operands: Option<&HourOperands>,
    gates: &Result<[u8; 2], String>,
    province: u32,
) -> Option<Clock> {
    if province == 0 || !matches!(gates, Ok([0, 0])) {
        return None;
    }
    Some(content.clocks.get(&province)?.update(operands?))
}

fn check_clocks(frame: &DecodedFrame, content: &Content, stats: &mut ClockStats) {
    stats.hours += 1;
    let TableView::Complete(rows) = &*frame.tables[TableId::WeatherProvinces.index()] else {
        stats.unavailable_hours += 1;
        return;
    };
    let Some(rows) = rows.rows::<tables::WeatherProvinces>() else {
        stats.unavailable_hours += 1;
        return;
    };
    let gates = gates(frame);
    let operands = HourOperands::new(frame.hour, &content.defines).ok();
    for row in rows {
        let recorded = match row.clock.state(row.id) {
            Ok(ClockState::NotRecorded) | Err(_) => {
                stats.not_recorded += 1;
                continue;
            }
            Ok(ClockState::Updated) => {
                stats.recorded_updated += 1;
                if content.clocks.get(&row.id).zip(operands.as_ref()).is_none() {
                    stats.updated_underivable += 1;
                }
                continue;
            }
            Ok(ClockState::Held(clock)) => clock,
        };
        let (hour, night) = (recorded.local_hour, recorded.night_factor);
        stats.rows += 1;
        if row.id == 0 {
            *stats.slot_zero.entry((hour, night)).or_default() += 1;
            continue;
        }
        if !matches!(gates, Ok([0, 0])) {
            let key = match &gates {
                Ok(flags) => format!("flags {flags:?}"),
                Err(reason) => reason.clone(),
            };
            *stats.held.entry(key).or_default() += 1;
            let unconditional = content
                .clocks
                .get(&row.id)
                .zip(operands.as_ref())
                .map(|(clock, operands)| clock.update(operands));
            if unconditional == Some(recorded) {
                stats.held_equal += 1;
            }
            continue;
        }
        let Some(derived) = derived_clock(content, operands.as_ref(), &gates, row.id) else {
            stats.no_content += 1;
            continue;
        };
        stats.updated += 1;
        stats.hour_equal += u64::from(derived.local_hour == hour);
        stats.night_equal += u64::from(derived.night_factor == night);
        if derived != recorded {
            stats.mismatches += 1;
            if stats.listed.len() < LISTED {
                stats.listed.push(format!(
                    "branch {} hour {} province {}: recorded {:?}, derived {:?}",
                    frame.branch, frame.hour, row.id, recorded, derived
                ));
            }
        }
    }
}

/// One way of writing the table again with the current writer.
struct Variant {
    name: &'static str,
    encoder: FrameEncoder,
    segments: Vec<Vec<Record>>,
    /// Write a held clock the gates prove updated as updated, as the current
    /// collector does (format-101 recordings held every clock).
    derive: bool,
    /// Also hold temperature constant, to attribute its bytes.
    temperature: bool,
    /// Estimate an anchored temperature layout on a recording that holds every hour's.
    anchored: Option<Anchored>,
}

/// What slot 0's display field carries in an anchored estimate.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// The recorded boundary.
    Recorded,
    /// A stand-in for the temperature pass: the anchor hour, a random checksum and five random
    /// bytes per hour since the anchor, the size and entropy of an hour's seed.
    Synthetic,
    /// None, to subtract the stand-in's own cost.
    Dropped,
}

/// Rows hold each province's temperature at the anchor (each original keyframe) and a drift
/// stand-in: the province's last change.
struct Anchored {
    pass: Pass,
    branch: u64,
    anchor_hour: i32,
    anchor: Vec<i64>,
    drift: Vec<i64>,
    previous: Vec<i64>,
    last_change: Vec<i64>,
    hours: Vec<u8>,
    random: u64,
}

impl Anchored {
    fn new(pass: Pass) -> Self {
        Self {
            pass,
            branch: u64::MAX,
            anchor_hour: 0,
            anchor: Vec::new(),
            drift: Vec::new(),
            previous: Vec::new(),
            last_change: Vec::new(),
            hours: Vec::new(),
            random: 0x9e37_79b9_7f4a_7c15,
        }
    }

    /// splitmix64.
    fn next(&mut self) -> u64 {
        self.random = self.random.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.random;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn apply(
        &mut self,
        branch: u64,
        hour: i32,
        keyframe: bool,
        rows: &mut [replay::rows::WeatherProvinceRow],
    ) {
        let temperatures: Vec<i64> = rows
            .iter()
            .map(|row| match row.temperature.record() {
                Ok(TemperatureRecord::AtHour(value)) => value,
                Ok(TemperatureRecord::Anchored(state)) => state.temperature,
                _ => 0,
            })
            .collect();
        if self.previous.len() == temperatures.len() {
            for (slot, (&now, &before)) in temperatures.iter().zip(&self.previous).enumerate() {
                if now != before {
                    self.last_change[slot] = now.wrapping_sub(before);
                }
            }
        } else {
            self.last_change = vec![0; temperatures.len()];
        }
        if keyframe || branch != self.branch || self.anchor.len() != temperatures.len() {
            self.anchor = temperatures.clone();
            self.drift = self.last_change.clone();
            self.hours.clear();
            self.anchor_hour = hour;
            self.branch = branch;
        } else {
            self.hours.push(1);
            let seed = self.next() as u32;
            self.hours.extend_from_slice(&seed.to_le_bytes());
        }
        self.previous = temperatures;
        let checksum = self.next();
        for (slot, row) in rows.iter_mut().enumerate() {
            row.temperature = ProvinceTemperature::anchored(State {
                temperature: self.anchor[slot],
                drift: self.drift[slot],
                staged: self.anchor[slot],
                working: self.drift[slot],
            });
            if row.id == 0 {
                match self.pass {
                    Pass::Recorded => {}
                    Pass::Synthetic => {
                        row.display = Recorded::Value(Some(replay::rows::WeatherDisplay {
                            width: self.anchor_hour as u32,
                            height: 0,
                            refresh_interval: 0,
                            refresh_identity: checksum,
                            edges: self.hours.clone(),
                        }))
                    }
                    Pass::Dropped => row.display = Recorded::Value(None),
                }
            }
        }
    }
}

impl Variant {
    fn new(name: &'static str, derive: bool, temperature: bool) -> Self {
        Self {
            name,
            encoder: FrameEncoder::new(),
            segments: Vec::new(),
            derive,
            temperature,
            anchored: None,
        }
    }

    fn anchored(name: &'static str, pass: Pass) -> Self {
        Self {
            anchored: Some(Anchored::new(pass)),
            ..Self::new(name, true, false)
        }
    }

    fn push(
        &mut self,
        mut frame: DecodedFrame,
        keyframe: bool,
        content: &Content,
    ) -> io::Result<()> {
        let gates = gates(&frame);
        let operands = HourOperands::new(frame.hour, &content.defines).ok();
        if let TableView::Complete(rows) = &mut *frame.tables[TableId::WeatherProvinces.index()]
            && let Some(rows) = rows.rows_mut::<tables::WeatherProvinces>()
        {
            for row in rows.iter_mut() {
                // The current writer stores anchored words: an hourly temperature is written
                // as an anchor at its own hour, so each change costs what it cost.
                if let Ok(TemperatureRecord::AtHour(temperature)) = row.temperature.record() {
                    row.temperature = ProvinceTemperature::anchored(State {
                        temperature,
                        staged: temperature,
                        ..State::default()
                    });
                }
                if row.temperature_pass == Recorded::NotRecorded {
                    row.temperature_pass = Recorded::Value(None);
                }
                if let Ok(ClockState::Held(clock)) = row.clock.state(row.id) {
                    // Format 101 recorded every clock without a source.
                    row.clock = DayNightClock::held(clock);
                    if self.derive
                        && derived_clock(content, operands.as_ref(), &gates, row.id) == Some(clock)
                    {
                        row.clock = DayNightClock::UPDATED;
                    }
                }
                if self.temperature {
                    row.temperature = ProvinceTemperature::anchored(State::default());
                    row.temperature_pass = Recorded::Value(None);
                }
            }
            if let Some(anchored) = &mut self.anchored {
                anchored.apply(frame.branch, frame.hour, keyframe, rows);
            }
        }
        let encoded = self.encoder.encode(frame, keyframe)?;
        self.segments
            .last_mut()
            .ok_or_else(|| io::Error::other("frame before its session"))?
            .push(Record::Frame(encoded));
        Ok(())
    }

    /// Compressed bytes of `table`'s sections, written as one recording.
    fn measure(&self, campaign: &[u8], table: TableId) -> io::Result<u64> {
        let replay = Replay::from_records(campaign.to_vec(), &self.segments)?;
        let mut bytes = 0;
        for block in &replay.index {
            for section in replay
                .block_sections(block)?
                .ok_or_else(|| io::Error::other("written block is not grouped"))?
            {
                if let Section::Keyframe(found) | Section::Later(found) = section.section
                    && found == table
                {
                    bytes += section.compressed;
                }
            }
        }
        Ok(bytes)
    }
}

fn run(
    path: &Path,
    game_dir: Option<PathBuf>,
    user_dir: Option<PathBuf>,
    census: bool,
    sizes: bool,
    weather_steps: &[i64],
) -> io::Result<()> {
    let replay = Replay::open(fs::read(path)?)?;
    println!(
        "recording: format {}, {} blocks",
        replay.format.version(),
        replay.index.len()
    );
    for gap in replay.compatibility_gaps() {
        println!("compatibility gap: {gap}");
    }
    let content = content(&replay, game_dir, user_dir)?;
    let selected = [
        TableId::WeatherProvinces,
        TableId::SimulationFeatures,
        TableId::DivisionStatus,
        TableId::PowerBalanceModifierBreakdowns,
        TableId::ModifierDefinitions,
        TableId::LexicalTokens,
        TableId::NavalCombatDetails,
        TableId::NavalCombatShips,
        TableId::NavalModifierTrees,
        TableId::NavalModifierTreeSamples,
        // The weather_factor what-if's region weather entries.
        TableId::Divisions,
        TableId::WeatherRegions,
    ];
    let mut clocks = ClockStats::default();
    let mut attack = attack::Checker::new(content.terrains.clone());
    let mut naval = naval_trees::Checker::default();
    let mut temperatures = temperature::Checker::new(content.temperature.clone());
    let mut balances = power_balance::Checker::default();
    let mut counts = Census::new();
    let mut variants = if sizes {
        vec![
            Variant::new("as recorded", false, false),
            Variant::new("derived", true, false),
            Variant::new("derived, temperature constant", true, true),
            Variant::anchored("derived, temperature anchored at keyframes", Pass::Recorded),
            Variant::anchored(
                "derived, temperature anchored, pass stand-in",
                Pass::Synthetic,
            ),
            Variant::anchored("derived, temperature anchored, no display", Pass::Dropped),
        ]
    } else {
        Vec::new()
    };
    let mut factors: Vec<weather_factor::Candidate> = if weather_steps.is_empty() {
        Vec::new()
    } else {
        std::iter::once(None)
            .chain(weather_steps.iter().copied().map(Some))
            .map(weather_factor::Candidate::new)
            .collect()
    };
    let regions = weather_factor::Regions(content.regions.clone());
    let mut hours = 0_u64;
    let (mut session, mut branch) = (None, None);
    for block in &replay.index {
        let start = replay
            .branches
            .iter()
            .find(|start| start.id == block.branch)
            .ok_or_else(|| io::Error::other("block refers to an unknown branch"))?;
        for variant in variants
            .iter_mut()
            .chain(factors.iter_mut().map(|candidate| &mut candidate.variant))
        {
            if session != Some(start.session) {
                let started = replay
                    .sessions
                    .iter()
                    .find(|candidate| candidate.id == start.session)
                    .ok_or_else(|| io::Error::other("branch refers to an unknown session"))?;
                variant
                    .segments
                    .push(vec![Record::SessionStart(started.clone())]);
            }
            if branch != Some(block.branch) {
                variant
                    .segments
                    .last_mut()
                    .unwrap()
                    .push(Record::BranchStart(start.clone()));
            }
        }
        session = Some(start.session);
        branch = Some(block.branch);
        let frames = if census {
            replay.read_block(block)?
        } else {
            replay.read_block_selected(block, &selected)?
        };
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in &frames {
            hours += 1;
            if census {
                counts.observe(frame);
            }
            let decoded = decoder.advance(frame)?;
            check_clocks(decoded, &content, &mut clocks);
            temperatures.observe(decoded, &gates(decoded));
            attack.observe(decoded);
            balances.observe(decoded, &content.layers);
            naval.observe(decoded);
            for variant in &mut variants {
                variant.push(decoded.clone(), frame.keyframe, &content)?;
            }
            for candidate in &mut factors {
                candidate.push(decoded.clone(), frame.keyframe, &content, &regions)?;
            }
        }
    }
    println!("hours: {hours}");
    println!(
        "WeatherProvinces clocks: {} hours ({} without a complete table), {} recorded clocks, {} updated rows ({} not derivable on this content), {} not recorded",
        clocks.hours,
        clocks.unavailable_hours,
        clocks.rows,
        clocks.recorded_updated,
        clocks.updated_underivable,
        clocks.not_recorded
    );
    println!(
        "  updated rows (slot > 0, gates proved clear): {}; local_hour equal {}, night_factor equal {}, rows differing {}; no content {}",
        clocks.updated, clocks.hour_equal, clocks.night_equal, clocks.mismatches, clocks.no_content
    );
    for (pair, count) in &clocks.slot_zero {
        println!("  slot 0 holds {pair:?} in {count} hours");
    }
    for (reason, count) in &clocks.held {
        println!("  rows in hours not proving the update ({reason}): {count}");
    }
    if !clocks.held.is_empty() {
        println!(
            "    of which equal to an unconditional derivation: {}",
            clocks.held_equal
        );
    }
    for line in &clocks.listed {
        println!("  mismatch: {line}");
    }
    temperatures.print();
    attack.print();
    balances.print();
    naval.print();
    if census {
        counts.print();
    }
    for variant in &variants {
        let weather = variant.measure(&replay.campaign_id, TableId::WeatherProvinces)?;
        println!(
            "sizes ({}): WeatherProvinces {:.1} B/h",
            variant.name,
            weather as f64 / hours as f64
        );
    }
    for candidate in &factors {
        candidate.print(&replay.campaign_id, hours)?;
    }
    Ok(())
}
