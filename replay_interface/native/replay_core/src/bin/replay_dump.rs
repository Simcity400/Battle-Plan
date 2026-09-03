
use replay_core::{
    open_recording, open_recording_game, Leaf, MapLayerSource, ProductionProfileAuthority,
    WorldFrame, WorldTimeline,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: replay_dump <recording-path> [--hour N] [--game ID]";
const SAMPLE_TAGS: usize = 8;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("replay_dump: {msg}");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    path: PathBuf,
    hour: Option<i64>,
    game: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut path: Option<PathBuf> = None;
    let mut hour: Option<i64> = None;
    let mut game: Option<String> = None;
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--hour" => {
                let v = argv
                    .next()
                    .ok_or_else(|| format!("--hour needs a value\n{USAGE}"))?;
                hour = Some(
                    v.parse()
                        .map_err(|e| format!("--hour {v:?} is not an integer: {e}"))?,
                );
            }
            "--game" => {
                let v = argv
                    .next()
                    .ok_or_else(|| format!("--game needs a value\n{USAGE}"))?;
                game = Some(v);
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
    Ok(Args {
        path: path.ok_or_else(|| format!("missing <recording-path>\n{USAGE}"))?,
        hour,
        game,
    })
}

fn render_list(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let source = match &args.game {
        Some(game) => open_recording_game(&args.path, game),
        None => open_recording(&args.path),
    }
    .map_err(|e| e.to_string())?;
    match source.product_admission() {
        None => println!("product admission: no recorder manifest"),
        Some(admission) if admission.is_full_production_profile() => {
            println!("product admission: full production profile")
        }
        Some(admission) => {
            println!(
                "product admission: {} profile authority, admitted without {}",
                match admission.authority {
                    ProductionProfileAuthority::Unstated => "unstated",
                    ProductionProfileAuthority::Disabled => "disabled",
                    ProductionProfileAuthority::Enabled => "enabled",
                },
                admission
                    .guarantees_not_held
                    .iter()
                    .map(|guarantee| guarantee.token())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!(
                "  profile domains not scheduled: {}",
                if admission.authority == ProductionProfileAuthority::Unstated {
                    "unknown (no production_profile authority)".to_string()
                } else {
                    render_list(&admission.profile_domains_not_scheduled)
                }
            );
            println!(
                "  required static artifacts missing: {}",
                render_list(&admission.missing_required_static_artifacts)
            );
            println!(
                "  attach-time proof refusals: {}",
                render_list(
                    &admission
                        .attach_time_refusals
                        .iter()
                        .map(|row| format!("{} ({})", row.role, row.rendered()))
                        .collect::<Vec<_>>()
                )
            );
            println!(
                "  static artifacts carried but refused (no content epoch): {}",
                render_list(
                    &admission
                        .static_artifacts_not_content_bound
                        .iter()
                        .map(|row| format!("{} ({})", row.role, row.rendered()))
                        .collect::<Vec<_>>()
                )
            );
        }
    }
    let timeline = WorldTimeline::open(source).map_err(|e| e.to_string())?;

    println!("game_id: {}", timeline.game_id());
    println!(
        "hours: {} frames, hour_index {}..{}",
        timeline.hours().len(),
        timeline.first_hour(),
        timeline.last_hour()
    );
    if timeline.warnings().is_empty() {
        println!("ingest warnings: none");
    } else {
        println!("ingest warnings:");
        for w in timeline.warnings() {
            println!("  - {w}");
        }
    }

    for label_hour in [
        ("first frame", timeline.first_hour()),
        ("last frame", timeline.last_hour()),
    ] {
        let (label, hour) = label_hour;
        let frame = timeline.frame(hour).map_err(|e| e.to_string())?;
        print_frame_coverage(label, &frame);
    }

    if let Some(hour) = args.hour {
        let frame = timeline.state_at(hour).map_err(|e| e.to_string())?;
        print_world_summary(hour, &frame);
    }
    Ok(())
}

fn print_frame_coverage(label: &str, frame: &WorldFrame) {
    println!(
        "{label}: hour_index {} date {}",
        frame.hour_index(),
        frame.date().unwrap_or("null(unobtained)")
    );
    let prov = frame.provenance();
    if prov.channels.is_empty() {
        println!("  provenance: no channels recorded");
    }
    for (channel, coverage) in &prov.channels {
        println!(
            "  provenance {channel}: captured={} domains=[{}]",
            fmt_leaf_display(&coverage.captured),
            coverage.domains.join(", ")
        );
    }
    for note in &prov.notes {
        println!("  provenance note: {note}");
    }
    println!("  coverage warnings: {}", frame.coverage_warnings().len());
    for w in frame.coverage_warnings() {
        println!("    - {w}");
    }
    print_snow_mud(frame);
}

fn print_snow_mud(frame: &WorldFrame) {
    let view = frame.snow_mud();
    let mut measured = 0usize;
    let mut mud_measured = 0usize;
    let mut mud_true = 0usize;
    let mut min_snow: Option<i64> = None;
    let mut max_snow: Option<i64> = None;
    for cell in view.by_province.values() {
        if let Leaf::Measured(raw) = &cell.snow_raw {
            measured += 1;
            min_snow = Some(min_snow.map_or(*raw, |m: i64| m.min(*raw)));
            max_snow = Some(max_snow.map_or(*raw, |m: i64| m.max(*raw)));
        }
        if let Leaf::Measured(mud) = &cell.mud {
            mud_measured += 1;
            if *mud {
                mud_true += 1;
            }
        }
    }
    println!(
        "  snow_mud: status={:?} by_province_status={:?} cells={} scale_raw={}",
        view.status,
        view.by_province_status,
        view.by_province.len(),
        fmt_leaf_display(&view.scale_raw)
    );
    println!(
        "    snow_raw measured={measured} range={}..={} | mud measured={mud_measured} true={mud_true}",
        min_snow.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string()),
        max_snow.map(|v| v.to_string()).unwrap_or_else(|| "-".to_string())
    );
}

