//! Emit row-major and column-major encodings of a synthetic d20 campaign, so the
//! §13 layout question can be decided by measurement.
//!
//! This writes raw bytes; `bench_layout.py` compresses them and reports. Splitting
//! it that way reuses the same zstd path as the committed chunked-compression
//! benchmark rather than introducing a second compressor to disagree with it.
//!
//! One count here is measured and the rest is modelled, and the difference
//! matters. **Measured:** 3,051 division bodies — a direct-root proof found
//! exactly that, with zero rejected vtable hits. **Modelled, i.e. assumed:** that
//! a few percent are moving or fighting at any hour, and that organisation and
//! strength follow bounded random walks. Only a real capture settles the second
//! group, and the headline ratio depends on it.
//!
//! **The model matters more than the code here.** An earlier version re-rolled
//! independent noise every hour, which has no temporal correlation at all and so
//! maximally punishes row-major's cross-hour redundancy — it flattered columnar.
//! Both models are reported, because the gap between them is the honest measure
//! of how much the answer depends on an assumption rather than on d20.

use std::collections::BTreeMap;
use std::io::Write;

use wire_format::body::{encode_baseline, key, KeyedState};
use wire_format::columnar::encode_baseline_columnar;
use wire_format::d20;
use wire_format::record::Record;
use wire_format::value::Value;
use wire_format::Kind;

const DIVISIONS: u64 = 3051;
const HOURS: u64 = 720; // one game month at hourly cadence

/// Per-hour hazards for the two persistent states, sized so the realised rates
/// are the "few percent" this model claims and stay there for the whole month
/// rather than drifting toward 50% the way a symmetric toggle does.
///
/// Movement settles near `(D - 1) / (D + S)` = 5.3%, not `D / (D + S)` = 6.0%,
/// because a division that arrives is stationary when the hour is counted. The
/// occupancy printout is the authority on that, not this comment — it is
/// measured from the world each hour, and the gap between a predicted rate and
/// the measured one is exactly what three earlier versions of this benchmark got
/// wrong.
const MOVE_START: u64 = 125;
const FIGHT_START: u64 = 194;
const FIGHT_STOP: u64 = 6;

/// A move takes this many hours, and then the division **arrives**.
///
/// Movement ends by completion, not by a second random hazard. With a stop
/// hazard the two processes fought each other: a leg needed 20 progress steps
/// while the hazard ended it every ~8 hours, so 16,309 of 16,496 advertised
/// journeys — 98.8% — were cancelled and the unit never reached the province the
/// row named as its target. A model that advertises a destination almost nothing
/// reaches is not modelling movement.
const MOVE_HOURS: u64 = 8;

/// Where a division arrives when a move completes. Province ids run 1000..1899,
/// so the wrap happens in the offset, not on the absolute id.
///
/// This exists as a function because the destination is written in two places —
/// the row's advertised target and the step that moves the unit — and when they
/// were two expressions they disagreed: a unit at 1010 advertised 1011 and
/// arrived at 1111, on every completed move in the month.
fn next_province(p: u64) -> u64 {
    1000 + ((p - 1000 + 1) % 900)
}

/// Deterministic pseudo-random, so two runs of this benchmark are comparable and
/// a surprising result can be reproduced exactly rather than re-rolled.
fn noise(seed: u64) -> u64 {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x
}

/// One division's evolving state, so hours are correlated the way a real campaign
/// is rather than independent draws.
#[derive(Clone)]
struct Div {
    org: f64,
    strength: f64,
    province: u64,
    moving: bool,
    in_combat: bool,
    progress: f64,
    losses: u64,
    // Every remaining value a row carries lives here too, so `build_hour` is a
    // pure function of the world. When these were constants inlined into
    // `build_hour`, the "maximal churn" control could not touch them: it froze
    // eight of the fifteen carried values, which is columnar's *best* case
    // rather than its worst.
    owner: u64,
    experience: f64,
    current_supply: f64,
    max_supply: f64,
    disrupted_supply: f64,
    strategic_redeployment: bool,
    army_id: u64,
    manpower: u64,
}

