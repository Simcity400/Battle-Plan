use super::{
    emit::{self, Emit},
    facts::{Country, Hour},
    query,
};
use dashboard_series::{
    Blob, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use serde_json::{Value, json};
use std::{sync::Arc, sync::atomic::AtomicBool};

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [5; 32],
        content: [6; 32],
        branch: [7; 32],
        schema: 1,
    }
}

fn hour(at: i32, named: &[(u32, &str)], valid: Option<&[u32]>, countries: &[Country]) -> Hour {
    Hour {
        branch: 1,
        hour: at,
        named: named
            .iter()
            .map(|&(index, tag)| (index, Arc::from(tag)))
            .collect(),
        valid: valid.map(<[u32]>::to_vec),
        countries: countries.to_vec(),
    }
}

fn country(index: u32, holder: (u32, u32), read: i32, values: [Option<i64>; 3]) -> Country {
    Country {
        index,
        holder,
        read,
        values,
    }
}

/// Builds the publication of `hours`, cut after each of `cuts`; at `resume` the emit is
/// saved and taken up again from the publication, as a restarted preparation goes on.
fn build(hours: Vec<Hour>, cuts: &[i32], resume: Option<i32>) -> (Memory, u64) {
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(identity(), first).unwrap();
    let mut emit = Emit::new();
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
            let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
            emit = Emit::resume(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    (pages, root)
}

fn ask(
    snapshot: &mut Snapshot<'_, Memory>,
    directory: &Directory,
    product: &str,
    params: Value,
) -> Value {
    query::query(
        product,
        params,
        snapshot,
        directory,
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// GER is read at 10, 20 and 30, its stability changing at 20; its holder moves to another
/// id at 30; war support is unknown from 40; it loses its inputs at 45 and comes back at 47;
/// 50 is not recorded. Every state's run, holder segment and read hour as hand-built, across
/// cuts at 24 and 49 and a restart at 24.
#[test]
fn states_follow_reads_holders_unknowns_removals_cuts_and_gaps() {
    let ger = |id, read, stability, war| country(0, (3, id), read, [stability, war, Some(9)]);
    let mut hours = vec![hour(
        10,
        &[(0, "GER"), (1, "ITA")],
        Some(&[0, 1]),
        &[
            ger(1, 10, Some(500), Some(700)),
            country(1, (4, 4), 10, [Some(1), Some(2), Some(3)]),
        ],
    )];
    for at in 11..=55 {
        if at == 50 {
            continue;
        }
        let mut entry = hour(at, &[], None, &[]);
        match at {
            20 => entry.countries.push(ger(1, 20, Some(600), Some(700))),
            // The first hour after the restart starts a segment: its projector names anew.
            25 => {
                entry.named = vec![(0, Arc::from("GER")), (1, Arc::from("ITA"))];
                entry.valid = Some(vec![0, 1]);
                entry.countries = vec![
                    ger(1, 20, Some(600), Some(700)),
                    country(1, (4, 4), 10, [Some(1), Some(2), Some(3)]),
                ];
            }
            30 => entry.countries.push(ger(2, 30, Some(600), Some(700))),
            40 => entry.countries.push(ger(2, 40, Some(600), None)),
            45 => entry.valid = Some(vec![1]),
            47 => {
                entry.valid = Some(vec![0, 1]);
                entry.countries.push(ger(2, 47, Some(600), Some(800)));
            }
            51 => {
                entry.named = vec![(0, Arc::from("ITA")), (1, Arc::from("GER"))];
                entry.valid = Some(vec![0, 1]);
                entry.countries = vec![
                    country(0, (4, 4), 10, [Some(1), Some(2), Some(3)]),
                    country(1, (3, 2), 47, [Some(600), Some(800), Some(9)]),
                ];
            }
            _ => {}
        }
        hours.push(entry);
    }
    let (pages, root) = build(hours, &[24, 49, 55], Some(24));
    let mut snapshot = Snapshot::open(pages, root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
    let family = |measure| json!({ "kind": "country-stability", "measure": measure });
    let reads: Vec<Value> = [10, 19, 20, 29, 30, 44, 45, 47, 49, 50, 51, 55]
        .iter()
        .map(|&hour| json!({ "tag": "GER", "hour": hour }))
        .chain([json!({ "tag": "SOV", "hour": 20 })])
        .collect();
    let reply = ask(
        &mut snapshot,
        &directory,
        "series-state",
        json!({ "branchId": "1", "family": family("stability"), "reads": reads }),
    );
    let states = reply["states"].as_array().unwrap();
    let run = |at: usize| &states[at]["run"];
    // 10..19: 500 from read 10, segment from 10.
    assert_eq!(run(0)["valueRaw"], "500");
    assert_eq!(
        (run(0)["firstHour"].as_i64(), run(0)["lastHour"].as_i64()),
        (Some(10), Some(19))
    );
    assert_eq!(run(0)["startsSegment"], true);
    assert_eq!(run(0)["readHour"], 10);
    assert_eq!(run(1)["readHour"], 10);
    // 20..29: 600, same holder, same segment.
    assert_eq!(run(2)["valueRaw"], "600");
    assert_eq!(run(2)["startsSegment"], false);
    assert_eq!(run(2)["segmentFirstHour"], 10);
    assert_eq!(run(3)["readHour"], 20);
    // The holder's id changes at 30: a new run and segment though the value is equal.
    assert_eq!(run(4)["startsSegment"], true);
    assert_eq!(run(4)["countryId"], 2);
    assert_eq!(run(4)["segmentFirstHour"], 30);
    assert_eq!(run(4)["firstHour"], 30);
    assert_eq!(run(5)["readHour"], 40);
    // No inputs at 45 and 46.
    assert!(run(6).is_null());
    assert_eq!(states[6]["recorded"], true);
    // Back at 47 through 49; 50 is not recorded, and 51 starts again.
    assert_eq!(run(7)["firstHour"], 47);
    assert_eq!(run(7)["lastHour"], 49);
    assert_eq!(run(8)["segmentFirstHour"], 47);
    assert!(run(9).is_null());
    assert_eq!(run(10)["firstHour"], 51);
    assert_eq!(run(10)["lastHour"], 55);
    assert_eq!(run(10)["startsSegment"], true);
    assert_eq!(run(11)["segmentFirstHour"], 51);
    assert_eq!(states[11]["readHour"], 47);
    assert!(run(12).is_null());
    assert_eq!(states[12]["recorded"], false);
    assert_eq!(
        reply["warnings"],
        json!(["SOV: no recorded stability inputs in this branch"])
    );

    let reply = ask(
        &mut snapshot,
        &directory,
        "series-state",
        json!({ "branchId": "1", "family": family("warSupport"), "reads": [
            { "tag": "GER", "hour": 39 }, { "tag": "GER", "hour": 40 }, { "tag": "GER", "hour": 47 },
        ] }),
    );
    let states = reply["states"].as_array().unwrap();
    assert_eq!(states[0]["run"]["valueRaw"], "700");
    assert!(states[1]["run"].is_null());
    assert_eq!(states[1]["readHour"], 40);
    assert_eq!(states[2]["run"]["valueRaw"], "800");
    assert_eq!(states[2]["run"]["startsSegment"], true);

    // An hourly chart from GER's first hour lists its original runs; marks are its holders.
    let reply = ask(
        &mut snapshot,
        &directory,
        "series-summary",
        json!({ "branchId": "1", "family": family("stability"), "tags": ["GER", "SOV"],
            "from": 10, "to": 55, "width": 64, "withMarks": true }),
    );
    let ger = &reply["series"][0];
    let runs: Vec<(i64, i64)> = ger["summary"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| {
            (
                run["firstHour"].as_i64().unwrap(),
                run["lastHour"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(runs, vec![(10, 19), (20, 29), (30, 44), (47, 49), (51, 55)]);
    let marks: u64 = ger["marks"]["clusters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|cluster| cluster["count"].as_str().unwrap().parse::<u64>().unwrap())
        .sum();
    // Holders at 10, 20, 30, 40, 47 and 51.
    assert_eq!(marks, 6);
    assert_eq!(reply["series"][1]["summary"]["buckets"]["slots"], 0);
    assert_eq!(reply["series"][1]["marks"]["clusters"], json!([]));

    let step = |snapshot: &mut Snapshot<'_, Memory>, key: &str, from: f64| {
        ask(
            snapshot,
            &directory,
            "series-next-change",
            json!({ "branchId": "1", "family": family("stability"), "tags": ["GER"],
                "measures": ["stability", "warSupport"], "key": key, "from": from,
                "firstHour": 10, "lastHour": 55 }),
        )["hour"]
            .as_f64()
    };
    assert_eq!(step(&mut snapshot, "ArrowRight", 10.0), Some(20.0));
    assert_eq!(step(&mut snapshot, "ArrowRight", 30.0), Some(40.0));
    assert_eq!(step(&mut snapshot, "ArrowLeft", 47.0), Some(45.0));

    let scoreboard =
        query::cells(&mut snapshot, &directory, &["GER", "SOV"], "stability", 20).unwrap();
    assert_eq!(scoreboard, vec![(Some(600), Some(20)), (None, None)]);
}

/// 1,500 countries over 120 hours, 40 read each hour, cut every 24 hours: every country's
/// stability runs equal the model.
#[test]
fn fifteen_hundred_countries_keep_their_runs() {
    let count = 1500_u32;
    let named: Vec<(u32, String)> = (0..count)
        .map(|index| (index, format!("T{index:04}")))
        .collect();
    let mut values = vec![0_i64; count as usize];
    let mut model: Vec<Vec<(i32, i64)>> = vec![Vec::new(); count as usize];
    let mut hours = Vec::new();
    for at in 0..120 {
        let mut countries = Vec::new();
        let indices: Vec<u32> = if at == 0 {
            (0..count).collect()
        } else {
            let mut indices: Vec<u32> =
                (0..40).map(|n| (at as u32 * 37 + n * 31) % count).collect();
            indices.sort_unstable();
            indices.dedup();
            indices
        };
        for index in indices {
            let value = &mut values[index as usize];
            if at % 3 == 0 {
                *value += 1;
            }
            let runs = &mut model[index as usize];
            if runs.last().is_none_or(|&(_, held)| held != *value) {
                runs.push((at, *value));
            }
            countries.push(country(
                index,
                (index, index),
                at,
                [Some(*value), Some(1), Some(2)],
            ));
        }
        // The hour after the restart at 47 starts a segment, naming every country anew.
        let segment = at == 0 || at == 48;
        hours.push(Hour {
            branch: 1,
            hour: at,
            named: if segment {
                named
                    .iter()
                    .map(|(index, tag)| (*index, Arc::from(tag.as_str())))
                    .collect()
            } else {
                Vec::new()
            },
            valid: segment.then(|| (0..count).collect()),
            countries,
        });
    }
    let cuts: Vec<i32> = (23..120).step_by(24).collect();
    let (pages, root) = build(hours, &cuts, Some(47));
    let mut snapshot = Snapshot::open(pages, root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
    assert_eq!(directory.actors.len(), count as usize);
    for index in (0..count).step_by(97) {
        let tag = format!("T{index:04}");
        let reply = ask(
            &mut snapshot,
            &directory,
            "series-summary",
            json!({ "branchId": "1", "family": { "kind": "country-stability", "measure": "stability" },
                "tags": [tag], "from": 0, "to": 119, "width": 120 }),
        );
        let runs: Vec<(i64, String)> = reply["series"][0]["summary"]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| (run["firstHour"].as_i64().unwrap(), run["value"].to_string()))
            .collect();
        let expected: Vec<(i64, String)> = model[index as usize]
            .iter()
            .map(|&(first, value)| {
                (
                    i64::from(first),
                    json!(value as f64 / 100_000.0).to_string(),
                )
            })
            .collect();
        assert_eq!(runs, expected, "{tag}");
    }
}
