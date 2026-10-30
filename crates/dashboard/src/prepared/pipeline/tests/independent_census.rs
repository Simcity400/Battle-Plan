//! Country factories and nuclear stockpiles prepared on their own, before the
//! hourly walk, from blocks read by their changes: each publishes and saves
//! exactly what hourly preparation does across quiet spans (both tables read
//! every hour, mostly with equal rows), changed counts inside blocks, a
//! country renamed and one gone whose rows stay unresolved, a stockpile
//! table unavailable for two hours, a gap, two branches, append, resume and
//! refusals, with one and several pre-pass workers; its quiet hours build no
//! frame; and factory faction totals, country factories and country
//! stockpiles answer from their families' coverage while the hourly walk
//! has accepted nothing, as hourly preparation answers them.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use dashboard_base::prepared::kept::counted;
use independent::Own;
use replay_data::{
    rows::CountryFactoriesRow, rows::FactionMemberRow, rows::FactionRow, rows::NukeRow,
    rows::TableRows,
};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

const A: u32 = 100_000;
const B: u32 = 100_001;
const C: u32 = 100_002;
const D: u32 = 100_003;

/// Branch 1's last hour.
const LAST: i32 = 216;

/// The census tables at `hour` of `branch` over the four countries A to D
/// (`frame`), whose balances move every hour. Both census tables are read
/// every hour (hourly in the current format), mostly with equal rows.
///
/// A's military factories rise at 110 (inside a block) and 150, B's civilian
/// factories at 125; C is renamed at 140; D's country and link rows are gone
/// from 180 while its factory and stockpile rows stay, unresolved. A's
/// stockpile rises at 120 and 160 (increases the feed reads), B holds a
/// second bomb type from 130 to 190; with `unavailable`, the stockpile table
/// is not read complete at 170 and 171 (a dated read of an incomplete table
/// at an hour whose rows are the hour before's). A and B form a faction, C
/// joins it at 150.
fn census_frame(branch: u64, hour: i32, unavailable: bool) -> DecodedFrame {
    let mut observed = frame(branch, hour, 4);
    // A missing dictionary is retried every hour; this fixture tests quiet
    // numeric acquisitions with a complete, unchanged naming dictionary.
    *observed.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![]));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *observed.tables[TableId::Countries.index()]
    {
        if hour >= 140 {
            rows[2].tag = b"RNM".to_vec();
        }
        if hour >= 180 {
            rows.retain(|row| row.id != D);
        }
    }
    if hour >= 180
        && let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *observed.tables[TableId::CountryLinks.index()]
    {
        rows.retain(|row| row.id != D);
    }
    let military = |country: u32| match country {
        A if hour >= 150 => 15,
        A if hour >= 110 => 12,
        A => 10,
        B => 4,
        C => 3,
        _ => 2,
    };
    let civilian = |country: u32| match country {
        B if hour >= 125 => 7,
        B => 5,
        _ => 1,
    };
    *observed.tables[TableId::CountryFactories.index()] =
        TableView::Complete(TableRows::CountryFactories(
            [A, B, C, D]
                .into_iter()
                .map(|country| CountryFactoriesRow {
                    country,
                    military_total_raw: military(country) * 100_000,
                    civilian_total_raw: civilian(country) * 100_000,
                    ..Default::default()
                })
                .collect(),
        ));
    let mut nukes = vec![NukeRow {
        country: A,
        nuke_type: 0,
        amount_raw: match hour {
            160.. => 4,
            120.. => 2,
            _ => 1,
        } * 100_000_000,
        ..Default::default()
    }];
    if (130..=190).contains(&hour) {
        nukes.push(NukeRow {
            country: B,
            nuke_type: 1,
            amount_raw: 100_000_000,
            ..Default::default()
        });
    }
    nukes.push(NukeRow {
        country: D,
        nuke_type: 0,
        amount_raw: 100_000_000,
        ..Default::default()
    });
    *observed.tables[TableId::Nukes.index()] = if unavailable && (170..=171).contains(&hour) {
        TableView::Unavailable {
            reason: "Stockpiles not read".into(),
        }
    } else {
        TableView::Complete(TableRows::Nukes(nukes))
    };
    *observed.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 42,
            name: b"Recorded pact".to_vec(),
            ..Default::default()
        }]));
    *observed.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(
            [A, B]
                .into_iter()
                .chain((hour >= 150).then_some(C))
                .enumerate()
                .map(|(ordinal, country)| FactionMemberRow {
                    faction: 42,
                    country,
                    ordinal: ordinal as u32,
                })
                .collect(),
        ));
    for rows in [
        TableRows::HumanControllers(Vec::new()),
        TableRows::WarRelations(Vec::new()),
        TableRows::Capitulation(Vec::new()),
    ] {
        let table = rows.table();
        *observed.tables[table.index()] = TableView::Complete(rows);
    }
    observed
}

/// Branch 1's hours from 100 through `last`, without 136 and 137 (a gap
/// between blocks), in blocks of 12, then branch 2's one block (hours 20 to
/// 25) when `second`.
fn scenario(last: i32, second: bool, unavailable: bool) -> Replay {
    let hours: Vec<i32> = (100..=last)
        .filter(|hour| !(136..=137).contains(hour))
        .collect();
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours
            .iter()
            .map(|&hour| census_frame(1, hour, unavailable))
            .collect()
    };
    let mut blocks = hours.chunks(12);
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, frames(blocks.next().unwrap()))],
    )
    .unwrap();
    for block in blocks {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(block))])
            .unwrap();
    }
    if second {
        replay = replay
            .append(&[records(
                2,
                (20..=25)
                    .map(|hour| census_frame(2, hour, unavailable))
                    .collect(),
            )])
            .unwrap();
    }
    replay
}

fn factories() -> &'static dyn Own {
    &independent::census::FACTORIES
}

fn nukes() -> &'static dyn Own {
    &independent::census::NUKES
}

/// Each census family's name and the field hourly preparation keeps its
/// state in.
const CENSUS: [(&str, &str); 2] = [("factories", "factories"), ("nukes", "nukes")];

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// Every block committed on its own.
fn measured(source: &Source) {
    run_measured_through(
        source,
        None,
        &AtomicBool::new(false),
        true,
        Duration::ZERO,
        Some(u64::MAX),
        |_| Ok(()),
        |_| {},
    )
    .unwrap();
}

