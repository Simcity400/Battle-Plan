//! What the XMAP14 combat-indicator and XMAP17 label layers actually do over a
//! whole recording, hour by hour, and why each one refuses when it refuses.
//!
//! Both layers answer from carriers the recording either holds or does not. A
//! per-hour count separates "this hour genuinely had nothing to draw" from
//! "the input was never recorded", which a single frame's refusal string
//! cannot.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use replay_core::{open_recording, open_recording_game, Leaf, ViewStatus, WorldFrame, WorldTimeline};
use serde_json::{json, Map, Value};

const USAGE: &str = "usage: dump_ur_map_view_gaps <recording-path> [--game ID] [--json OUT]";

const XMAP14_CARRIER_KEY: &str = "_xmap14_combat_indicators_exact";
const XMAP17_FACTION_CARRIER_KEY: &str = "_xmap17_faction_labels_exact";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("dump_ur_map_view_gaps: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut path: Option<PathBuf> = None;
    let mut game: Option<String> = None;
    let mut json_out: Option<PathBuf> = None;
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--game" => game = Some(argv.next().ok_or("--game needs a value")?),
            "--json" => json_out = Some(PathBuf::from(argv.next().ok_or("--json needs a value")?)),
            "--help" | "-h" => return Err(USAGE.to_string()),
            other if other.starts_with("--") => return Err(format!("unknown flag {other:?}")),
            other => path = Some(PathBuf::from(other)),
        }
    }
    let path = path.ok_or(USAGE)?;

    let source = match &game {
        Some(id) => open_recording_game(&path, id).map_err(|error| error.to_string())?,
        None => open_recording(&path).map_err(|error| error.to_string())?,
    };
    let timeline = WorldTimeline::open(source).map_err(|error| error.to_string())?;
    let hours = timeline.hours().to_vec();

    let mut tally = Tally::default();
    let mut unreadable_hours = 0usize;
    for hour in &hours {
        match timeline.frame(*hour) {
            Ok(frame) => tally.observe(&frame),
            Err(_) => unreadable_hours += 1,
        }
    }

    let report = json!({
        "schema": "hoi4.ur.xmap14-xmap17-gap-ledger.v1",
        "recording": path.display().to_string(),
        "game": game,
        "hour_count": hours.len(),
        "first_hour": hours.first(),
        "last_hour": hours.last(),
        "unreadable_hours": unreadable_hours,
        "xmap14": tally.xmap14_report(),
        "xmap17": tally.xmap17_report(),
    });

    println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
    if let Some(out) = json_out {
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        std::fs::write(&out, format!("{text}\n"))
            .map_err(|error| format!("write {}: {error}", out.display()))?;
    }
    Ok(())
}

/// A carrier is absent, an explicit null, or a value. The three are different
/// claims and a count that merged them would hide the one that matters.
#[derive(Default)]
struct CarrierTally {
    absent: usize,
    explicit_null: usize,
    present: usize,
}

impl CarrierTally {
    fn observe(&mut self, frame: &WorldFrame, key: &str) {
        match frame.domain(key) {
            None => self.absent += 1,
            Some(Value::Null) => self.explicit_null += 1,
            Some(_) => self.present += 1,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "absent": self.absent,
            "explicit_null": self.explicit_null,
            "present": self.present,
        })
    }
}

#[derive(Default)]
struct Tally {
    hours: usize,
    xmap14_carrier: CarrierTally,
    combats_status: BTreeMap<String, usize>,
    naval_status: BTreeMap<String, usize>,
    land_rows: usize,
    naval_rows: usize,
    hours_with_land_rows: usize,
    hours_with_naval_rows: usize,
    land_rows_max: usize,
    naval_rows_max: usize,

    faction_carrier: CarrierTally,
    player_named_countries_max: usize,
    player_unnamed_human_max: usize,
    player_co_op_max: usize,
    player_shared_key_max: usize,
    faction_view_status: BTreeMap<String, usize>,
    player_view_status: BTreeMap<String, usize>,
    faction_view_diagnostic: BTreeMap<String, usize>,
    player_view_diagnostic: BTreeMap<String, usize>,
    emitted_causes: BTreeMap<String, usize>,
    dark_layer_hours: usize,
    cause_rows: usize,
    faction_members_status: BTreeMap<String, usize>,
    factions_max: usize,
    hours_with_factions: usize,
    subject_autonomy_status: BTreeMap<String, usize>,
    subject_autonomy_rows_max: usize,
    d18_complete_hours: usize,
    d18_partial_hours: usize,
    d18_refused_hours: usize,
    d18_excluded_max: usize,
}

fn bump(map: &mut BTreeMap<String, usize>, key: impl Into<String>) {
    *map.entry(key.into()).or_default() += 1;
}

fn leaf_state<T>(leaf: &Leaf<T>) -> &'static str {
    match leaf {
        Leaf::Measured(_) => "measured",
        Leaf::Absent => "absent",
        Leaf::Unobtained => "unobtained",
    }
}

impl Tally {
    fn observe(&mut self, frame: &WorldFrame) {
        self.hours += 1;
        self.observe_xmap14(frame);
        self.observe_xmap17(frame);
    }

    fn observe_xmap14(&mut self, frame: &WorldFrame) {
        self.xmap14_carrier.observe(frame, XMAP14_CARRIER_KEY);
        bump(
            &mut self.combats_status,
            format!("{:?}", frame.combats_status()),
        );
        let naval = frame.naval_air_combat();
        bump(
            &mut self.naval_status,
            format!("{:?}", naval.naval_combats_status),
        );
        let land = frame.combats().len();
        let sea = naval.naval_combats.len();
        self.land_rows += land;
        self.naval_rows += sea;
        self.land_rows_max = self.land_rows_max.max(land);
        self.naval_rows_max = self.naval_rows_max.max(sea);
        if land > 0 {
            self.hours_with_land_rows += 1;
        }
        if sea > 0 {
            self.hours_with_naval_rows += 1;
        }
    }

