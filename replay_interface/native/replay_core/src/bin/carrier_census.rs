//! Carrier census: what a recording's hours actually CARRY, read from the
//! recording and nothing else.
//!
//! `recording_reach` (in `replay_app`) answers the consumer half of the seam —
//! which map-authority inputs the replay admits, and which coverage warnings a
//! frame raises. It cannot answer the producer half, because a warning is only
//! ever raised for a carrier a consumer already looks for: a carrier the file
//! carries that NOTHING reads is silent in that report, and so is a carrier
//! whose declared schema version drifted past the one the consumer contracts
//! for.
//!
//! This binary reads [`RawFrame::rest`] — the extractor's folded hour snapshot
//! exactly as the container stores it — and reports, per carrier:
//!
//! * the DISTINCT HOURS it appears in and is present-but-null, alongside the
//!   OBSERVATION counts (at country grain a carrier is looked at once per
//!   country, so the two differ by up to the country count),
//! * the row count it carries (array length, or the member count of a keyed
//!   object), summed and per hour,
//! * every distinct `schema_version` and `authority` spelling it declares.
//!
//! Two grains, because the snapshot has two: WORLD carriers are the top-level
//! keys, and COUNTRY carriers are the keys inside each `countries.<key>` object
//! (and its `extra` sub-object, which is where the `_r2_*` carriers live).
//! Country grain is aggregated across countries — a carrier is a carrier
//! whichever country carries it — with the country count kept so a row count
//! can be read per country.
//!
//! At world grain a top-level key is sometimes the carrier (`_r2_weather_live`)
//! and sometimes a SECTION holding several (`map.rail_replay_state_live`,
//! `world_events.sunk_convoys`). Both are reported: a section is tallied for
//! its own member count and each of its members is tallied as
//! `<section>.<member>`. A section is anything object-shaped whose keys are not
//! predominantly numeric — the numeric-keyed objects (`countries`, `provinces`,
//! `states`) are keyed COLLECTIONS whose member count is a row count, and
//! splitting one would emit 13,000 phantom carriers named after province ids.
//!
//! The census never interprets a carrier: it does not know which are required,
//! and it takes no view on whether a version is current. Crossing it with the
//! consumer side is the caller's job, and keeping that join outside this binary
//! is what lets the census stay true when the consumer contract changes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::ExitCode;

use replay_core::{open_recording, open_recording_game, RawFrame};
use serde_json::{json, Map, Value};

const USAGE: &str = "usage: carrier_census <recording-path> --out <census.json> \
                     [--hours all|sample|<i64>[,<i64>...]] [--game ID] \
                     [--dump-country <key> --dump-out <rows.json>]";

const CENSUS_SCHEMA: &str = "hoi4.replay.carrier_census.v1";

/// Cap on the distinct spellings kept for one declaration field. A carrier that
/// declares a per-hour-varying `authority` is a finding, not a list to print in
/// full, and an unbounded set would let one such carrier dominate the report.
const MAX_DISTINCT_DECLARATIONS: usize = 16;

/// One carrier's tally.
///
/// HOURS AND OBSERVATIONS ARE DIFFERENT NUMBERS at country grain and the same
/// number at world grain, so both are kept. A world carrier is looked at once
/// per hour; a country carrier is looked at once per (hour, country), so a
/// 250-hour recording with 440 countries yields up to 110,000 observations of
/// it. Counting those as "hours" published `capitulation.hours_present =
/// 109,311` for a 250-hour recording — a number that cannot be an hour count,
/// and that made every country-grain hour column unusable.
#[derive(Default)]
struct CarrierTally {
    /// Distinct hours in which the carrier was present and non-null somewhere.
    present_hours: BTreeSet<i64>,
    /// Distinct hours in which it was present and null somewhere. An hour can
    /// be in BOTH at country grain: one country's copy null, another's not.
    null_hours: BTreeSet<i64>,
    /// Distinct hours in which it was present, non-null and carried no rows.
    empty_hours: BTreeSet<i64>,
    observations_present: u64,
    observations_null: u64,
    observations_empty: u64,
    rows: u64,
    /// Country grain only: how many (hour, country) pairs the key appeared in.
    country_hours: u64,
    min_rows: Option<u64>,
    max_rows: Option<u64>,
    shapes: BTreeSet<&'static str>,
    schema_versions: BTreeSet<String>,
    authorities: BTreeSet<String>,
    overflowed_declarations: bool,
}