fn plain(source: &Source) {
    run(source, None, &AtomicBool::new(false), |_| {}).unwrap();
}

/// Prepare a fresh source with `prepare` and take its payloads.
fn prepared(fixture: &mut Fixture, name: &str, prepare: &dyn Fn(&Source)) -> Payloads {
    let source = fixture.source(name);
    prepare(&source);
    let payloads = payloads(&source, None);
    clear(source);
    payloads
}

/// Sections only one mode writes: the checkpoint, the families' own coverage
/// pages and continuations, the hourly branch state (compared apart) and the
/// identity history the walk keeps under territory's name while the
/// identities are prepared on their own.
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name.starts_with("preparation-family-")
        || name.starts_with("preparation-branch-")
        || name.contains("-continuation-")
        || name.starts_with("territory-identities-")
}

/// A continuation or branch state page as JSON.
fn decoded(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
}

/// Every section both modes publish is byte-identical (with `refusals`, but
/// for each branch's coverage page, whose refusals the families keep in
/// their own pages), each census family named in `own` published, the
/// hourly branch state is the same apart from their fields and, without
/// `refusals`, each one's continuation is the field hourly preparation keeps
/// in its branch state.
fn assert_same(actual: &Payloads, hourly: &Payloads, label: &str, own: &[&str], refusals: bool) {
    let shared = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                !(mode_specific(name) || (refusals && name.starts_with("preparation-coverage-")))
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
    for published in [
        "factories-values-1-0",
        "factories-1-catalogue",
        "nukes-values-1",
        "nukes-increases-1",
    ] {
        assert!(
            actual.keys().any(|(name, _)| name == published),
            "{label}: nothing published as {published}"
        );
    }
    let fields: Vec<_> = CENSUS
        .iter()
        .filter(|(family, _)| own.contains(family))
        .collect();
    let mut states = 0;
    for ((name, range), bytes) in hourly {
        let Some(branch) = name.strip_prefix("preparation-branch-") else {
            continue;
        };
        let mut saved = decoded(bytes);
        let mut kept = decoded(
            actual
                .get(&(name.clone(), *range))
                .unwrap_or_else(|| panic!("{label}: no {name}")),
        );
        for (family, field) in &fields {
            let state = saved.as_object_mut().unwrap().remove(*field).unwrap();
            // The walk saves no state of a family it does not run.
            assert_eq!(
                kept.as_object_mut().unwrap().remove(*field),
                None,
                "{label}: branch {branch}'s saved state holds {field}"
            );
            if !refusals {
                let section = (format!("{family}-continuation-{branch}"), 0);
                let continuation = actual
                    .get(&section)
                    .unwrap_or_else(|| panic!("{label}: {family} saved no continuation"));
                assert_eq!(
                    decoded(continuation),
                    state,
                    "{label}: branch {branch}'s {family} continuation"
                );
                states += 1;
            }
        }
        assert_eq!(kept, saved, "{label}: branch {branch}'s hourly state");
    }
    assert!(
        refusals || fields.is_empty() || states > 0,
        "{label}: no continuation"
    );
}

/// Blocks each census observed hour by hour, and folds that fell back.
fn notes() -> [usize; 4] {
    [
        prepass::noted("factories.sequential"),
        prepass::noted("factories.fallback"),
        prepass::noted("nukes.sequential"),
        prepass::noted("nukes.fallback"),
    ]
}

