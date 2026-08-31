//! Lane WO instrument: what the XMAP13 and XMAP15 producers got out of every
//! hour of a recording, and what they did not.
//!
//! One fold of one spool. For each hour it reads the keys the two producers
//! write -- `map_unit_presentation`, `map_unit_presentation_gate` and
//! `order_of_battle.extra.xmap15_orders_fronts_movement_semantic_gate` -- and
//! records the censuses they minted beside the recorded inputs the hour did not
//! carry. Nothing is sampled: every hour is a row and the roll-ups are
//! recomputable from them.
//!
//! It also MINTS from the first hour that has anything to mint and carries three
//! rows of the result, because a count of minted rows is not the same evidence
//! as the rows. That mint runs over the snapshot the fold just wrote, through
//! the producers' own projection, with one deliberate difference from the fold:
//! the country map for army-group formations is empty here, so an army group
//! would gate rather than resolve. On a recording whose theatre selector is
//! null that difference cannot arise.
//!
//! usage: wo_xmap13_gate_scan <spool> <out.json> [identity]

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: wo_xmap13_gate_scan <spool> <out.json> [identity]");
        std::process::exit(2);
    }
    let mode = if args.get(3).map(|value| value == "identity").unwrap_or(false) {
        replay_decoder::StreamIdentityMode::IdentityV1
    } else {
        detect_identity_mode(&args[1])
    };
    let source = replay_decoder::Source::StreamFile(std::path::PathBuf::from(&args[1]));
    let mut cursor = source
        .open_cursor_with_identity_mode(&replay_decoder::OpenOptions::default(), mode)
        .unwrap_or_else(|error| panic!("open: {error}"));

    let mut per_hour: Vec<Value> = Vec::new();
    let mut hours_with_carrier_key = 0u64;
    let mut hours_with_gate = 0u64;
    let mut hours_with_no_hour_gate = 0u64;
    let mut hour_gate_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut minted = [0u64; 4];
    let mut hours = 0u64;
    let mut xmap15_hours_with_gate = 0u64;
    let mut xmap15_minted_plan_rows = 0u64;
    let mut xmap15_visual_orders = 0u64;
    let mut xmap15_gate_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut sample: Option<Value> = None;
    loop {
        match cursor.next_hour() {
            Some(Ok(hour)) => {
                hours += 1;
                // RawValue rather than Value: a folded hour is tens of
                // megabytes and every value but two is irrelevant here, so the
                // whole document is tokenized once and nothing under a root key
                // is built. Parsing them all into `Value` made this instrument
                // slower than the fold it is reading.
                let snapshot: BTreeMap<String, &serde_json::value::RawValue> =
                    serde_json::from_slice(&hour.snapshot)
                        .unwrap_or_else(|error| panic!("hour {}: {error}", hour.hour_index));
                let carrier = snapshot.get(portable_render_stream::XMAP13_CARRIER_WIRE_KEY);
                if carrier.is_some() {
                    hours_with_carrier_key += 1;
                }
                let gate = snapshot
                    .get(portable_render_stream::XMAP13_PRODUCER_GATE_WIRE_KEY)
                    .and_then(|raw| {
                        serde_json::from_str::<portable_render_stream::Xmap13ProducerGate>(
                            raw.get(),
                        )
                        .ok()
                    });
                let mut row = json!({
                    "hour_index": hour.hour_index,
                    "carrier_key_present": carrier.is_some(),
                    "carrier_is_typed_null": carrier.map(|raw| raw.get().trim() == "null")
                        == Some(true),
                });
                if let Some(gate) = gate {
                    hours_with_gate += 1;
                    minted[0] += u64::from(gate.minted_division_rows);
                    minted[1] += u64::from(gate.minted_task_force_rows);
                    minted[2] += u64::from(gate.minted_air_wing_rows);
                    minted[3] += u64::from(gate.minted_formations);
                    if gate.hour_gates.is_empty() {
                        hours_with_no_hour_gate += 1;
                    }
                    for hour_gate in &gate.hour_gates {
                        *hour_gate_counts
                            .entry(hour_gate.recorded_input.clone())
                            .or_default() += 1;
                    }
                    let object = row.as_object_mut().expect("row is an object");
                    object.insert(
                        "gate".into(),
                        serde_json::to_value(&gate).expect("gate serializes"),
                    );
                    object.insert(
                        "could_seal".into(),
                        Value::Bool(gate.could_seal()),
                    );
                }
                if let Some(gate) = xmap15_gate(&snapshot) {
                    xmap15_hours_with_gate += 1;
                    xmap15_minted_plan_rows += u64::from(gate.minted_plan_rows);
                    xmap15_visual_orders += u64::from(gate.visual_d54_orders);
                    for hour_gate in &gate.hour_gates {
                        *xmap15_gate_counts
                            .entry(hour_gate.recorded_input.clone())
                            .or_default() += 1;
                    }
                    row.as_object_mut().expect("row is an object").insert(
                        "xmap15_gate".into(),
                        serde_json::to_value(&gate).expect("gate serializes"),
                    );
                }
                if sample.is_none() {
                    sample = mint_sample(hour.hour_index, &hour.snapshot);
                }
                per_hour.push(row);
            }
            Some(Err(error)) => panic!("fold after {hours} hours: {error}"),
            None => break,
        }
    }

    let contract_gates = portable_render_stream::xmap13_row_contract_gate_leaves();
    let report = json!({
        "schema": "hoi4.wo.xmap13_gate_scan.v1",
        "spool": args[1],
        "hours": hours,
        "hours_with_carrier_key": hours_with_carrier_key,
        "hours_with_gate": hours_with_gate,
        "hours_with_every_recorded_input": hours_with_no_hour_gate,
        "hour_gate_counts_by_input": hour_gate_counts,
        "minted_totals": {
            "division_rows": minted[0],
            "task_force_rows": minted[1],
            "air_wing_rows": minted[2],
            "formations": minted[3],
        },
        // Constant across every hour and every recording, so it is reported
        // once here rather than per hour: these are the leaves no capture can
        // source, which is why no hour seals however complete it is.
        "contract_gate_leaves": contract_gates,
        "xmap15": {
            "hours_with_gate": xmap15_hours_with_gate,
            "minted_plan_rows": xmap15_minted_plan_rows,
            "visual_d54_orders": xmap15_visual_orders,
            "hour_gate_counts_by_input": xmap15_gate_counts,
            "contract_gate_leaves": portable_render_stream::xmap15_contract_gate_leaves(),
        },
        "minted_sample": sample,
        "per_hour": per_hour,
    });
    let mut file = std::fs::File::create(&args[2]).expect("create out");
    file.write_all(&serde_json::to_vec_pretty(&report).expect("report serializes"))
        .expect("write out");
    println!(
        "{hours} hours; carrier key on {hours_with_carrier_key}; gate on {hours_with_gate}; \
         minted {} division / {} task-force / {} air-wing rows and {} formations",
        minted[0], minted[1], minted[2], minted[3]
    );
}