impl CarrierTally {
    fn record(&mut self, hour: i64, value: &Value) {
        if value.is_null() {
            self.null_hours.insert(hour);
            self.observations_null += 1;
            return;
        }
        self.present_hours.insert(hour);
        self.observations_present += 1;
        self.shapes.insert(shape_name(value));
        let rows = row_count(value);
        if rows == 0 {
            self.empty_hours.insert(hour);
            self.observations_empty += 1;
        }
        self.rows += rows;
        self.min_rows = Some(self.min_rows.map_or(rows, |seen| seen.min(rows)));
        self.max_rows = Some(self.max_rows.map_or(rows, |seen| seen.max(rows)));
        if let Some(object) = value.as_object() {
            self.note(object.get("schema_version"), true);
            self.note(object.get("authority"), false);
        }
    }

    fn note(&mut self, value: Option<&Value>, is_version: bool) {
        let Some(value) = value else { return };
        let spelling = match value {
            Value::String(text) => text.clone(),
            Value::Null => "null".to_string(),
            other => other.to_string(),
        };
        let set = if is_version {
            &mut self.schema_versions
        } else {
            &mut self.authorities
        };
        if set.contains(&spelling) {
            return;
        }
        if set.len() >= MAX_DISTINCT_DECLARATIONS {
            self.overflowed_declarations = true;
            return;
        }
        set.insert(spelling);
    }

    fn to_json(&self, country_grain: bool) -> Value {
        let mut object = Map::new();
        object.insert("hours_present".into(), json!(self.present_hours.len()));
        object.insert("hours_null".into(), json!(self.null_hours.len()));
        object.insert("hours_empty".into(), json!(self.empty_hours.len()));
        object.insert(
            "observations_present".into(),
            json!(self.observations_present),
        );
        object.insert("observations_null".into(), json!(self.observations_null));
        object.insert("observations_empty".into(), json!(self.observations_empty));
        object.insert("rows".into(), json!(self.rows));
        object.insert("min_rows".into(), json!(self.min_rows));
        object.insert("max_rows".into(), json!(self.max_rows));
        if country_grain {
            object.insert("country_hours".into(), json!(self.country_hours));
        }
        object.insert(
            "shapes".into(),
            json!(self.shapes.iter().copied().collect::<Vec<_>>()),
        );
        object.insert(
            "schema_versions".into(),
            json!(self.schema_versions.iter().collect::<Vec<_>>()),
        );
        object.insert(
            "authorities".into(),
            json!(self.authorities.iter().collect::<Vec<_>>()),
        );
        if self.overflowed_declarations {
            object.insert("declarations_truncated".into(), json!(true));
        }
        Value::Object(object)
    }
}

fn shape_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// A carrier's row count: array length, object member count, or 1 for a scalar.
///
/// An object's member count is the right row count for the keyed-collection
/// carriers (`countries`, `provinces`, `states`) and is a serviceable field
/// count for the document-shaped ones; the `shapes` set is what tells the two
/// apart when a reader needs to know which they are looking at.
/// The members of a top-level SECTION, or `None` when the value is not one.
///
/// A section is object-shaped and named: its keys are field names a consumer
/// reads by name. A keyed COLLECTION is object-shaped and keyed by engine
/// identity — `provinces` by province id, `naval_combats` by `<combat>:<gen>` —
/// and its members are rows of one carrier rather than carriers of their own.
///
/// The test is on the KEY SPELLING, not on how many keys parse as integers: an
/// identity key is whatever the engine minted, and only a key that is a
/// Rust/JSON field identifier can be one a consumer reads by name. The split is
/// by majority so a collection carrying a handful of named sidecar fields
/// beside its id keys is still read as a collection.
fn section_members(value: &Value) -> Option<&Map<String, Value>> {
    let members = value.as_object()?;
    if members.is_empty() {
        return None;
    }
    let named = members.keys().filter(|key| is_field_identifier(key)).count();
    (named * 2 > members.len()).then_some(members)
}