/// Hours recorded in `replay`.
fn hours(replay: &Replay) -> usize {
    let mut hours = 0;
    for index in &replay.index {
        replay
            .read_block_alone(
                index,
                &[TableId::Campaign],
                |_, _| {
                    hours += 1;
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    hours
}

#[test]
fn census_families_prepared_on_their_own_publish_and_save_what_hourly_preparation_does() {
    for unavailable in [false, true] {
        let replay = scenario(LAST, true, unavailable);
        let mut fixture = Fixture::new(&replay);
        let hourly =
            independent::with_test_hourly(|| prepared(&mut fixture, "census-parity", &plain));
        // Alone, together, and with the identity history they are named
        // through prepared on its own too.
        let registries: [Vec<&'static dyn Own>; 4] = [
            vec![factories()],
            vec![nukes()],
            vec![factories(), nukes()],
            vec![factories(), nukes(), &independent::IDENTITIES],
        ];
        for families in registries {
            let own: Vec<&str> = families.iter().map(|own| own.family().name).collect();
            for (workers, together) in [(1, true), (3, false)] {
                let label = format!(
                    "{own:?}, {workers} workers, at once {together}, unavailable {unavailable}"
                );
                let before = notes();
                let payloads = prepass::with_test_prepass(workers, None, || {
                    independent::with_test_families(&families, || {
                        if together {
                            prepared(&mut fixture, "census-parity", &plain)
                        } else {
                            independent::with_test_one_after_another(|| {
                                prepared(&mut fixture, "census-parity", &plain)
                            })
                        }
                    })
                });
                assert_same(&payloads, &hourly, &label, &own, false);
                let after = notes();
                let [
                    factories_sequential,
                    factories_fallback,
                    nukes_sequential,
                    nukes_fallback,
                ] = [0, 1, 2, 3].map(|index| after[index] - before[index]);
                assert_eq!(
                    (factories_sequential, nukes_sequential),
                    (0, 0),
                    "{label}: every block folded from its extract"
                );
                assert_eq!(factories_fallback, 0, "{label}: factories never fall back");
                // A stockpile read at 171 that is not complete names its
                // reason from the frame: unless the pre-pass gives that
                // hour a frame, its block is observed hour by hour from
                // there. No other block falls back.
                let allowed = usize::from(unavailable && own.contains(&"nukes"));
                assert!(nukes_fallback <= allowed, "{label}: stockpile fallbacks");
            }
        }
    }
}

#[test]
fn census_families_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation() {
    let first = scenario(161, false, true);
    let grown = scenario(LAST, true, true);
    // The hourly walk prepares one block per run.
    let resumed = |source: &Source| {
        let flag = AtomicBool::new(false);
        while unprepared_blocks(source, None).unwrap() != Some(0) {
            run_measured(source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
        }
    };
    // Cancelled at the first accepted report, then prepared again; every
    // block commits on its own in both runs.
    let cancelled = |source: &Source| {
        let flag = AtomicBool::new(false);
        match run_measured_through(
            source,
            None,
            &flag,
            true,
            Duration::ZERO,
            Some(u64::MAX),
            |_| Ok(()),
            |_| flag.store(true, Ordering::Release),
        ) {
            Ok(_) => {}
            Err(error) => assert_eq!(error.kind(), io::ErrorKind::Interrupted),
        }
        flag.store(false, Ordering::Release);
        measured(source);
    };
    let modes: [PrepareReloadMode; 5] = [
        ("run", &plain, false),
        ("measured", &measured, false),
        ("measured, reloaded", &measured, true),
        ("resumed", &resumed, false),
        ("cancelled", &cancelled, false),
    ];
    let mut fixture = Fixture::new(&first);
    for (mode, prepare, reload) in modes {
        let both = |fixture: &mut Fixture| -> Payloads {
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
            fixture.replace(&first);
            let source = fixture.source("census-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("census-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let own = prepass::with_test_prepass(2, None, || {
            independent::with_test_families(&[factories(), nukes()], || both(&mut fixture))
        });
        assert_same(&own, &hourly, mode, &["factories", "nukes"], false);
    }
}

/// `family`'s refusal of each branch, from the checkpoint in hourly
/// preparation, or from its own coverage page.
fn refusals_of(source: &Source, family: &str, own: bool) -> BTreeMap<u64, Option<Refusal>> {
    let (store, key) = super::super::super::files(source, None).unwrap();
    let (state, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    [1, 2]
        .into_iter()
        .map(|branch| {
            let refusal = if own {
                store
                    .read(&key, &independent::coverage_section(family, branch), 0)
                    .unwrap()
                    .and_then(|file| {
                        serde_json::from_reader::<_, CoveragePage>(std::io::BufReader::new(file))
                            .unwrap()
                            .refused
                            .get(family)
                            .cloned()
                    })
            } else {
                state
                    .refused
                    .get(&branch)
                    .and_then(|refused| refused.0.get(family).cloned())
            };
            (branch, refusal)
        })
        .collect()
}

#[test]
fn census_families_prepared_on_their_own_refuse_where_hourly_preparation_does() {
    let replay = scenario(LAST, true, true);
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    for (family, query) in [
        ("factories", "country-factories"),
        ("nukes", "country-nukes"),
    ] {
        // At a block's first hour, and at an hour inside a block whose rows
        // are the hour before's.
        for hour in [124, 128] {
            let label = format!("{family} refusing at {hour}");
            let refusals = [(family, hour)];
            let (hourly, hourly_refused) = independent::with_test_hourly(|| {
                lanes::with_test_refusals(&refusals, || {
                    let source = fixture.source("census-refusal");
                    measured(&source);
                    let result = (payloads(&source, None), refusals_of(&source, family, false));
                    clear(source);
                    result
                })
            });
            let source = fixture.source("census-refusal");
            let sequential = prepass::noted(if family == "factories" {
                "factories.sequential"
            } else {
                "nukes.sequential"
            });
            lanes::with_test_refusals(&refusals, || {
                prepass::with_test_prepass(2, None, || {
                    independent::with_test_families(&[factories(), nukes()], || measured(&source))
                })
            });
            assert_eq!(
                prepass::noted(if family == "factories" {
                    "factories.sequential"
                } else {
                    "nukes.sequential"
                }) - sequential,
                1,
                "{label}: the refusing block is observed hour by hour"
            );
            assert_same(
                &payloads(&source, None),
                &hourly,
                &label,
                &["factories", "nukes"],
                true,
            );
            let own_refused = refusals_of(&source, family, true);
            assert_eq!(own_refused, hourly_refused, "{label}");
            assert_eq!(own_refused[&1].as_ref().unwrap().hour, hour, "{label}");
            assert!(own_refused[&2].is_none(), "{label}: branch 2 goes on");
            independent::with_test_families(&[factories(), nukes()], || {
                let error = refusal(&source, None, 1, query, &flag).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
                assert_eq!(
                    error.to_string(),
                    format!("Injected refusal of {family} at hour {hour}"),
                    "{label}"
                );
                refusal(&source, None, 2, query, &flag).unwrap();
                // Faction totals read the factories: they refuse with them.
                if family == "factories" {
                    let error = refusal(&source, None, 1, "faction-totals", &flag).unwrap_err();
                    assert_eq!(
                        error.to_string(),
                        format!("Injected refusal of factories at hour {hour}"),
                        "{label}"
                    );
                }
            });
            clear(source);
        }
    }
}

/// Read by its changes, each census alone builds frames only at the hours a
/// row, key, link or availability it reads changes and at each block's first
/// hour: fewer than half the recorded hours although every hour reads its
/// table, every one kept and observed once on the carried census. Read
/// whole, every hour is a frame. Both publish what hourly preparation does.
#[test]
fn census_quiet_hours_build_no_frame() {
    let replay = scenario(LAST, true, false);
    let recorded = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| prepared(&mut fixture, "census-frames", &plain));
    for (family, own) in [("factories", factories()), ("nukes", nukes())] {
        for whole in [true, false] {
            let label = format!("{family}, read whole {whole}");
            let before = (counted(family), notes());
            let source = fixture.source("census-frames");
            let read = || {
                prepass::with_test_prepass(2, None, || {
                    independent::with_test_families(&[own], || plain(&source))
                })
            };
            if whole {
                prepass::with_test_alone(read);
            } else {
                read();
            }
            let payloads = payloads(&source, None);
            clear(source);
            assert_eq!(notes(), before.1, "{label}: every block folded");
            assert_same(&payloads, &hourly, &label, &[family], false);
            let (observed, frames) = counted(family);
            let (observed, frames) = (observed - before.0.0, frames - before.0.1);
            if whole {
                assert_eq!(frames, recorded, "{label}: a frame every hour");
            } else {
                assert!(
                    frames * 2 < recorded,
                    "{label}: {frames} frames of {recorded} hours"
                );
            }
            assert_eq!(
                observed, frames,
                "{label}: every frame kept and observed once"
            );
        }
    }
}

fn query(source: &Source, product: &str, params: serde_json::Value) -> serde_json::Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply =
        super::super::super::query::query(source, &request, None, &AtomicBool::new(false)).unwrap();
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["data"].take()
}

/// A faction-totals series request of `branch` with the factory or the
/// territory measure.
fn faction_request(branch: &str, factories: bool, extra: serde_json::Value) -> serde_json::Value {
    let source = if factories {
        serde_json::json!({"kind": "factories", "mode": "built", "groups": []})
    } else {
        serde_json::json!({"kind": "territory", "measure": "states", "rule": "owned"})
    };
    let mut params = serde_json::json!({
        "branchId": branch,
        "family": {"kind": "faction-totals", "selection": {
            "branchId": branch, "source": source, "membership": "hour", "top": 5,
        }},
    });
    for (key, value) in extra.as_object().unwrap() {
        params[key] = value.clone();
    }
    params
}

/// A faction-totals reply's values: everything but its coverage, which says
/// how far preparation went, and its warnings, which count the reads the
/// prepared catalogue saw.
fn faction_values(reply: &serde_json::Value) -> serde_json::Value {
    let mut values = reply.clone();
    if let Some(values) = values.as_object_mut() {
        values.remove("coverage");
        values.remove("warnings");
    }
    values
}

/// Product requests, as (product, params).
type Requests = Vec<(&'static str, serde_json::Value)>;

/// The requests of `branch` (hours `first` to `last`) answered from the
/// families' own coverage: factory faction totals at reached hours, in an
/// exact window and the next change, the countries' factories and stockpiles
/// at reached hours. Then those that wait for the hourly walk: territory
/// faction totals and a dense factory summary, which joins territory's
/// member presence.
fn requests(branch: &str, first: i32, last: i32) -> (Requests, Requests) {
    let tag = |index| String::from_utf8(crate::test_fixtures::scale_tag(index)).unwrap();
    let reads = serde_json::json!([
        {"tag": tag(0), "hour": last},
        {"tag": tag(1), "hour": (last - 7).max(first)},
    ]);
    let own = vec![
        (
            "series-state",
            faction_request(branch, true, serde_json::json!({"reads": [{"hour": last}]})),
        ),
        (
            "series-summary",
            faction_request(
                branch,
                true,
                serde_json::json!({"from": (last - 9).max(first), "to": last, "width": 30}),
            ),
        ),
        (
            "series-next-change",
            faction_request(
                branch,
                true,
                serde_json::json!({"key": "ArrowLeft", "from": last, "firstHour": first,
                    "lastHour": last}),
            ),
        ),
        (
            "series-state",
            serde_json::json!({"branchId": branch, "family": {"kind": "country-factories"},
                "reads": reads}),
        ),
        (
            "series-state",
            serde_json::json!({"branchId": branch, "family": {"kind": "country-nukes"},
                "reads": reads}),
        ),
    ];
    let walked = vec![
        (
            "series-state",
            faction_request(
                branch,
                false,
                serde_json::json!({"reads": [{"hour": last}]}),
            ),
        ),
        (
            "series-summary",
            faction_request(
                branch,
                true,
                serde_json::json!({"from": first, "to": last, "width": 3}),
            ),
        ),
    ];
    (own, walked)
}

#[test]
fn faction_factories_and_census_answer_from_their_families_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(LAST, true, true);
    let mut fixture = Fixture::new(&replay);
    let branches = [("1", 100, LAST), ("2", 20, 25)];
    // The replies of hourly preparation, with every block accepted.
    let expected: Vec<Vec<serde_json::Value>> = independent::with_test_hourly(|| {
        let source = fixture.source("census-hourly-replies");
        plain(&source);
        let replies = branches
            .iter()
            .map(|&(branch, first, last)| {
                let (own, walked) = requests(branch, first, last);
                own.into_iter()
                    .chain(walked)
                    .map(|(product, params)| query(&source, product, params))
                    .collect()
            })
            .collect();
        clear(source);
        replies
    });
    for replies in &expected {
        assert!(
            replies.iter().all(|reply| !reply.is_null()),
            "hourly preparation answers every request"
        );
    }
    // The faction rosters and the identity history factory totals read are
    // prepared on their own too.
    independent::with_test_only(&["factories", "nukes", "groups", "identities"], || {
        let source = fixture.source("census-stalled");
        let flag = AtomicBool::new(false);
        let (own, _) = requests("1", 100, LAST);
        // Nothing is prepared yet: the factory faction totals at a reached
        // hour answer from its frames, as the prepared pages will, and their
        // summary and next change wait.
        let (product, params) = own[0].clone();
        let cold = query(&source, product, params);
        assert!(!cold.is_null(), "the reached hour answers from its frames");
        assert!(
            cold["coverage"]["firstHour"]
                .as_i64()
                .is_some_and(|first| cold["coverage"]["lastHour"].as_i64() == first.checked_sub(1))
                && cold["coverage"]["complete"] == false,
            "{cold}"
        );
        assert_eq!(
            faction_values(&cold),
            faction_values(&expected[0][0]),
            "the factory faction totals at the last hour as hourly preparation"
        );
        for (index, (product, params)) in own.into_iter().enumerate().skip(1).take(2) {
            assert!(
                query(&source, product, params).is_null(),
                "{product} {index} waits"
            );
        }
        // The hourly walk stalls: it is cancelled at its first commit.
        let mut commits = 0;
        let error = independent::with_test_one_after_another(|| {
            run_checked(
                &source,
                None,
                &flag,
                |_| {},
                |_| {
                    commits += 1;
                    flag.store(true, Ordering::Release);
                    Ok(())
                },
            )
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(commits, 1);
        flag.store(false, Ordering::Release);
        assert_eq!(
            unprepared_blocks(&source, None).unwrap(),
            Some(replay.index.len()),
            "the hourly walk accepted no block"
        );
        for (position, &(branch, first, last)) in branches.iter().enumerate() {
            let id = branch.parse().unwrap();
            assert!(
                coverage(&source, None, id, &flag).unwrap().is_none(),
                "branch {branch}: the hourly walk covers nothing"
            );
            for product in [
                crate::products::faction_totals::FACTORIES_GATE,
                "country-factories",
                "country-nukes",
            ] {
                assert!(
                    complete(&source, None, id, product, &flag).unwrap(),
                    "branch {branch}: {product} is covered by its families"
                );
            }
            assert!(
                !complete(&source, None, id, "faction-totals", &flag).unwrap(),
                "branch {branch}: territory faction totals wait for the walk"
            );
            let (own, walked) = requests(branch, first, last);
            let count = own.len();
            for (index, (product, params)) in own.into_iter().enumerate() {
                let reply = query(&source, product, params);
                assert!(
                    !reply.is_null(),
                    "branch {branch}: {product} {index} answers"
                );
                assert_eq!(
                    reply, expected[position][index],
                    "branch {branch}: {product} {index} answers as hourly preparation does"
                );
            }
            for (index, (product, params)) in walked.into_iter().enumerate() {
                assert!(
                    query(&source, product, params).is_null(),
                    "branch {branch}: {product} {} waits for the walk",
                    count + index
                );
            }
        }
        // Once the walk has prepared too, the dense and territory requests
        // answer as hourly preparation does.
        plain(&source);
        for (position, &(branch, first, last)) in branches.iter().enumerate() {
            let (own, walked) = requests(branch, first, last);
            for (index, (product, params)) in own.into_iter().chain(walked).enumerate() {
                assert_eq!(
                    query(&source, product, params),
                    expected[position][index],
                    "branch {branch}: {product} {index} after the walk"
                );
            }
        }
        clear(source);
    });
}

/// Rewrite `section`'s coverage page of branch 1 to end at `last_hour`, not
/// complete: a family whose preparation has reached no further.
fn stagger(source: &Source, section: &str, last_hour: i32) {
    let (store, key) = super::super::super::files(source, None).unwrap();
    let mut writer = store.extend(&key).unwrap();
    json(
        &mut writer,
        section,
        &CoveragePage {
            coverage: Coverage {
                schema: SCHEMA,
                first_hour: 100,
                last_hour,
                complete: false,
            },
            refused: Default::default(),
        },
    )
    .unwrap();
    writer.commit().unwrap();
}

/// Whether each run of every series begins after the one before it ends.
fn ordered(series: &serde_json::Value, field: &str) -> bool {
    series.as_array().unwrap().iter().all(|series| {
        series[field].as_array().is_none_or(|runs| {
            runs.windows(2)
                .all(|pair| pair[0]["lastHour"].as_i64() < pair[1]["firstHour"].as_i64())
        })
    })
}

/// A family prepared on its own ahead of another its queries read with: the
/// factories and stockpiles prepared through the branch while the identity
/// history (or, for faction totals, the faction rosters) covers less. The
/// queries read the census pages only through the last batch committed by
/// the joint coverage and the hours after it as an unprepared tail: no run
/// or supplying read overlaps another, and the replies are those of hourly
/// preparation whose coverage ends there with its pages ahead of it. A
/// faction total past that batch waits.
#[test]
fn census_queries_read_only_through_their_joint_coverage_when_one_family_is_ahead() {
    let replay = scenario(LAST, true, false);
    let mut fixture = Fixture::new(&replay);
    let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
    let census = |kind: &str| {
        serde_json::json!({"branchId": "1", "family": {"kind": kind}, "tags": [tag],
            "from": 140, "to": LAST, "width": 200})
    };
    // At a block's end (149, where every family commits) and inside the next
    // block (155: the census pages committed by then end at 149).
    for (behind, reached) in [(149, 149), (155, 149)] {
        let label = format!("covered through {behind}");
        // Hourly preparation whose coverage ends there, its pages ahead.
        let expected: Vec<serde_json::Value> = independent::with_test_hourly(|| {
            let source = fixture.source("census-staggered-hourly");
            measured(&source);
            stagger(&source, &coverage_section(1), behind);
            let replies = ["country-factories", "country-nukes"]
                .map(|kind| query(&source, "series-summary", census(kind)))
                .to_vec();
            clear(source);
            replies
        });
        independent::with_test_only(&["factories", "nukes", "identities"], || {
            let source = fixture.source("census-staggered");
            measured(&source);
            stagger(
                &source,
                &independent::coverage_section("identities", 1),
                behind,
            );
            for (index, kind) in ["country-factories", "country-nukes"]
                .into_iter()
                .enumerate()
            {
                let reply = query(&source, "series-summary", census(kind));
                assert_eq!(reply["coverage"]["lastHour"], reached, "{label}: {kind}");
                assert!(
                    ordered(&reply["series"], "runs"),
                    "{label}: {kind} runs overlap"
                );
                assert!(
                    ordered(&reply["series"], "reads"),
                    "{label}: {kind} reads overlap"
                );
                assert_eq!(
                    reply, expected[index],
                    "{label}: {kind} as hourly preparation"
                );
            }
            clear(source);
        });
        // Factory faction totals with the faction rosters behind.
        let state =
            |hour: i32| faction_request("1", true, serde_json::json!({"reads": [{"hour": hour}]}));
        let (complete, at_behind) = independent::with_test_hourly(|| {
            let source = fixture.source("census-staggered-hourly");
            measured(&source);
            let replies = (
                query(&source, "series-state", state(reached)),
                query(&source, "series-state", state(behind)),
            );
            clear(source);
            replies
        });
        independent::with_test_only(&["factories", "groups", "identities"], || {
            let source = fixture.source("census-staggered");
            measured(&source);
            stagger(&source, &independent::coverage_section("groups", 1), behind);
            let mut reached_reply = query(&source, "series-state", state(reached));
            assert_eq!(
                reached_reply["coverage"],
                serde_json::json!({
                    "schema":SCHEMA,"firstHour":100,"lastHour":behind,"complete":false
                })
            );
            let mut complete = complete.clone();
            assert_eq!(complete["coverage"]["complete"], true);
            reached_reply.as_object_mut().unwrap().remove("coverage");
            complete.as_object_mut().unwrap().remove("coverage");
            assert_eq!(
                reached_reply, complete,
                "{label}: faction totals at {reached}"
            );
            // Past the committed factories, the reached hour answers from its
            // frames as the prepared pages will.
            let past = query(&source, "series-state", state(behind));
            assert!(!past.is_null(), "{label}: faction totals at {behind}");
            assert_eq!(
                faction_values(&past),
                faction_values(&at_behind),
                "{label}: faction totals at {behind} as hourly preparation"
            );
            // A summary past them still waits.
            let summary = faction_request(
                "1",
                true,
                serde_json::json!({"from": 140, "to": behind, "width": 30}),
            );
            assert_eq!(
                query(&source, "series-summary", summary).is_null(),
                behind > reached,
                "{label}: a faction summary past the committed factories waits"
            );
            clear(source);
        });
    }
}

/// The factory faction totals at every reached hour of the branch, answered
/// from the hour's frames before anything is prepared, are those the
/// prepared pages give there: the counted members, their sums and segment
/// breaks at faction changes (C joins at 150), factory changes inside a
/// block (110) and at a block's first hour, a rename (140), a country gone
/// whose rows stay unresolved (180) and the hour after the gap. A request
/// with member parts, or under end or ever membership, still waits.
#[test]
fn faction_factory_totals_at_a_reached_hour_answer_from_its_frames_as_prepared() {
    let replay = scenario(LAST, false, false);
    let mut fixture = Fixture::new(&replay);
    let hours: Vec<i32> = replay
        .index
        .iter()
        .filter(|block| block.branch == 1)
        .flat_map(|block| [block.first_hour, block.first_hour + 1, block.last_hour])
        .chain([110, 125, 140, 150, 180, LAST])
        .collect();
    let state = |hour: i32, extra: serde_json::Value| {
        let mut request =
            faction_request("1", true, serde_json::json!({"reads": [{"hour": hour}]}));
        for (key, value) in extra.as_object().unwrap() {
            request["family"]["selection"][key] = value.clone();
        }
        request
    };
    let expected: Vec<serde_json::Value> = independent::with_test_hourly(|| {
        let source = fixture.source("factions-sampled-hourly");
        plain(&source);
        let replies = hours
            .iter()
            .map(|&hour| query(&source, "series-state", state(hour, serde_json::json!({}))))
            .collect();
        clear(source);
        replies
    });
    independent::with_test_only(&["factories", "groups", "identities"], || {
        let source = fixture.source("factions-sampled");
        for (&hour, expected) in hours.iter().zip(&expected) {
            assert!(!expected.is_null(), "hour {hour} is prepared");
            let cold = query(&source, "series-state", state(hour, serde_json::json!({})));
            let reads = dashboard_base::prepared::query::frames::reads(&source);
            let observations = dashboard_base::prepared::query::observations::usage(
                &source,
                "faction-totals:state",
            )
            .2;
            assert_eq!(
                query(&source, "series-state", state(hour, serde_json::json!({}))),
                cold
            );
            assert_eq!(
                dashboard_base::prepared::query::frames::reads(&source),
                reads
            );
            assert_eq!(
                dashboard_base::prepared::query::observations::usage(
                    &source,
                    "faction-totals:state"
                )
                .2,
                observations
            );
            assert_eq!(
                faction_values(&cold),
                faction_values(expected),
                "hour {hour}"
            );
        }
        for extra in [
            serde_json::json!({"parts": true, "factionId": "42"}),
            serde_json::json!({"membership": "end"}),
            serde_json::json!({"membership": "ever"}),
        ] {
            assert!(
                query(&source, "series-state", state(LAST, extra.clone())).is_null(),
                "{extra} waits"
            );
        }
        clear(source);
    });
}

fn repair_frame(hour: i32) -> DecodedFrame {
    use engine_executable::modifier_keys as keys;
    use replay_data::rows::{
        CountryProductionModifiersRow, ModifierBreakdownNode, NavalRepairCountryRow,
        ProductionModifierTree,
    };
    let mut observed = frame(1, hour, 2);
    // Every other table the family watches is recorded, as in a recording,
    // and holds nothing: only the use and output inputs change.
    for (table, _) in crate::products::dockyard_repairs::SELECTION_FIELDS {
        if *table != TableId::CountryLinks {
            *observed.tables[table.index()] = TableView::Complete(TableRows::empty(*table));
        }
    }
    for table in [
        TableId::CountryAppearance,
        TableId::LexicalTokens,
        TableId::VariantDefinitions,
        TableId::VariantModules,
        TableId::VariantUpgrades,
        TableId::VariantChanges,
    ] {
        *observed.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    *observed.tables[TableId::NavalRepairCountries.index()] =
        TableView::Complete(TableRows::NavalRepairCountries(vec![
            NavalRepairCountryRow {
                country: A,
                num_used_dockyards: if hour < 12 { 2 } else { 3 },
                max_factories_for_repair: i32::from(hour >= 24),
                ..Default::default()
            },
            NavalRepairCountryRow {
                country: B,
                num_used_dockyards: 0,
                ..Default::default()
            },
        ]));
    // A complete synthetic request catalogue, independent of installed content.
    let trees = [
        keys::LINE_CHANGE_PRODUCTION_EFFICIENCY_FACTOR,
        keys::OUT_OF_POWER_IMPACT_FACTOR,
        keys::PRODUCTION_FACTORY_MAX_EFFICIENCY_FACTOR,
        keys::PRODUCTION_FACTORY_EFFICIENCY_GAIN_FACTOR,
        keys::PRODUCTION_FACTORY_START_EFFICIENCY_FACTOR,
        keys::INDUSTRIAL_CAPACITY_FACTORY,
        keys::INDUSTRIAL_CAPACITY_FACTORY_POWERED,
        keys::INDUSTRIAL_CAPACITY_DOCKYARD,
        keys::INDUSTRIAL_CAPACITY_DOCKYARD_POWERED,
        keys::EQUIPMENT_CONVERSION_SPEED,
        keys::REFIT_SPEED,
    ]
    .into_iter()
    .map(|key| ProductionModifierTree {
        key,
        nodes: vec![ModifierBreakdownNode {
            children: 0,
            name: b"Synthetic source".to_vec(),
            value: if hour >= 56 && key == keys::INDUSTRIAL_CAPACITY_DOCKYARD {
                20_000
            } else {
                0
            },
        }],
        failure: Vec::new(),
    })
    .collect();
    *observed.tables[TableId::CountryProductionModifiers.index()] = if (32..56).contains(&hour) {
        TableView::Unavailable {
            reason: "Synthetic daily repair-output refusal".into(),
        }
    } else {
        TableView::Complete(TableRows::CountryProductionModifiers(if hour < 8 {
            Vec::new()
        } else {
            vec![CountryProductionModifiersRow {
                country: A,
                power_raw: if hour >= 56 { 50_000 } else { 100_000 },
                factory_speed: [200_000, 400_000, 600_000, 1_000_000],
                trees,
                ..Default::default()
            }]
        }))
    };
    observed.metrics.table_ns[TableId::CountryProductionModifiers.index()] =
        u64::from([8, 32, 56].contains(&hour));
    if hour >= 65 {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *observed.tables[TableId::Countries.index()]
        {
            rows[0].id = C;
            rows.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *observed.tables[TableId::CountryLinks.index()]
        {
            rows.push(replay_data::rows::CountryLinkRow {
                id: C,
                canonical_slot: 0,
            });
        }
    }
    observed
}

fn repair_recording(last: i32) -> Replay {
    repair_recording_with(last, repair_frame)
}

fn repair_recording_with(last: i32, observed: fn(i32) -> DecodedFrame) -> Replay {
    let hours: Vec<_> = (0..=last).filter(|hour| !(41..44).contains(hour)).collect();
    let mut blocks = hours.chunks(13);
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            blocks
                .next()
                .unwrap()
                .iter()
                .copied()
                .map(observed)
                .collect(),
        )],
    )
    .unwrap();
    for block in blocks {
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                block.iter().copied().map(observed).collect(),
            )])
            .unwrap();
    }
    replay
}

#[test]
fn dockyard_repairs_fold_matches_hourly_through_daily_reads_gaps_holders_resume_and_append() {
    let first = repair_recording(38);
    let grown = repair_recording(76);
    let mut fixture = Fixture::new(&first);
    for workers in [1, 3] {
        let prepare = |fixture: &mut Fixture| {
            fixture.replace(&first);
            let source = fixture.source("repair-census");
            measured(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("repair-census");
            measured(&source);
            let payload = payloads(&source, None);
            let (store, key) = super::super::super::files(&source, None).unwrap();
            let flag = AtomicBool::new(false);
            let mut reads =
                crate::products::dockyard_repairs::RepairReads::new(&store, &key, 1, &flag)
                    .unwrap();
            let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
            assert_eq!(reads.origin(&tag).unwrap(), Some(8));
            use crate::products::equipment::production::ProductionSpendingUnknown as Unknown;
            assert_eq!(
                reads
                    .integrals(
                        &tag,
                        &[
                            (0, 7),
                            (8, 11),
                            (12, 31),
                            (32, 40),
                            (40, 44),
                            (56, 64),
                            (64, 65)
                        ]
                    )
                    .unwrap(),
                [
                    Err(Unknown::SpendingUnavailable),
                    Ok(8_000_000),
                    Ok(60_000_000),
                    Err(Unknown::SpendingUnavailable),
                    Err(Unknown::RecordingGap),
                    Ok(23_760_000),
                    Err(Unknown::CountryChanged)
                ]
            );
            drop(reads);
            clear(source);
            payload
        };
        let hourly = independent::with_test_hourly(|| prepare(&mut fixture));
        let before = (
            counted("repairs"),
            prepass::noted("repairs.fallback"),
            prepass::noted("repairs.sequential"),
        );
        let own = prepass::with_test_prepass(workers, None, || {
            independent::with_test_families(&[&independent::census::REPAIRS], || {
                prepare(&mut fixture)
            })
        });
        let pages = |payload: &Payloads| -> Payloads {
            payload
                .iter()
                .filter(|((name, _), _)| name.starts_with("dockyard-repairs-"))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        };
        assert_payloads_equal(&pages(&own), &pages(&hourly), "repairs hourly/fold");
        let hourly_state = decoded(&hourly[&("preparation-branch-1".into(), 0)]);
        assert_eq!(
            decoded(&own[&("repairs-continuation-1".into(), 0)]),
            hourly_state["repairs"]
        );
        assert_eq!(prepass::noted("repairs.fallback"), before.1);
        assert_eq!(prepass::noted("repairs.sequential"), before.2);
        let counts = counted("repairs");
        assert!(
            counts.1 - before.0.1 < 76 / 2,
            "quiet repair spans should build no frames: {counts:?}"
        );
        assert_eq!(counts.0 - before.0.0, counts.1 - before.0.1);
    }
}

#[test]
fn dockyard_repairs_joint_gate_and_refusals_name_both_suppliers() {
    assert_eq!(
        plan::current()
            .joint("dockyard-spending")
            .map(|suppliers| suppliers.iter().map(|id| id.name()).collect::<Vec<_>>()),
        Some(vec!["production", "repairs"])
    );
    assert_eq!(independent::gating("dockyard-spending"), Some("repairs"));
    let replay = repair_recording(12);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("repairs-refusal");
    independent::with_test_families(&[&independent::census::REPAIRS], || {
        lanes::with_test_refusals(&[("repairs", 8)], || measured(&source))
    });
    independent::with_test_families(&[&independent::census::REPAIRS], || {
        let error = refusal(
            &source,
            None,
            1,
            "dockyard-spending",
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "Injected refusal of repairs at hour 8");
    });
    assert!(
        sections(State::Preparing, 0, None)
            .iter()
            .any(|section| section.id == "dockyard-repairs")
    );
}

#[test]
fn dockyard_repairs_equal_off_slot_modifier_reads_start_observation() {
    for timed in [
        TableId::CountryProductionModifiers,
        TableId::ProductionLines,
    ] {
        let mut initial = repair_frame(8);
        initial.hour = 0;
        initial.metrics.table_ns[TableId::CountryProductionModifiers.index()] = 0;
        let frames: Vec<_> = (0..4)
            .map(|hour| {
                let mut observed = initial.clone();
                observed.hour = hour;
                observed.metrics.table_ns[timed.index()] = u64::from(hour == 2);
                observed
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let mut fixture = Fixture::new(&replay);
        let prepare = |fixture: &mut Fixture| {
            let source = fixture.source("repair-off-slot");
            plain(&source);
            let (store, key) = super::super::super::files(&source, None).unwrap();
            let flag = AtomicBool::new(false);
            let mut reads =
                crate::products::dockyard_repairs::RepairReads::new(&store, &key, 1, &flag)
                    .unwrap();
            let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
            assert_eq!(reads.origin(&tag).unwrap(), Some(2));
            assert_eq!(reads.integrals(&tag, &[(2, 3)]).unwrap(), [Ok(4_000_000)]);
            let result = payloads(&source, None);
            drop(reads);
            clear(source);
            result
                .into_iter()
                .filter(|((name, _), _)| name.starts_with("dockyard-repairs-"))
                .collect::<Payloads>()
        };
        let hourly = independent::with_test_hourly(|| prepare(&mut fixture));
        let own = prepass::with_test_prepass(1, None, || {
            independent::with_test_families(&[&independent::census::REPAIRS], || {
                prepare(&mut fixture)
            })
        });
        assert_payloads_equal(&own, &hourly, "equal off-slot repair output read");
    }
}

fn repair_split_frame(hour: i32) -> DecodedFrame {
    use replay_data::rows::*;
    let mut observed = repair_frame(hour);
    let mut put = |rows: TableRows| {
        let index = rows.table().index();
        observed.tables[index] = TableView::Complete(rows).into();
    };
    put(TableRows::NavalBases(vec![NavalBaseRow {
        province: 7,
        country: A,
        level: 10,
        ..Default::default()
    }]));
    // The base's assignment always reconciles with the country's use. Before
    // hour 12 the second ship is whole, so one assigned dockyard finds no
    // eligible ship (an allocation residual); from hour 12 it is damaged and
    // selected twice.
    put(TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
        province: 7,
        allocated_dockyards: if hour < 12 { 2 } else { 3 },
        ..Default::default()
    }]));
    put(TableRows::NavalRepairQueue(
        [1, 2, 2]
            .into_iter()
            .enumerate()
            .map(|(ordinal, ship)| NavalRepairQueueRow {
                province: 7,
                ordinal: ordinal as u32,
                ship,
            })
            .collect(),
    ));
    put(TableRows::Ships(
        (1..=2)
            .map(|id| ShipRow {
                id,
                task_force: 8,
                strength: if id == 2 && hour < 12 { 100 } else { 50 },
                ..Default::default()
            })
            .collect(),
    ));
    put(TableRows::ShipModels(
        (1..=2)
            .map(|ship| ShipModelRow {
                ship,
                variant: if ship == 1 { 20 } else { 30 },
                counter: Some(ShipCounterInputs {
                    maximum_strength: 100,
                    ..Default::default()
                }),
                has_criticals: Some(0_u32).into(),
                ..Default::default()
            })
            .collect(),
    ));
    put(TableRows::TaskForces(vec![TaskForceRow {
        id: 8,
        province: 7,
        roster_owner: A,
        ..Default::default()
    }]));
    put(TableRows::Provinces(vec![ProvinceRow {
        id: 7,
        controller: A,
        ..Default::default()
    }]));
    put(TableRows::VariantDefinitions(
        [20, 30, 99]
            .into_iter()
            .map(|id| VariantDefinitionRow {
                id,
                creator_link: A,
                equipment_token: 1,
                display_name: format!("Read at {}", if hour < 46 { 22 } else { 46 }).into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    put(TableRows::LexicalTokens(vec![LexicalTokenRow {
        token: 1,
        literal: b"synthetic_ship".to_vec(),
    }]));
    put(TableRows::VariantModules(vec![]));
    put(TableRows::VariantUpgrades(vec![]));
    put(TableRows::VariantChanges(vec![]));
    observed.metrics.table_ns[TableId::VariantDefinitions.index()] =
        u64::from(hour == 22 || hour == 46);
    observed
}

#[test]
fn repair_split_fold_matches_hourly_dated_catalogues_selection_changes_gaps_and_resume() {
    use crate::products::dockyard_repairs::{RepairPart, RepairReads};
    let first = repair_recording_with(38, repair_split_frame);
    let grown = repair_recording_with(58, repair_split_frame);
    let mut fixture = Fixture::new(&first);
    for workers in [1, 3] {
        let prepare = |fixture: &mut Fixture| {
            fixture.replace(&first);
            drop({
                let source = fixture.source("repair-split");
                measured(&source);
                source
            });
            fixture.replace(&grown);
            let source = fixture.source("repair-split");
            measured(&source);
            let (store, key) = super::super::super::files(&source, None).unwrap();
            let flag = AtomicBool::new(false);
            let mut reads = RepairReads::new(&store, &key, 1, &flag).unwrap();
            let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
            assert_eq!(reads.origin(&tag).unwrap(), Some(8));
            assert_eq!(
                reads.parts(&tag).unwrap(),
                [
                    RepairPart::Unattributed,
                    RepairPart::Design(20),
                    RepairPart::Design(30)
                ]
            );
            assert_eq!(
                reads
                    .variants()
                    .versions
                    .keys()
                    .copied()
                    .collect::<Vec<_>>(),
                [20, 30]
            );
            assert_eq!(reads.variants().versions[&20][0].first_hour, 22);
            assert_eq!(
                reads
                    .part_integrals(&tag, RepairPart::Design(30), &[(8, 11)])
                    .unwrap(),
                [Ok(0)]
            );
            for part in reads.parts(&tag).unwrap() {
                assert_eq!(reads.part_integrals(&tag, part, &[(0, 7)]).unwrap(),
                    [Err(crate::products::equipment::production::ProductionSpendingUnknown::SpendingUnavailable)]);
            }
            let spans = [(8, 11), (12, 31), (56, 58)];
            let mut sums = vec![0; spans.len()];
            for part in reads.parts(&tag).unwrap() {
                for (sum, value) in sums
                    .iter_mut()
                    .zip(reads.part_integrals(&tag, part, &spans).unwrap())
                {
                    *sum += value.unwrap();
                }
            }
            assert_eq!(
                reads.integrals(&tag, &spans).unwrap(),
                sums.into_iter().map(Ok).collect::<Vec<_>>()
            );
            let payload = payloads(&source, None);
            drop(reads);
            clear(source);
            payload
        };
        let hourly = independent::with_test_hourly(|| prepare(&mut fixture));
        let own = prepass::with_test_prepass(workers, None, || {
            independent::with_test_families(&[&independent::census::REPAIRS], || {
                prepare(&mut fixture)
            })
        });
        let pages = |payload: &Payloads| {
            payload
                .iter()
                .filter(|((name, _), _)| name.starts_with("dockyard-repairs-"))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Payloads>()
        };
        assert_payloads_equal(&pages(&own), &pages(&hourly), "repair split hourly/fold");
        let hourly_state = decoded(&hourly[&("preparation-branch-1".into(), 0)]);
        assert_eq!(
            decoded(&own[&("repairs-continuation-1".into(), 0)]),
            hourly_state["repairs"]
        );
    }
}