fn print_world_summary(requested_hour: i64, frame: &WorldFrame) {
    println!(
        "state at hour {requested_hour} -> frame hour_index {} date {}",
        frame.hour_index(),
        frame.date().unwrap_or("null(unobtained)")
    );
    print_snow_mud(frame);

    let mut owned: BTreeMap<&str, usize> = BTreeMap::new();
    let mut states_unmeasured = 0usize;
    for leaf in frame.owner_by_state().values() {
        match leaf {
            Leaf::Measured(tag) => *owned.entry(tag.tag()).or_default() += 1,
            _ => states_unmeasured += 1,
        }
    }
    println!(
        "  owned states ({}): {} [{} entries without a measured owner]",
        fmt_layer_source(frame.owner_by_state_source()),
        fmt_counts(&owned),
        states_unmeasured
    );

    let mut divisions: BTreeMap<&str, usize> = BTreeMap::new();
    let mut divisions_unowned = 0usize;
    for row in frame.divisions() {
        match &row.owner {
            Leaf::Measured(tag) => *divisions.entry(tag.tag()).or_default() += 1,
            _ => divisions_unowned += 1,
        }
    }
    println!(
        "  divisions: {} [{} rows without a measured owner]",
        fmt_counts(&divisions),
        divisions_unowned
    );

    println!("  active combats: {}", frame.combats().len());
    for combat in frame.combats() {
        println!(
            "    combat_id={} kind={} province={} attackers={} defenders={}",
            fmt_leaf_display(&combat.combat_id),
            fmt_leaf_display(&combat.kind),
            fmt_leaf_display(&combat.location_province),
            fmt_leaf_ids(&combat.attackers),
            fmt_leaf_ids(&combat.defenders)
        );
    }
    println!(
        "  ended combats: {} status={:?}",
        frame.ended_combats().len(),
        frame.ended_combats_status()
    );
    for ((combat_id, generation), combat) in frame.ended_combats() {
        println!(
            "    ({combat_id},{generation}) kind={:?} lifecycle={:?} province={:?}",
            combat.kind, combat.lifecycle, combat.location_province
        );
    }

    println!(
        "  country core (first {} of {} tags):",
        frame.countries().len().min(SAMPLE_TAGS),
        frame.countries().len()
    );
    for (tag, core) in frame.countries().iter().take(SAMPLE_TAGS) {
        println!(
            "    {tag}: stability={} war_support={} political_power={} ruling_party={} \
             fuel={}/{} divisions={}",
            fmt_leaf_display(&core.stability),
            fmt_leaf_display(&core.war_support),
            fmt_leaf_display(&core.political_power),
            fmt_leaf_display(&core.ruling_party),
            fmt_leaf_display(&core.fuel_current),
            fmt_leaf_display(&core.fuel_max),
            fmt_leaf_display(&core.division_count)
        );
    }
}

fn fmt_layer_source(source: MapLayerSource) -> &'static str {
    match source {
        MapLayerSource::MapMirror => "from map mirror",
        MapLayerSource::DomainWalk => "walked from domain objects",
        MapLayerSource::Missing => "layer missing",
    }
}

fn fmt_leaf_display<T: std::fmt::Display>(leaf: &Leaf<T>) -> String {
    match leaf {
        Leaf::Measured(v) => format!("{v}"),
        Leaf::Unobtained => "null(unobtained)".to_string(),
        Leaf::Absent => "absent".to_string(),
    }
}

fn fmt_leaf_ids(leaf: &Leaf<Vec<i64>>) -> String {
    match leaf {
        Leaf::Measured(ids) => format!(
            "[{}]",
            ids.iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ),
        Leaf::Unobtained => "null(unobtained)".to_string(),
        Leaf::Absent => "absent".to_string(),
    }
}

fn fmt_counts(counts: &BTreeMap<&str, usize>) -> String {
    if counts.is_empty() {
        return "none".to_string();
    }
    counts
        .iter()
        .map(|(tag, n)| format!("{tag}={n}"))
        .collect::<Vec<_>>()
        .join(", ")
}
