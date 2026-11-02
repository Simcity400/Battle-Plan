use super::{LandSource, LossCause, LossCountry, LossesParams};
use dashboard_base::prepared::served::{self, Ask, Holder, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const SOURCES: [LandSource; 2] = [LandSource::Destruction, LandSource::CombatLog];
const NAVIGATION: [&str; 4] = ["Home", "End", "ArrowLeft", "ArrowRight"];
const WIDTH: u32 = 64;

fn selection(branch: &str, holder: &Holder, land_source: LandSource) -> LossesParams {
    LossesParams {
        branch_id: branch.to_owned(),
        tag: holder.tag.clone(),
        land_source,
        country: Some(LossCountry {
            slot: holder.slot,
            country_id: holder.country_id,
            origin_hour: holder.origin_hour,
        }),
    }
}

fn losses(
    selection: &LossesParams,
    (from, to, width): (i32, i32, u32),
    mut reads: Vec<i32>,
    (head, detail): (i32, i32),
) -> Value {
    reads.extend([head, detail]);
    reads.sort_unstable();
    reads.dedup();
    json!({
        "selection": selection,
        "from": from, "to": to, "width": width, "reads": reads,
        "headHour": head, "detailHour": detail,
    })
}

fn leaves(reply: &Value) -> io::Result<Vec<Value>> {
    let causes: Vec<LossCause> = serde_json::from_value(reply["causes"].clone())?;
    let mut leaves = Vec::new();
    for cause in causes
        .into_iter()
        .filter(|cause| cause.loss && cause.unknown.is_none())
    {
        for leaf in cause.leaves {
            let variant = match leaf.design {
                Some(design) => reply["designs"][design as usize]["variant"]
                    .as_u64()
                    .and_then(|variant| u32::try_from(variant).ok())
                    .map(Value::from)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "Loss design has no variant")
                    })?,
                None => Value::Null,
            };
            leaves.push(json!([cause.source, variant]));
        }
    }
    Ok(leaves)
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let holders = ask.holders(branch.id)?;
        let hours = branch.hours();
        let spans: Vec<_> = branch
            .spans()
            .into_iter()
            .filter(|&(from, to)| first <= from && to <= last)
            .collect();
        let per_holder = 1 + 2 * spans.len() + hours.len() + 2 * NAVIGATION.len();
        ask.request_allowance(
            branch.id,
            SOURCES.len() * (holders.len() + holders.len().min(served::KEYS) * per_holder),
        )?;
        for source in SOURCES {
            let mut active = Vec::new();
            for holder in &holders {
                let reply = ask.ask(
                    "equipment-losses",
                    losses(
                        &selection(&id, holder, source),
                        (first, last, WIDTH),
                        vec![first, last],
                        (last, last),
                    ),
                )?;
                if reply.as_ref().is_some_and(|reply| {
                    served::values_at(reply, &["causes", "*"])
                        .iter()
                        .any(|cause| {
                            cause["loss"] == true
                                && (source != LandSource::CombatLog
                                    || cause["source"]["kind"] == "combatLog")
                                && cause["leaves"]
                                    .as_array()
                                    .is_some_and(|leaves| !leaves.is_empty())
                        })
                }) {
                    active.push(holder.clone());
                }
            }
            let detailed = if active.is_empty() { &holders } else { &active };
            for holder in served::spread(detailed, served::KEYS) {
                let selection = selection(&id, &holder, source);
                let reply = ask.ask(
                    "equipment-losses",
                    losses(
                        &selection,
                        (first, last, WIDTH),
                        vec![first, branch.middle(), last],
                        (last, first),
                    ),
                )?;
                for &(from, to) in &spans {
                    let detail = (i64::from(from) + (i64::from(to) - i64::from(from)) / 2) as i32;
                    let mut reads = vec![from, to];
                    if let Some(baseline) = from.checked_sub(1) {
                        reads.push(baseline);
                    }
                    for viewport in [(from, to, WIDTH), (to, to, 1)] {
                        ask.ask(
                            "equipment-losses",
                            losses(&selection, viewport, reads.clone(), (to, detail)),
                        )?;
                    }
                }
                for (at, &hour) in hours.iter().enumerate() {
                    let viewport = if at % 2 == 0 {
                        (hour, hour, 1)
                    } else {
                        (first, last, WIDTH)
                    };
                    ask.ask(
                        "equipment-losses",
                        losses(
                            &selection,
                            viewport,
                            vec![first, last],
                            (hour, branch.middle()),
                        ),
                    )?;
                }
                let Some(reply) = reply.filter(|reply| !reply.is_null()) else {
                    continue;
                };
                let all_leaves = leaves(&reply)?;
                let mut selections = vec![all_leaves.clone()];
                if all_leaves.len() > 1 {
                    selections.push(served::spread(&all_leaves, 1));
                }
                for leaves in selections {
                    for key in NAVIGATION {
                        let from = match key {
                            "ArrowLeft" | "ArrowRight" => Some(branch.middle()),
                            _ => None,
                        };
                        ask.ask(
                            "series-next-change",
                            json!({
                                "branchId": id, "family": { "kind": "equipment-losses", "selection": selection },
                                "leaves": leaves, "key": key, "from": from,
                                "firstHour": first, "lastHour": last,
                            }),
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}