fn is_field_identifier(key: &str) -> bool {
    let mut characters = key.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn row_count(value: &Value) -> u64 {
    match value {
        Value::Null => 0,
        Value::Array(rows) => rows.len() as u64,
        Value::Object(members) => members.len() as u64,
        _ => 1,
    }
}

struct Args {
    path: PathBuf,
    out: PathBuf,
    game: Option<String>,
    hours: HourSelection,
    /// One `countries` key to write out verbatim, per counted hour.
    ///
    /// The census answers "how much" for every carrier; this answers "what
    /// exactly" for one country, which is what a numeric key nobody can name
    /// needs — the census can only say that key `367` exists, and the question
    /// that key raises is which domains put it there.
    dump_country: Option<String>,
    dump_out: Option<PathBuf>,
}

enum HourSelection {
    All,
    Sample,
    Explicit(Vec<i64>),
}

fn parse_args() -> Result<Args, String> {
    let mut path: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut game: Option<String> = None;
    let mut hours = HourSelection::All;
    let mut dump_country: Option<String> = None;
    let mut dump_out: Option<PathBuf> = None;
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--out" => {
                out = Some(PathBuf::from(argv.next().ok_or_else(|| {
                    format!("--out needs a path\n{USAGE}")
                })?));
            }
            "--game" => {
                game = Some(
                    argv.next()
                        .ok_or_else(|| format!("--game needs a value\n{USAGE}"))?,
                );
            }
            "--hours" => {
                let value = argv
                    .next()
                    .ok_or_else(|| format!("--hours needs a value\n{USAGE}"))?;
                hours = match value.as_str() {
                    "all" => HourSelection::All,
                    "sample" => HourSelection::Sample,
                    list => {
                        let mut picked = Vec::new();
                        for token in list.split(',') {
                            picked.push(token.trim().parse::<i64>().map_err(|error| {
                                format!("--hours {token:?} is not an hour index: {error}")
                            })?);
                        }
                        if picked.is_empty() {
                            return Err(format!("--hours listed no hour\n{USAGE}"));
                        }
                        HourSelection::Explicit(picked)
                    }
                };
            }
            "--dump-country" => {
                dump_country = Some(
                    argv.next()
                        .ok_or_else(|| format!("--dump-country needs a key\n{USAGE}"))?,
                );
            }
            "--dump-out" => {
                dump_out = Some(PathBuf::from(argv.next().ok_or_else(|| {
                    format!("--dump-out needs a path\n{USAGE}")
                })?));
            }
            "--help" | "-h" => return Err(USAGE.to_string()),
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {other:?}\n{USAGE}"))
            }
            other => {
                if path.is_some() {
                    return Err(format!("unexpected extra argument {other:?}\n{USAGE}"));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }
    if dump_country.is_some() != dump_out.is_some() {
        return Err(format!(
            "--dump-country and --dump-out are used together or not at all\n{USAGE}"
        ));
    }
    Ok(Args {
        path: path.ok_or_else(|| format!("missing <recording-path>\n{USAGE}"))?,
        out: out.ok_or_else(|| format!("missing --out\n{USAGE}"))?,
        game,
        hours,
        dump_country,
        dump_out,
    })
}