fn seed_world() -> Vec<Div> {
    (0..DIVISIONS)
        .map(|div| {
            let n = noise(div);
            Div {
                org: 0.4 + ((n >> 8) % 600) as f64 / 1000.0,
                strength: 0.5 + ((n >> 16) % 500) as f64 / 1000.0,
                province: 1000 + (div % 900),
                moving: n % 100 < 6,
                in_combat: n % 100 < 3,
                progress: 0.0,
                losses: 0,
                owner: div % 60,
                experience: 0.3,
                current_supply: 0.8,
                max_supply: 1.0,
                disrupted_supply: 0.0,
                strategic_redeployment: false,
                army_id: div / 24,
                manpower: 9000 + (n % 1000),
            }
        })
        .collect()
}

/// The uncorrelated control: every division's values redrawn independently each
/// hour, with no memory. It is the pessimal case for row-major, whose advantage
/// is entirely cross-hour redundancy, so it brackets the answer from the other
/// side. It is kept runnable — `--white-noise` — because the specification cites
/// its number, and a cited number that cannot be re-run is a number nobody can
/// check.
fn step_world_white_noise(world: &mut [Div], hour: u64) {
    for (div, d) in world.iter_mut().enumerate() {
        let n = noise(div as u64 * 7919 + hour);
        let m = noise(n ^ 0x9E37_79B9_7F4A_7C15);
        d.org = ((n >> 8) % 1000) as f64 / 1000.0;
        d.strength = ((n >> 18) % 1000) as f64 / 1000.0;
        d.province = 1000 + ((n >> 28) % 900);
        d.moving = (n >> 40) % 100 < 6;
        d.in_combat = (n >> 45) % 100 < 3;
        d.progress = ((n >> 50) % 100) as f64 / 100.0;
        d.losses = (n >> 3) % 10_000;
        // Every remaining carried value too — that is what makes this the
        // maximal-churn end of the range rather than a mix of noisy and frozen
        // columns.
        d.owner = m % 60;
        d.experience = ((m >> 7) % 1000) as f64 / 1000.0;
        d.current_supply = ((m >> 17) % 1000) as f64 / 1000.0;
        d.max_supply = ((m >> 27) % 1000) as f64 / 1000.0;
        d.disrupted_supply = ((m >> 37) % 1000) as f64 / 1000.0;
        d.strategic_redeployment = (m >> 47) % 2 == 0;
        d.army_id = (m >> 48) % 128;
        d.manpower = 9000 + ((m >> 55) % 1000);
    }
}

/// Step the world one hour: small bounded walks, persistent states, advancing
/// movement. This is the shape an hourly capture actually sees.
fn step_world(world: &mut [Div], hour: u64) {
    for (div, d) in world.iter_mut().enumerate() {
        let n = noise(div as u64 * 7919 + hour);
        // Organisation and strength drift by at most a hundredth per hour.
        let step = |v: f64, bits: u64| -> f64 {
            let delta = ((bits % 3) as f64 - 1.0) * 0.01;
            (v + delta).clamp(0.0, 1.0)
        };
        d.org = step(d.org, n >> 3);
        d.strength = step(d.strength, n >> 11);
        // Movement and combat are **asymmetric** processes: a division is ordered
        // to move rarely and arrives a handful of hours later. A symmetric toggle
        // looks the same per hour and has a 50% stationary distribution, so by
        // month end it models a world where half the army is marching — not the
        // few percent this model claims. `occupancy` measures the realised rates
        // so the claim is checked rather than asserted.
        if !d.moving && (n >> 19) % MOVE_START == 0 {
            d.moving = true;
            d.progress = 0.0;
        }
        if d.in_combat {
            if (n >> 37) % FIGHT_STOP == 0 {
                d.in_combat = false;
            }
        } else if (n >> 37) % FIGHT_START == 0 {
            d.in_combat = true;
        }
        if d.moving {
            // One leg is MOVE_HOURS steps, so a move that starts arrives.
            d.progress += 1.0 / MOVE_HOURS as f64;
            if d.progress >= 1.0 - 1e-9 {
                d.progress = 0.0;
                d.moving = false;
                d.province = next_province(d.province);
            }
        }
        if d.in_combat {
            d.losses += n % 7;
        }
    }
}

