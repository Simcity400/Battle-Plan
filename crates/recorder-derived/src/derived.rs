//! Offline proof that playback's derivations reproduce what a recording holds,
//! province by province and hour by hour, with the recording's own content
//! stack: the day/night clocks of `WeatherProvinces` ([`replay_data::day_night`]),
//! its temperatures ([`replay_data::temperature`], [`temperature`]) and the
//! selected army's attack operands against the native ones a recording made
//! with the collector's `attack_witness` diagnostic holds ([`attack`]), and the
//! power-balance description trees against their native witnesses
//! ([`replay_derive::power_balance_derive`], [`power_balance`]). A recording from a
//! `naval_tree_oracle` collector also checks the naval hover trees' join and
//! carry against their fresh native samples ([`naval_trees`]). Every division
//! that keeps its edge across two consecutive hours also compares playback's
//! movement speed with the game's own, the next hour's progress numerator step
//! ([`movement`]); these cross-hour categories are diagnostic because operands
//! change inside the hourly update. Every recorded value is compared; every mismatch
//! is counted and the first ones are listed with their province or record and hour.
//!
//! `--census` counts, for every table, how many rows and which fields each
//! hour's delta changes. Differences and incomplete comparisons outside the
//! diagnostic cross-hour progress comparison fail the command.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

use installed_content_core::ResolveOptions;
use replay::campaign::Replay;
use replay_data::rows::TABLE_COUNT;
use replay_data::{
    day_night, day_night::Clock, day_night::HourOperands, day_night::ProvinceClock,
    identity::Identity, identity::SessionStart, identity::Verification, rows::ClockState,
    rows::Recorded, rows::TableId, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::TableUpdate, frame::TableView};

mod attack;
mod movement;
mod naval_trees;
mod power_balance;
mod temperature;

pub const USAGE: &str = "recorder derived <campaign.battleplan> [--game-dir <directory>] [--user-dir <directory>] [--census]";

/// Mismatches listed by name; all of them are counted.
const LISTED: usize = 40;

pub fn command(args: Vec<OsString>) -> Result<(), String> {
    let (path, game_dir, user_dir, census) = arguments(args)?;
    run(&path, game_dir, user_dir, census).map_err(|error| error.to_string())
}

type Arguments = (PathBuf, Option<PathBuf>, Option<PathBuf>, bool);

fn arguments(args: Vec<OsString>) -> Result<Arguments, String> {
    let mut path = None;
    let mut game_dir = None;
    let mut user_dir = None;
    let mut census = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--game-dir") => game_dir = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
            Some("--user-dir") => user_dir = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
            Some("--census") => census = true,
            Some(flag) if flag.starts_with("--") => return Err(USAGE.into()),
            _ if path.is_none() => path = Some(PathBuf::from(arg)),
            _ => return Err(USAGE.into()),
        }
    }
    Ok((path.ok_or(USAGE)?, game_dir, user_dir, census))
}

/// The recording's content facts the derivations read.
struct Content {
    defines: day_night::Defines,
    /// Day/night geometry constants by province id.
    clocks: BTreeMap<u32, ProvinceClock>,
    /// What the hourly temperature pass reads.
    temperature: Result<replay_data::temperature::Content, String>,
    /// Each terrain category's own adjuster attack, in database order.
    terrains: Result<Vec<i64>, String>,
    /// The verified stack, for lazily read databases (static modifiers).
    layers: installed_content_core::ResolvedLayers,
}

/// The content stack every session recorded, each session verified to have loaded it.
fn recorded_stack(sessions: &[SessionStart]) -> io::Result<&Identity> {
    let first = sessions
        .first()
        .ok_or_else(|| io::Error::other("recording has no session"))?;
    if sessions
        .iter()
        .any(|session| !session.identity.same_content(&first.identity))
    {
        return Err(io::Error::other(
            "sessions record different content stacks; check them separately",
        ));
    }
    if let Some(session) = sessions.iter().find(|session| {
        !matches!(session.identity.verification, Verification::Verified { .. })
    }) {
        return Err(io::Error::other(format!(
            "session {}'s content is not verified; derivations need the recorded stack",
            session.id
        )));
    }
    Ok(&first.identity)
}

fn content(
    replay: &Replay,
    game_dir: Option<PathBuf>,
    user_dir: Option<PathBuf>,
) -> io::Result<Content> {
    let identity = recorded_stack(&replay.sessions)?;
    let options = ResolveOptions {
        game_dir: match game_dir {
            Some(directory) => directory,
            None => crate::directories::default_game_directory()?,
        },
        user_dir: match user_dir {
            Some(directory) => directory,
            None => crate::directories::resolve_user_directory().map_err(io::Error::other)?,
        },
        cache_dir: PathBuf::from(
            std::env::var_os("LOCALAPPDATA")
                .ok_or_else(|| io::Error::other("LOCALAPPDATA is not set"))?,
        )
        .join("HOI4Replay")
        .join("recorder"),
    };
    // The movement speed's modifier keys are checked against the installation's
    // executable metadata, as the save comparison checks them.
    engine_executable::executable_tables::initialize(&options.game_dir)?;
    let resolved = installed_content_core::resolve(identity, &options)?.exact()?;
    let defines = installed_content_core::defines::read(&resolved.layers)?;
    let defines = day_night::Defines::read(
        |section, name| defines.int32(section, name),
        |section, name| defines.fixed5(section, name),
    )?;
    let geometry = installed_content_core::province_geometry::ProvinceGeometry::read(&resolved.layers)?;
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
    let temperature = installed_content_weather::weather::temperature::read(&resolved.layers)
        .map_err(|error| error.to_string());
    let terrains = installed_content_core::terrain::read_definitions(&resolved.layers)
        .map(|terrain| {
            terrain
                .categories
                .iter()
                .map(|category| category.common_adjuster.attack)
                .collect()
        })
        .map_err(|error| error.to_string());
    Ok(Content {
        defines,
        clocks,
        temperature,
        terrains,
        layers: resolved.layers,
    })
}