fn select(all: &[i64], selection: &HourSelection) -> Result<Vec<i64>, String> {
    match selection {
        HourSelection::All => Ok(all.to_vec()),
        HourSelection::Sample => {
            let mut picks: Vec<i64> = Vec::new();
            if let Some(first) = all.first() {
                picks.push(*first);
            }
            if all.len() > 2 {
                picks.push(all[all.len() / 2]);
            }
            if let Some(last) = all.last() {
                if !picks.contains(last) {
                    picks.push(*last);
                }
            }
            Ok(picks)
        }
        HourSelection::Explicit(picked) => {
            // DEDUPLICATED, in the order given. A repeated hour would be
            // absorbed twice and double every tally it touches, which is a
            // silently wrong census rather than a visible error; and the
            // `--dump-country` map is keyed by hour, so the duplicate would
            // vanish there and the two halves of one run would disagree.
            let mut seen = BTreeSet::new();
            let mut selected = Vec::with_capacity(picked.len());
            for hour in picked {
                if !all.contains(hour) {
                    return Err(format!("hour {hour} is not in this recording"));
                }
                if seen.insert(*hour) {
                    selected.push(*hour);
                }
            }
            Ok(selected)
        }
    }
}

#[derive(Default)]
struct Census {
    world: BTreeMap<String, CarrierTally>,
    country: BTreeMap<String, CarrierTally>,
    country_keys: BTreeSet<String>,
    /// Country keys the fold could not name — it keeps the numeric
    /// `tag_db_id` spelling rather than fabricating a TAG. One of these makes
    /// the D18 roster incomplete for the whole hour, so the census names them
    /// instead of letting them hide inside the key count.
    numeric_country_keys: BTreeSet<String>,
    hours_counted: u64,
}

impl Census {
    fn absorb(&mut self, frame: &RawFrame) {
        self.hours_counted += 1;
        for (key, value) in &frame.rest {
            // The `countries` object is a carrier in its own right — its member
            // count is the emitted country count — AND the country grain's
            // whole source, so it is tallied here and split there.
            self.world
                .entry(key.clone())
                .or_default()
                .record(frame.hour_index, value);
            if key == "countries" {
                self.absorb_countries(frame.hour_index, value);
                continue;
            }
            if let Some(members) = section_members(value) {
                for (member_key, member) in members {
                    self.world
                        .entry(format!("{key}.{member_key}"))
                        .or_default()
                        .record(frame.hour_index, member);
                }
            }
        }
    }

    fn absorb_countries(&mut self, hour: i64, value: &Value) {
        let Some(countries) = value.as_object() else {
            return;
        };
        for (country_key, country) in countries {
            self.country_keys.insert(country_key.clone());
            if country_key.parse::<u32>().is_ok() {
                self.numeric_country_keys.insert(country_key.clone());
            }
            let Some(country) = country.as_object() else {
                continue;
            };
            for (key, member) in country {
                self.tally_country(hour, key, member);
                // Sections split at country grain too, and for the same reason
                // as at world grain: `capitulation` is a section whose member
                // `capitulation_progress_raw` is the carrier a consumer reads,
                // and `extra` is a section holding every `_r2_*` carrier.
                if let Some(members) = section_members(member) {
                    for (member_key, member_value) in members {
                        self.tally_country(hour, &format!("{key}.{member_key}"), member_value);
                    }
                }
            }
        }
    }