/// The XMAP15 gate, two levels down from the root the fold writes it at.
fn xmap15_gate(
    snapshot: &BTreeMap<String, &serde_json::value::RawValue>,
) -> Option<portable_render_stream::Xmap15ProducerGate> {
    let order_of_battle: BTreeMap<String, &serde_json::value::RawValue> =
        serde_json::from_str(snapshot.get("order_of_battle")?.get()).ok()?;
    let extra: BTreeMap<String, &serde_json::value::RawValue> =
        serde_json::from_str(order_of_battle.get("extra")?.get()).ok()?;
    let raw = extra.get(portable_render_stream::XMAP15_PRODUCER_GATE_WIRE_PATH[2])?;
    serde_json::from_str(raw.get()).ok()
}

/// Mint from one folded hour and keep three rows of each family's result.
///
/// The counts say how MUCH the producers got; this says WHAT they got, which is
/// the half a count cannot show.
fn mint_sample(hour_index: i64, snapshot: &[u8]) -> Option<Value> {
    let parsed: Value = serde_json::from_slice(snapshot).ok()?;
    let divisions = parsed.get("divisions")?;
    let order_of_battle = parsed.get("order_of_battle")?;
    let theatre = parsed
        .get("map")
        .and_then(|map| map.get("_r2_theatre_selector_live"))
        .cloned()
        .unwrap_or(Value::Null);
    let xmap13_hour = map_unit_presentation::project_fold_sections(
        hour_index,
        divisions,
        order_of_battle,
        &theatre,
        &BTreeMap::new(),
    );
    let xmap13 = map_unit_presentation::produce(&xmap13_hour);
    if xmap13.minted_rows.is_empty() {
        return None;
    }
    let formations: Vec<xmap15_orders_semantic::FormationOwner> = xmap13_hour
        .formations
        .iter()
        .map(|formation| xmap15_orders_semantic::FormationOwner {
            is_army_group: formation.owner_is_army_group,
            owner_id: formation.owner_id,
            owner_country: formation.owner_country.clone(),
        })
        .collect();
    let owner_census: Option<Vec<xmap15_orders_semantic::FormationIdentity>> = xmap13_hour
        .formation_owner_census
        .as_ref()
        .map(|owners| {
            owners
                .iter()
                .map(|owner| xmap15_orders_semantic::FormationIdentity {
                    is_army_group: owner.owner_is_army_group,
                    owner_id: owner.owner_id,
                })
                .collect()
        });
    let xmap15_hour = xmap15_orders_semantic::project_fold_sections(
        hour_index,
        order_of_battle,
        &xmap15_orders_semantic::Xmap13Formations {
            owner_census: owner_census.as_deref(),
            with_countries: &formations,
            gate: xmap13_hour.formations_gate,
        },
    );
    let xmap15 = xmap15_orders_semantic::produce(&xmap15_hour);
    Some(json!({
        "hour_index": hour_index,
        "xmap13_minted_rows": xmap13.minted_rows.len(),
        "xmap13_first_three": xmap13.minted_rows.iter().take(3).collect::<Vec<_>>(),
        "xmap13_minted_formations": xmap13.minted_formations.len(),
        "xmap15_minted_plan_rows": xmap15.minted_plan_rows.len(),
        "xmap15_visual_d54_orders": xmap15_hour.visual_d54_orders,
        "xmap15_first_three": xmap15.minted_plan_rows.iter().take(3).collect::<Vec<_>>(),
    }))
}

fn detect_identity_mode(path: &str) -> replay_decoder::StreamIdentityMode {
    use std::io::Read;
    let mut file = std::fs::File::open(path).expect("open");
    let mut head = [0u8; 40];
    let read = file.read(&mut head).expect("read");
    if read >= 24
        && &head[0..4] == compact_contract::COMPACT_RECORD_MAGIC
        && u16::from_le_bytes([head[22], head[23]])
            == compact_contract::COMPACT_RECORD_TYPE_STREAM_HEADER
    {
        replay_decoder::StreamIdentityMode::IdentityV1
    } else {
        replay_decoder::StreamIdentityMode::Legacy
    }
}