fn build_hour(collection: &wire_format::schema::Collection, world: &[Div]) -> KeyedState {
    let mut state = BTreeMap::new();
    for (div, d) in world.iter().enumerate() {
        let div = div as u64;
        let moving = d.moving;
        let in_combat = d.in_combat;
        let org = d.org;
        let strength = d.strength;
        let province = d.province;

        let values: Vec<Value> = collection
            .value_fields()
            .map(|f| match f.field_id {
                d20::field::LOCATION_PROVINCE => Value::ContentId(province),
                d20::field::ORGANISATION => Value::Continuous(org),
                d20::field::STRENGTH => Value::Continuous(strength),
                d20::field::MOVING => Value::Bool(moving),
                d20::field::IN_COMBAT => Value::Bool(in_combat),
                d20::field::DISRUPTED_SUPPLY => Value::Continuous(d.disrupted_supply),
                d20::field::STRATEGIC_REDEPLOYMENT => Value::Bool(d.strategic_redeployment),
                d20::field::OWNER => Value::ContentId(d.owner),
                d20::field::MOVE_PROGRESS => {
                    if moving {
                        Value::Continuous(d.progress)
                    } else {
                        Value::Absent
                    }
                }
                d20::field::MOVE_TARGET_PROVINCE => {
                    if moving {
                        Value::ContentId(next_province(province))
                    } else {
                        Value::Absent
                    }
                }
                d20::field::CURRENT_SUPPLY => Value::Continuous(d.current_supply),
                d20::field::MAX_SUPPLY => Value::Continuous(d.max_supply),
                d20::field::EXPERIENCE => Value::Continuous(d.experience),
                d20::field::CURRENT_MANPOWER => {
                    Value::Unsigned(d.manpower - d.losses.min(500))
                }
                d20::field::CUMULATIVE_LOSSES => Value::Unsigned(d.losses),
                d20::field::ARMY_ID => Value::Unsigned(d.army_id),
                _ => match f.kind {
                    Kind::Boolean => Value::Bool(false),
                    _ => Value::Absent,
                },
            })
            .collect();
        state.insert(key(&[div, 1]), Record::new(values));
    }
    state
}