    fn tally_country(&mut self, hour: i64, key: &str, value: &Value) {
        let tally = self.country.entry(key.to_string()).or_default();
        tally.country_hours += 1;
        tally.record(hour, value);
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("carrier_census: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let source = match &args.game {
        Some(game) => open_recording_game(&args.path, game),
        None => open_recording(&args.path),
    }
    .map_err(|error| format!("open recording {}: {error}", args.path.display()))?;
    let all_hours = source.hours().to_vec();
    let selected = select(&all_hours, &args.hours)?;

    let mut census = Census::default();
    let mut dumped: Map<String, Value> = Map::new();
    for hour in &selected {
        let frame = source
            .frame(*hour)
            .map_err(|error| format!("hour {hour}: {error}"))?;
        if let Some(wanted) = &args.dump_country {
            if let Some(country) = frame
                .rest
                .get("countries")
                .and_then(Value::as_object)
                .and_then(|countries| countries.get(wanted))
            {
                dumped.insert(hour.to_string(), country.clone());
            }
        }
        census.absorb(&frame);
    }
    if let Some(dump_out) = &args.dump_out {
        let mut bytes = serde_json::to_vec_pretty(&json!({
            "schema": "hoi4.replay.carrier_census.country_dump.v1",
            "recording": args.path.display().to_string(),
            "country_key": args.dump_country,
            "hours_carrying_the_key": dumped.len(),
            "by_hour": dumped,
        }))
        .map_err(|error| format!("serialize country dump: {error}"))?;
        bytes.push(b'\n');
        std::fs::write(dump_out, &bytes)
            .map_err(|error| format!("write {}: {error}", dump_out.display()))?;
        println!(
            "country {:?}: carried in {} of {} counted hour(s) -> {}",
            args.dump_country.as_deref().unwrap_or(""),
            dumped.len(),
            selected.len(),
            dump_out.display()
        );
    }

    let report = json!({
        "schema": CENSUS_SCHEMA,
        "recording": args.path.display().to_string(),
        "game_id": source.game_id(),
        "recording_hour_count": all_hours.len(),
        "hours_counted": census.hours_counted,
        "first_hour_counted": selected.first(),
        "last_hour_counted": selected.last(),
        "open_warnings": source.warnings(),
        "distinct_country_keys": census.country_keys.len(),
        "unnamed_numeric_country_keys": census.numeric_country_keys.iter().collect::<Vec<_>>(),
        "world_carriers": census
            .world
            .iter()
            .map(|(key, tally)| (key.clone(), tally.to_json(false)))
            .collect::<Map<String, Value>>(),
        "country_carriers": census
            .country
            .iter()
            .map(|(key, tally)| (key.clone(), tally.to_json(true)))
            .collect::<Map<String, Value>>(),
    });
    let mut bytes =
        serde_json::to_vec_pretty(&report).map_err(|error| format!("serialize report: {error}"))?;
    bytes.push(b'\n');
    std::fs::write(&args.out, &bytes)
        .map_err(|error| format!("write {}: {error}", args.out.display()))?;
    println!(
        "{}: {} of {} hour(s) counted, {} world carrier(s), {} country carrier(s), \
         {} country key(s) ({} unnamed) -> {}",
        source.game_id(),
        census.hours_counted,
        all_hours.len(),
        census.world.len(),
        census.country.len(),
        census.country_keys.len(),
        census.numeric_country_keys.len(),
        args.out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_count_reads_arrays_objects_and_scalars() {
        // kills: returning 1 for an array, or the array length for an object.
        assert_eq!(row_count(&json!([1, 2, 3])), 3);
        assert_eq!(row_count(&json!({"a": 1, "b": 2})), 2);
        assert_eq!(row_count(&json!(7)), 1);
        assert_eq!(row_count(&json!(null)), 0);
    }

    #[test]
    fn a_null_carrier_counts_as_null_and_never_as_a_present_row() {
        // kills: folding present-but-null into hours_present, which would make
        // a domain that emitted nothing all recording look like it emitted.
        let mut tally = CarrierTally::default();
        tally.record(7, &json!(null));
        tally.record(8, &json!([1, 2]));
        assert_eq!(tally.null_hours.len(), 1);
        assert_eq!(tally.present_hours.len(), 1);
        assert_eq!(tally.rows, 2);
        assert_eq!(tally.min_rows, Some(2));
        assert_eq!(tally.max_rows, Some(2));
    }

    #[test]
    fn one_hour_observed_once_per_country_is_still_one_hour() {
        // kills: counting (hour, country) observations as hours. Country grain
        // looks at a carrier once per country, so a 250-hour recording with 440
        // countries published `hours_present = 109,311` — a figure that cannot
        // be an hour count. Both numbers are kept because both are wanted: the
        // hour count says WHEN, the observation count says HOW MANY COPIES.
        let mut tally = CarrierTally::default();
        for _ in 0..440 {
            tally.record(7, &json!([1]));
        }
        tally.record(8, &json!([1]));
        assert_eq!(tally.present_hours.len(), 2, "two hours, not 441");
        assert_eq!(tally.observations_present, 441);
        assert_eq!(tally.rows, 441);
    }

    #[test]
    fn an_hour_can_be_both_present_and_null_at_country_grain() {
        // kills: making the two hour sets exclusive. In one hour one country's
        // copy of a carrier is null while another's is not, and collapsing that
        // would report the hour as wholly one or wholly the other.
        let mut tally = CarrierTally::default();
        tally.record(7, &json!(null));
        tally.record(7, &json!([1]));
        assert_eq!(tally.null_hours.len(), 1);
        assert_eq!(tally.present_hours.len(), 1);
        assert_eq!(tally.observations_null, 1);
        assert_eq!(tally.observations_present, 1);
    }

    #[test]
    fn an_empty_present_carrier_is_distinguished_from_an_absent_one() {
        // kills: reporting `hours_empty` as `hours_null`, which would dress a
        // domain that emitted a definite zero as a domain that emitted nothing.
        let mut tally = CarrierTally::default();
        tally.record(7, &json!([]));
        assert_eq!(tally.present_hours.len(), 1);
        assert_eq!(tally.empty_hours.len(), 1);
        assert_eq!(tally.null_hours.len(), 0);
        assert_eq!(tally.rows, 0);
    }

    #[test]
    fn declarations_are_collected_and_bounded() {
        // kills: dropping the version/authority declarations (the whole point
        // of the census), and letting a per-hour-varying one grow unbounded.
        let mut tally = CarrierTally::default();
        tally.record(7, &json!({"schema_version": 4, "authority": "exact_a"}));
        tally.record(8, &json!({"schema_version": 4, "authority": "exact_b"}));
        assert_eq!(
            tally.schema_versions.iter().collect::<Vec<_>>(),
            vec!["4"],
            "one distinct version, recorded once"
        );
        assert_eq!(tally.authorities.len(), 2);
        for index in 0..MAX_DISTINCT_DECLARATIONS + 4 {
            tally.record(9, &json!({ "authority": format!("a{index}") }));
        }
        assert_eq!(tally.authorities.len(), MAX_DISTINCT_DECLARATIONS);
        assert!(tally.overflowed_declarations);
    }

    #[test]
    fn country_grain_splits_extra_and_names_unnamed_keys() {
        // kills: flattening `extra` (which hides every `_r2_*` carrier behind
        // one `extra` row), and losing the numeric country keys that make the
        // D18 roster incomplete.
        let mut census = Census::default();
        census.absorb_countries(7, &json!({
            "GER": {"extra": {"_r2_tag_db_id": 47, "_r2_weather_live": [1, 2]}, "focus": {"a": 1}},
            "367": {"production": {"queue": []}},
        }));
        assert_eq!(census.country_keys.len(), 2);
        assert_eq!(
            census.numeric_country_keys.iter().collect::<Vec<_>>(),
            vec!["367"]
        );
        assert!(census.country.contains_key("extra._r2_weather_live"));
        assert!(census.country.contains_key("focus"));
        assert!(census.country.contains_key("focus.a"));
        assert!(census.country.contains_key("production"));
        assert!(
            census.country.contains_key("extra"),
            "a section is tallied for its own member count AND split"
        );
        assert_eq!(census.country["extra._r2_weather_live"].rows, 2);
    }

    #[test]
    fn a_named_section_splits_and_a_numeric_collection_does_not() {
        // kills: hiding `map.rail_replay_state_live` inside a `map` row count
        // (the drift this census exists to find lives one level down), and
        // splitting `provinces` into 13,000 carriers named after province ids.
        assert!(section_members(&json!({"rail_replay_state_live": {}, "detail": 1})).is_some());
        assert!(section_members(&json!({"11": {}, "12": {}, "13": {}})).is_none());
        assert!(
            section_members(&json!({"11": {}, "12": {}, "complete": true})).is_none(),
            "a collection with one named sidecar field is still a collection"
        );
        assert!(
            section_members(&json!({"158455:11": {}, "158141:7": {}})).is_none(),
            "`naval_combats` is keyed by `<combat_id>:<generation>`, not by field name"
        );
        assert!(section_members(&json!([1, 2])).is_none());
        assert!(section_members(&json!({})).is_none());
    }

    #[test]
    fn world_grain_tallies_a_section_and_its_members() {
        // kills: reporting only the section, or only its members — the report
        // needs both, because a section's own presence is what says whether a
        // missing member is absent or the whole section is.
        let mut census = Census::default();
        census.absorb(&RawFrame {
            hour_index: 1,
            date: None,
            save_version: None,
            schema_version: None,
            rest: json!({
                "map": {"rail_replay_state_live": {"schema_version": 4}, "detail": null},
                "provinces": {"11": {}, "12": {}},
            })
            .as_object()
            .expect("object")
            .clone(),
        });
        assert_eq!(census.world["map"].rows, 2, "the section's member count");
        assert_eq!(
            census.world["map.rail_replay_state_live"]
                .schema_versions
                .iter()
                .collect::<Vec<_>>(),
            vec!["4"]
        );
        assert_eq!(census.world["map.detail"].null_hours.len(), 1);
        assert_eq!(census.world["provinces"].rows, 2);
        assert!(
            !census.world.contains_key("provinces.11"),
            "a numeric collection is never split"
        );
    }

    #[test]
    fn countries_is_split_at_country_grain_and_never_at_world_grain() {
        // kills: letting `countries` fall through to `section_members`. A TAG
        // key IS identifier-shaped, so the majority test alone would call the
        // country map a section and emit 440 world carriers named after
        // countries — burying the country grain the census exists to report.
        let mut census = Census::default();
        census.absorb(&RawFrame {
            hour_index: 1,
            date: None,
            save_version: None,
            schema_version: None,
            rest: json!({"countries": {"GER": {"focus": [1]}, "ENG": {"focus": [2, 3]}}})
                .as_object()
                .expect("object")
                .clone(),
        });
        assert_eq!(census.world["countries"].rows, 2, "the emitted country count");
        assert!(!census.world.contains_key("countries.GER"));
        assert_eq!(census.country["focus"].rows, 3);
        assert_eq!(census.country["focus"].country_hours, 2);
    }

    #[test]
    fn sample_selection_is_first_middle_last_and_explicit_hours_must_exist() {
        // kills: an explicit --hours list silently counting nothing.
        let hours: Vec<i64> = (100..111).collect();
        assert_eq!(
            select(&hours, &HourSelection::Sample).expect("sample"),
            vec![100, 105, 110]
        );
        assert_eq!(
            select(&hours, &HourSelection::Explicit(vec![101, 109])).expect("explicit"),
            vec![101, 109]
        );
        let error = select(&hours, &HourSelection::Explicit(vec![999]))
            .expect_err("an hour outside the recording is refused");
        assert!(error.contains("999"), "the refusal names the hour: {error}");
        // kills: absorbing a repeated hour twice, which doubles every tally it
        // touches while the hour-keyed country dump keeps only one copy.
        assert_eq!(
            select(&hours, &HourSelection::Explicit(vec![101, 109, 101])).expect("deduplicated"),
            vec![101, 109]
        );
    }
}