    fn observe_xmap17(&mut self, frame: &WorldFrame) {
        self.faction_carrier.observe(frame, XMAP17_FACTION_CARRIER_KEY);

        let diplomacy = frame.diplomacy();
        let faction_view = frame.xmap17_faction_labels_exact();
        bump(
            &mut self.faction_view_status,
            format!("{:?}", faction_view.status),
        );
        if let Some(diagnostic) = &faction_view.diagnostic {
            bump(&mut self.faction_view_diagnostic, diagnostic.clone());
        }
        let player_view = frame.xmap17_player_labels_derived();
        bump(
            &mut self.player_view_status,
            format!("{:?}", player_view.status),
        );
        if let Some(diagnostic) = &player_view.diagnostic {
            bump(&mut self.player_view_diagnostic, diagnostic.clone());
        }
        if let Some(derived) = &player_view.derived {
            let named = derived
                .rows
                .iter()
                .filter(|row| row.player_key_hash.is_some())
                .count();
            self.player_named_countries_max = self.player_named_countries_max.max(named);
            self.player_unnamed_human_max = self
                .player_unnamed_human_max
                .max(derived.unnamed_human_countries.len());
            self.player_co_op_max = self.player_co_op_max.max(derived.co_op_countries.len());
            self.player_shared_key_max = self
                .player_shared_key_max
                .max(derived.shared_player_key_countries.len());
        }

        bump(
            &mut self.faction_members_status,
            leaf_state(&diplomacy.faction_members),
        );
        if let Leaf::Measured(roster) = &diplomacy.faction_members {
            self.factions_max = self.factions_max.max(roster.len());
            if !roster.is_empty() {
                self.hours_with_factions += 1;
            }
        }
        bump(
            &mut self.subject_autonomy_status,
            leaf_state(&diplomacy.subject_autonomy_current),
        );
        if let Leaf::Measured(rows) = &diplomacy.subject_autonomy_current {
            self.subject_autonomy_rows_max = self.subject_autonomy_rows_max.max(rows.len());
        }

        // The causes the renderer would actually emit for this frame, taken
        // from the producer's own refusal functions rather than re-derived.
        let causes = map_viewer::xmap17_dark_label_layer_causes(frame);
        let mut dark_layers = std::collections::BTreeSet::new();
        for (layer, cause) in &causes {
            bump(&mut self.emitted_causes, format!("{layer}:{cause}"));
            dark_layers.insert(*layer);
        }
        self.dark_layer_hours += dark_layers.len();
        self.cause_rows += causes.len();

        let roster = frame.d18_country_roster();
        match roster.partial() {
            Ok(partial) => {
                self.d18_excluded_max = self.d18_excluded_max.max(partial.excluded.len());
                if roster.complete().is_ok() {
                    self.d18_complete_hours += 1;
                } else {
                    self.d18_partial_hours += 1;
                }
            }
            Err(_) => self.d18_refused_hours += 1,
        }
    }

    fn xmap14_report(&self) -> Value {
        let mut out = Map::new();
        out.insert("carrier_key".into(), json!(XMAP14_CARRIER_KEY));
        out.insert("carrier".into(), self.xmap14_carrier.to_json());
        out.insert(
            "combats_status".into(),
            json!(self.combats_status),
        );
        out.insert("naval_combats_status".into(), json!(self.naval_status));
        out.insert(
            "recorded_combat_rows".into(),
            json!({
                "land_total": self.land_rows,
                "land_hours_nonempty": self.hours_with_land_rows,
                "land_max_in_one_hour": self.land_rows_max,
                "naval_total": self.naval_rows,
                "naval_hours_nonempty": self.hours_with_naval_rows,
                "naval_max_in_one_hour": self.naval_rows_max,
            }),
        );
        Value::Object(out)
    }

    fn xmap17_report(&self) -> Value {
        json!({
            "emitted_dark_layer_causes": self.emitted_causes,
            "dark_layer_hours": self.dark_layer_hours,
            "emitted_cause_rows": self.cause_rows,
            "faction_carrier_key": XMAP17_FACTION_CARRIER_KEY,
            "faction_carrier": self.faction_carrier.to_json(),
            "faction_view_status": self.faction_view_status,
            "faction_view_diagnostic": self.faction_view_diagnostic,
            "player_layer_source": "derived from D18 roster + D57 players_by_country",
            "player_named_countries_max": self.player_named_countries_max,
            "player_unnamed_human_countries_max": self.player_unnamed_human_max,
            "player_co_op_countries_max": self.player_co_op_max,
            "player_shared_player_key_countries_max": self.player_shared_key_max,
            "player_view_status": self.player_view_status,
            "player_view_diagnostic": self.player_view_diagnostic,
            "d53_faction_members_leaf": self.faction_members_status,
            "d53_factions_max_in_one_hour": self.factions_max,
            "d53_hours_with_a_faction": self.hours_with_factions,
            "d53_subject_autonomy_current_leaf": self.subject_autonomy_status,
            "d53_subject_autonomy_rows_max": self.subject_autonomy_rows_max,
            "d18_roster": {
                "complete_hours": self.d18_complete_hours,
                "partial_hours": self.d18_partial_hours,
                "refused_hours": self.d18_refused_hours,
                "excluded_max_in_one_hour": self.d18_excluded_max,
            },
        })
    }
}

#[allow(dead_code)]
fn status_is_present(status: ViewStatus) -> bool {
    status == ViewStatus::Present
}