#[derive(Default)]
struct ClockStats {
    /// Rows holding a recorded clock.
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
    held_unproven: u64,
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

    fn observe(&mut self, frame: &replay_frame::frame::RecordedFrame) {
        self.hours += 1;
        for index in 0..TABLE_COUNT {
            if let TableUpdate::Delta { changed, .. } = &frame.tables[index] {
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
            stats.held_unproven += u64::from(gates.is_err());
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

fn run(
    path: &Path,
    game_dir: Option<PathBuf>,
    user_dir: Option<PathBuf>,
    census: bool,
) -> io::Result<()> {
    // Blocks are read as they are checked, never the whole recording at once.
    let replay = Replay::open_path(path)?;
    println!(
        "recording: format {}, {} blocks",
        replay.format.version(),
        replay.index.len()
    );
    let content = content(&replay, game_dir, user_dir)?;
    let mut selected = vec![
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
    ];
    for table in movement::tables() {
        if !selected.contains(&table) {
            selected.push(table);
        }
    }
    let mut clocks = ClockStats::default();
    let mut attack = attack::Checker::new(content.terrains.clone());
    let mut naval = naval_trees::Checker::default();
    let mut temperatures = temperature::Checker::new(content.temperature.clone());
    let mut balances = power_balance::Checker::default();
    let mut speeds = movement::Checker::new(
        movement::Content::read(&content.layers).map_err(|error| error.to_string()),
    );
    let mut counts = Census::new();
    let mut hours = 0_u64;
    let blocks: Vec<_> = replay.index.iter().collect();
    let read = if census { &TableId::ALL[..] } else { &selected[..] };
    replay.visit_blocks(
        &blocks,
        read,
        None,
        |_, recorded, decoded| {
            hours += 1;
            if census {
                counts.observe(recorded);
            }
            check_clocks(decoded, &content, &mut clocks);
            temperatures.observe(decoded);
            attack.observe(decoded);
            balances.observe(decoded, &content.layers);
            naval.observe(decoded);
            speeds.observe(decoded);
            Ok(true)
        },
        || Ok(()),
    )?;
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
    speeds.print();
    if census {
        counts.print();
    }
    let [mismatches, incomplete] = [
        [
            clocks.mismatches,
            clocks.unavailable_hours
                + clocks.not_recorded
                + clocks.updated_underivable
                + clocks.no_content
                + clocks.held_unproven
                + u64::from(hours == 0),
        ],
        temperatures.failures(),
        attack.failures(),
        balances.failures(),
        naval.failures(),
        speeds.failures(),
    ]
    .into_iter()
    .fold([0, 0], |[differences, refused], [next, missing]| {
        [differences + next, refused + missing]
    });
    if mismatches == 0 && incomplete == 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "derivation comparison failed: {mismatches} mismatches; {incomplete} incomplete comparisons"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_retired_movement_witness_flag_is_refused_and_not_read_as_a_path() {
        for list in [
            &["campaign.battleplan", "--movement-witness"][..],
            &["--movement-witness", "campaign.battleplan"],
        ] {
            assert_eq!(arguments(args(list)).unwrap_err(), USAGE);
        }
        let (path, game_dir, user_dir, census) = arguments(args(&[
            "campaign.battleplan",
            "--census",
            "--game-dir",
            "game",
        ]))
        .unwrap();
        assert_eq!(path, PathBuf::from("campaign.battleplan"));
        assert_eq!(game_dir, Some(PathBuf::from("game")));
        assert_eq!(user_dir, None);
        assert!(census);
        assert_eq!(arguments(args(&["--census"])).unwrap_err(), USAGE);
    }

    #[test]
    fn sessions_continued_by_new_tools_share_the_stack_and_each_must_be_verified() {
        use replay_data::identity::{CaptureMode, SampleBoundary};
        let verified = || {
            let mut identity = replay_identity::test_fixtures::identity();
            let roster = identity.roster();
            identity.verification = Verification::Verified {
                launcher: roster.clone(),
                memory: roster,
            };
            identity
        };
        let session = |id, identity| SessionStart {
            id,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 1,
            sample_boundary: SampleBoundary::EndOfHour,
            capture_mode: CaptureMode::Full,
            identity,
        };
        // The campaign continued with another recorder and collector build.
        let mut rebuilt = verified();
        rebuilt.recorder_version = "next recorder".into();
        rebuilt.collector_hash = [99; 32];
        let sessions = [session(1, verified()), session(2, rebuilt)];
        assert_eq!(recorded_stack(&sessions).unwrap(), &sessions[0].identity);
        for state in [
            Verification::Unchecked,
            Verification::Unverified {
                reason: "rosters differ".into(),
                launcher: Default::default(),
                memory: Default::default(),
            },
        ] {
            let mut unverified = sessions.clone();
            unverified[1].identity.verification = state;
            let error = recorded_stack(&unverified).unwrap_err().to_string();
            assert!(error.contains("session 2's content is not verified"), "{error}");
        }
        let mut other = sessions.clone();
        other[1].identity.language = "l_french".into();
        let error = recorded_stack(&other).unwrap_err().to_string();
        assert!(error.contains("different content stacks"), "{error}");
        assert!(recorded_stack(&[]).is_err());
    }
}