fn main() -> std::io::Result<()> {
    // `--white-noise` runs the uncorrelated control instead of the committed
    // model. Its output goes to its own files so it can never be mistaken for
    // the headline run.
    let white_noise = std::env::args().any(|a| a == "--white-noise");
    let schema = d20::schema();
    let collection = &schema.collections[0];
    let dir = std::path::Path::new("runs/d20_reference");
    std::fs::create_dir_all(dir)?;

    let mut row_major = Vec::new();
    let mut column_major = Vec::new();
    // Byte offset of each hour in both streams. The compression script sliced by
    // average hour size and flipped a published tenth three reviews running; the
    // encoder knows the real boundaries, so it writes them down.
    let mut row_offsets: Vec<usize> = vec![0];
    let mut col_offsets: Vec<usize> = vec![0];

    // The model's stated shape is measured, not asserted. Two prior versions of
    // this benchmark described dynamics in a comment and implemented different
    // ones — uncorrelated noise, then a symmetric toggle drifting to 50%. Both
    // moved the headline number, and both were invisible because nothing counted.
    let mut occupancy: Vec<(f64, f64)> = Vec::with_capacity(HOURS as usize);

    let mut world = seed_world();
    for hour in 0..HOURS {
        if white_noise {
            step_world_white_noise(&mut world, hour);
        } else {
            step_world(&mut world, hour);
        }
        let moving = world.iter().filter(|d| d.moving).count() as f64;
        let fighting = world.iter().filter(|d| d.in_combat).count() as f64;
        let n = DIVISIONS as f64;
        occupancy.push((100.0 * moving / n, 100.0 * fighting / n));

        let state = build_hour(collection, &world);
        encode_baseline(&schema, &[state.clone()], &mut row_major).expect("row-major encodes");
        encode_baseline_columnar(collection, &state, &mut column_major)
            .expect("column-major encodes");
        row_offsets.push(row_major.len());
        col_offsets.push(column_major.len());
    }

    let mean = |f: fn(&(f64, f64)) -> f64| -> f64 {
        occupancy.iter().map(f).sum::<f64>() / occupancy.len() as f64
    };
    let first = occupancy[0];
    let last = occupancy[occupancy.len() - 1];

    let suffix = if white_noise { "_white_noise" } else { "" };
    std::fs::write(dir.join(format!("row_major{suffix}.bin")), &row_major)?;
    std::fs::write(dir.join(format!("column_major{suffix}.bin")), &column_major)?;
    let offsets = |v: &[usize]| v.iter().map(|o| o.to_string()).collect::<Vec<_>>().join("
");
    std::fs::write(dir.join(format!("row_major{suffix}.offsets")), offsets(&row_offsets))?;
    std::fs::write(dir.join(format!("column_major{suffix}.offsets")), offsets(&col_offsets))?;

    let mut log = std::fs::File::create(dir.join(format!("11_layout_raw_sizes{suffix}.txt")))?;
    writeln!(log, "=== cmd: cargo run -p wire_format --bin bench_layout{}",
             if white_noise { " -- --white-noise" } else { "" })?;
    writeln!(log, "model    : {}", if white_noise {
        "UNCORRELATED CONTROL - values redrawn independently each hour"
    } else {
        "committed model - correlated walks, persistent movement/combat"
    })?;
    writeln!(log, "model    : {DIVISIONS} divisions, {HOURS} hourly baselines, d20 schema v1")?;
    writeln!(log, "dynamics : bounded random walks, persistent movement/combat, advancing progress")?;
    writeln!(log, "\nrealised occupancy (the model's claim, measured):")?;
    writeln!(log, "  moving   hour 0 {:5.2}%  hour {} {:5.2}%  month mean {:5.2}%",
             first.0, HOURS - 1, last.0, mean(|o| o.0))?;
    writeln!(log, "  in combat hour 0 {:5.2}%  hour {} {:5.2}%  month mean {:5.2}%",
             first.1, HOURS - 1, last.1, mean(|o| o.1))?;
    writeln!(log, "  A stationary rate near the hour-0 rate is the point: a symmetric")?;
    writeln!(log, "  toggle would land near 50% by the end and silently change the model.")?;
    writeln!(log, "row-major   raw: {:>12} bytes", row_major.len())?;
    writeln!(log, "column-major raw: {:>12} bytes", column_major.len())?;
    writeln!(
        log,
        "column-major is {:.1}% of row-major before compression",
        100.0 * column_major.len() as f64 / row_major.len() as f64
    )?;
    writeln!(log, "\nCompression is the deciding measurement; see bench_layout.py.")?;

    println!("row-major   raw: {} bytes", row_major.len());
    println!("column-major raw: {} bytes", column_major.len());
    println!(
        "occupancy   : moving {:.2}% -> {:.2}% (mean {:.2}%), combat {:.2}% -> {:.2}% (mean {:.2}%)",
        first.0, last.0, mean(|o| o.0), first.1, last.1, mean(|o| o.1)
    );
    Ok(())
}
