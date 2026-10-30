//! Real queries answer from the one publication they pinned, as its decoded gates hold it: a
//! series, by any family's reader, stops before its suppliers' first hole and answers before a
//! refusal, no query reads publication metadata, nor any reader the manifest, a family of
//! another version is pending, and a query whose files a later publication removed starts again
//! on that one. Each store folder's publication is decoded as it was committed, whichever key
//! binds the folder. Every product's request, point and series alike, answers for the hours it
//! names, before any refusal they do not reach. A changed walk version prepares only the walk and
//! the families depending on it again.
use super::*;
use crate::prepared::driver::{
    plan,
    publication::{Family, Hours, Made, Publisher},
    registry::{self, Descriptor},
};
use crate::{
    prepared_stability::StabilityAccumulator,
    test_fixtures::{stability_frame, stability_inputs},
};
use dashboard_base::prepared::{gates, roots::Binding};
use serde_json::{Value, json};
use std::{cell::Cell, collections::BTreeSet, io::Read, path::Path, rc::Rc, sync::Mutex};

/// GER's stability from hour 0 to 29, another value every hour, in blocks of ten hours.
fn frames() -> Vec<DecodedFrame> {
    (0..30)
        .map(|hour| {
            let mut inputs = stability_inputs();
            inputs.stability_raw = 50_000 + i64::from(hour) * 100;
            stability_frame(hour, inputs, hour, vec![])
        })
        .collect()
}

fn recording(frames: &[DecodedFrame]) -> Replay {
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..10].to_vec())]).unwrap();
    for block in frames[10..].chunks(10) {
        replay = replay
            .append(&[continuation_records(&replay, 1, block.to_vec())])
            .unwrap();
    }
    replay
}

/// Writes the stability pages of `frames` and the stability family's coverage of them.
fn stability_pages(writer: &mut store::Writer, frames: &[DecodedFrame], complete: bool) {
    let mut accumulator = StabilityAccumulator::default();
    for frame in frames {
        accumulator.observe(frame).unwrap();
    }
    accumulator
        .flush(writer, 1, &AtomicBool::new(false))
        .unwrap();
    json(
        writer,
        &independent::coverage_section("stability", 1),
        &Coverage {
            schema: SCHEMA,
            first_hour: frames[0].hour,
            last_hour: frames[frames.len() - 1].hour,
            complete,
        },
    )
    .unwrap();
}

/// Publishes the stability pages of `frames` alone, replacing whatever `store` published.
fn publish_pages(store: &store::Store, key: &store::SourceKey, frames: &[DecodedFrame]) {
    let mut writer = store.replace(key.clone()).unwrap();
    stability_pages(&mut writer, frames, frames.len() == 30);
    writer.commit().unwrap();
}

/// Publishes `family`'s own root over its pages, ten hours a segment from hour `start`, each made
/// as `segments` says.
fn publish_root(
    store: &store::Store,
    key: &store::SourceKey,
    family: &str,
    start: i32,
    segments: &[Made],
) {
    let family = Family::planned(plan::rows(), &[family]).unwrap()[0];
    let (publisher, _) = Publisher::open(
        store,
        key.clone(),
        Binding::new(b"publication gates", b"no content"),
        plan::rows(),
        &[family],
        &|_, _, _| true,
    )
    .unwrap();
    let mut stage = publisher.stage(family.id).unwrap();
    for (index, made) in (0_u32..).zip(segments) {
        let first = index * 10;
        let hours = Hours {
            branch: 1,
            run: 0,
            first,
            last: first + 9,
            first_hour: start + first as i32,
            last_hour: start + first as i32 + 9,
            ends_segment: true,
            ends_branch: false,
            proof: [index as u8; 32],
        };
        stage.processed(hours, made.clone(), None, None).unwrap();
        stage.submit().unwrap();
    }
}

/// The whole reply to `product`, through the query path the app's widgets use.
fn reply(source: &Source, product: &str, params: Value) -> io::Result<Value> {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    crate::prepared::query::query(source, &request, None, &AtomicBool::new(false))
        .map(|reply| serde_json::from_str(&reply).unwrap())
}

fn summary(from: i32, to: i32) -> Value {
    json!({"branchId":"1","family":{"kind":"country-stability","measure":"stability"},
        "tags":["GER"],"from":from,"to":to,"width":100})
}

fn states(hours: &[i32]) -> Value {
    let reads: Vec<_> = hours
        .iter()
        .map(|hour| json!({"tag":"GER","hour":hour}))
        .collect();
    json!({"branchId":"1","family":{"kind":"country-stability","measure":"stability"},
        "reads":reads})
}

/// The extents of GER's runs in a summary reply.
fn runs(reply: &Value) -> Vec<(i64, i64)> {
    reply["data"]["series"][0]["summary"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| {
            (
                run["firstHour"].as_i64().unwrap(),
                run["lastHour"].as_i64().unwrap(),
            )
        })
        .collect()
}

fn lost() -> Made {
    Made::Lost {
        reason: "A damaged unit".into(),
        ends: false,
    }
}

/// A fixture whose stability family prepared hours 0 to 29 as `segments` say.
fn published(name: &str, segments: &[Made]) -> (Fixture, Arc<Source>) {
    assert_eq!(
        plan::current().gating("country-stability"),
        Some("stability")
    );
    let frames = frames();
    let mut fixture = Fixture::new(&recording(&frames));
    let source = fixture.source(name);
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    publish_pages(&store, &key, &frames);
    publish_root(&store, &key, "stability", 0, segments);
    (fixture, source)
}

#[test]
fn a_series_history_stops_before_its_supplier_s_first_hole() {
    let (_fixture, source) = published("hole", &[Made::Prepared, lost(), Made::Prepared]);
    let across = reply(&source, "series-summary", summary(0, 29)).unwrap();
    assert_eq!(
        across["holes"],
        json!([{"firstHour":10,"lastHour":19,"family":"stability","unavailable":"lost",
            "reason":"A damaged unit"}])
    );
    let coverage = &across["data"]["coverage"];
    assert_eq!(
        (&coverage["lastHour"], &coverage["complete"]),
        (&json!(9), &json!(false))
    );
    let drawn = runs(&across);
    assert_eq!(drawn.last().map(|run| run.1), Some(9));
    assert!(
        drawn.iter().all(|&(_, last)| last < 10),
        "nothing drawn from the hole on: {drawn:?}"
    );
    // Hours before the hole answer whole; a state across it reads only its hours before it.
    let before = reply(&source, "series-summary", summary(0, 9)).unwrap();
    assert!(before.get("holes").is_none(), "{before}");
    assert_eq!(runs(&before), drawn);
    let read = reply(&source, "series-state", states(&[5, 25])).unwrap();
    let hours: Vec<_> = read["data"]["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|state| state["hour"].clone())
        .collect();
    assert_eq!(hours, [json!(5)]);
    assert_eq!(read["holes"][0]["firstHour"], 10);
}

#[test]
fn a_series_state_before_a_refusal_answers_and_one_across_it_refuses() {
    let refused = Made::Refused {
        hour: 20,
        reason: "Refused for the test".into(),
        from: None,
    };
    let (_fixture, source) = published("refusal", &[Made::Prepared, Made::Prepared, refused]);
    let before = reply(&source, "series-state", states(&[15])).unwrap();
    assert_eq!(before["data"]["states"][0]["readHour"], 15, "{before}");
    for across in [&[25][..], &[15, 25][..]] {
        let error = reply(&source, "series-state", states(across)).unwrap_err();
        assert_eq!(error.to_string(), "Refused for the test");
    }
    let drawn = reply(&source, "series-summary", summary(0, 19)).unwrap();
    assert_eq!(runs(&drawn).last().map(|run| run.1), Some(19));
    assert!(reply(&source, "series-summary", summary(0, 29)).is_err());
}

#[test]
fn queries_read_their_gates_from_memory_also_right_after_a_publication() {
    let frames = frames();
    let mut fixture = Fixture::new(&recording(&frames));
    let source = fixture.source("metadata");
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    publish_pages(&store, &key, &frames[..10]);
    let read = |params| {
        let before = store::counters::now();
        let answered = reply(&source, "series-summary", params).unwrap();
        (answered, store::counters::now() - before)
    };
    let (first, work) = read(summary(0, 29));
    assert_eq!(first["data"]["coverage"]["lastHour"], 9);
    assert_eq!((work.manifest_reads, work.stats), (0, 0), "{work:?}");
    // The committing thread decodes the publication it makes, before its next writer can remove
    // anything an earlier one read.
    let held = gates::current(&store, &key).unwrap();
    publish_pages(&store, &key, &frames);
    let current = gates::current(&store, &key).unwrap();
    assert!(!Arc::ptr_eq(&held, &current));
    let (second, work) = read(summary(0, 29));
    assert_eq!(second["data"]["coverage"]["complete"], true);
    assert_eq!((work.manifest_reads, work.stats), (0, 0), "{work:?}");
    // Asked again, the answer and every identity keying it come from memory alone.
    let (again, work) = read(summary(0, 29));
    assert_eq!(again["data"], second["data"]);
    assert_eq!(work, store::counters::Counters::default());
}

#[test]
fn a_query_whose_files_a_later_publication_removed_starts_again_on_that_one() {
    let frames = frames();
    let mut fixture = Fixture::new(&recording(&frames));
    let source = fixture.source("restart");
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    publish_pages(&store, &key, &frames[..10]);
    let attempts = Rc::new(Cell::new(0));
    let mut later = Some({
        let (store, key) = (Arc::clone(&store), key.clone());
        move || {
            // Preparation publishes on its own thread, and its next writer removes the files of
            // the publication the query pinned.
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        publish_pages(&store, &key, &frames);
                        drop(store.extend(&key).unwrap());
                    })
                    .join()
                    .unwrap();
            });
        }
    });
    let counted = Rc::clone(&attempts);
    let hook: Box<dyn FnMut()> = Box::new(move || {
        counted.set(counted.get() + 1);
        if let Some(later) = later.take() {
            later();
        }
    });
    crate::prepared::query::ON_PINNED.with(|on| *on.borrow_mut() = Some(hook));
    let answered = reply(&source, "series-summary", summary(0, 29));
    crate::prepared::query::ON_PINNED.with(|on| on.borrow_mut().take());
    let answered = answered.unwrap();
    assert_eq!(attempts.get(), 2, "the query started again once");
    let coverage = &answered["data"]["coverage"];
    assert_eq!(
        (&coverage["lastHour"], &coverage["complete"]),
        (&json!(29), &json!(true))
    );
    assert_eq!(runs(&answered).last().map(|run| run.1), Some(29));
}

/// `name`'s registry entry one version on.
fn bumped(name: &str) -> &'static Descriptor {
    let family = registry::family_named(name).unwrap();
    Box::leak(Box::new(Descriptor {
        version: family.version + 1,
        ..*family
    }))
}

/// Prepares GER's stability recording through the production pipeline.
fn prepared(name: &str) -> (Fixture, Arc<Source>) {
    let frames = frames();
    let mut fixture = Fixture::new(&recording(&frames));
    let source = fixture.source(name);
    run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
    (fixture, source)
}

#[test]
fn a_family_published_under_another_version_is_pending_through_every_gate() {
    let (_fixture, source) = prepared("versions");
    let flag = AtomicBool::new(false);
    let bumped = plan::test_plan(&[bumped("stability")], &independent::row_names()).unwrap();
    assert_eq!(bumped.stamp("groups"), plan::rows().stamp("groups"));
    assert_eq!(bumped.gating("country-groups"), Some("groups"));
    plan::with_test_plan(bumped, || {
        let pending = reply(&source, "series-summary", summary(0, 29)).unwrap();
        assert_eq!(pending["gate"], json!(["stability"]), "{pending}");
        let coverage = &pending["data"]["coverage"];
        assert_eq!(
            coverage["lastHour"].as_i64(),
            coverage["firstHour"].as_i64().map(|hour| hour - 1),
            "nothing of the old version answers: {pending}"
        );
        let (ranges, short) = crate::prepared::coverage::waiting(|| {
            crate::prepared::coverage::ranges(&source, None, 1, "country-stability", None, &flag)
                .unwrap()
        });
        assert_eq!(ranges, Default::default());
        assert_eq!(short.into_iter().collect::<Vec<_>>(), ["stability"]);
        assert!(
            product_coverage(&source, None, 1, "country-groups", &flag)
                .unwrap()
                .is_some(),
            "a family whose version holds answers"
        );
    });
    let current = product_coverage(&source, None, 1, "country-stability", &flag).unwrap();
    assert!(current.is_some_and(|coverage| coverage.complete));
}

/// Every range of every section `family` owns, as `store` publishes them.
fn owned(
    store: &store::Store,
    key: &store::SourceKey,
    family: &str,
) -> Vec<(String, Vec<Vec<u8>>)> {
    let snapshot = store.current().unwrap().unwrap();
    assert!(snapshot.key() == key);
    snapshot
        .section_names()
        .filter(|section| {
            matches!(
                plan::rows().owner(section),
                Some(plan::Owner::Family(id)) if id.name() == family
            )
        })
        .map(|section| {
            let ranges = (0..snapshot.range_count(section).unwrap())
                .map(|range| {
                    let mut bytes = Vec::new();
                    snapshot
                        .read(section, range)
                        .unwrap()
                        .unwrap()
                        .read_to_end(&mut bytes)
                        .unwrap();
                    bytes
                })
                .collect();
            (section.to_owned(), ranges)
        })
        .collect()
}

#[test]
fn a_walk_version_change_prepares_the_walk_and_its_dependents_again_alone() {
    let (_fixture, source) = prepared("walk-version");
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let own = independent::row_names();
    let army = plan::test_plan(&[bumped("army")], &own).unwrap();
    assert_ne!(army.walk_stamp(), plan::rows().walk_stamp());
    let moved: BTreeSet<&str> = own
        .iter()
        .copied()
        .filter(|name| army.stamp(name) != plan::rows().stamp(name))
        .collect();
    assert!(!moved.contains("stability"));
    let kept = owned(&store, &key, "stability");
    assert!(!kept.is_empty());
    let republished = Arc::new(Mutex::new(BTreeSet::new()));
    let seen = Arc::clone(&republished);
    plan::with_test_plan(army, || {
        independent::with_test_on_publish(
            move |family, _| {
                seen.lock().unwrap().insert(family.to_owned());
            },
            || run(&source, None, &AtomicBool::new(false), |_| {}),
        )
        .unwrap();
    });
    let republished = republished.lock().unwrap();
    assert!(
        republished
            .iter()
            .all(|family| moved.contains(family.as_str())),
        "only families whose stamp moved publish again: {republished:?}, moved {moved:?}"
    );
    assert_eq!(owned(&store, &key, "stability"), kept);
    let current = gates::current(&store, &key).unwrap();
    let versions = current.versions().unwrap().unwrap();
    assert_eq!(versions.walk, army.walk_stamp(), "the walk published again");
}

#[test]
fn a_publication_under_a_grown_recording_s_key_is_decoded_as_it_was_committed() {
    let frames = frames();
    let mut fixture = Fixture::new(&recording(&frames[..10]));
    let first = fixture.source("first");
    let (store, key) = crate::prepared::files(&first, None).unwrap();
    publish_pages(&store, &key, &frames[..10]);
    let read = |source: &Source| reply(source, "series-summary", summary(0, 29)).unwrap();
    assert_eq!(read(&first)["data"]["coverage"]["lastHour"], 9);
    // Binding the grown recording checks its key against the shorter one's manifest, which
    // refuses it until its own first commit.
    fixture.replace(&recording(&frames));
    let grown = fixture.source("grown");
    let (store, key) = crate::prepared::files(&grown, None).unwrap();
    assert!(!store.is_reusable(&key).unwrap());
    let cold = read(&grown);
    let coverage = &cold["data"]["coverage"];
    assert_eq!(
        coverage["lastHour"].as_i64(),
        coverage["firstHour"].as_i64().map(|hour| hour - 1),
        "nothing is published to the grown recording yet: {cold}"
    );
    // Its one commit, with none after it.
    publish_pages(&store, &key, &frames);
    let answered = read(&grown);
    assert!(answered.get("gate").is_none(), "{answered}");
    let coverage = &answered["data"]["coverage"];
    assert_eq!(
        (&coverage["lastHour"], &coverage["complete"]),
        (&json!(29), &json!(true))
    );
    assert_eq!(runs(&answered).last().map(|run| run.1), Some(29));
}

/// A source of the recording at `path`.
fn open(path: &Path, name: &str) -> Arc<Source> {
    Arc::new(
        Source::open(
            crate::SourceKey {
                source_id: name.into(),
                revision: crate::protocol::replay_revision(path).unwrap(),
            },
            path.to_path_buf(),
            &AtomicBool::new(false),
        )
        .unwrap(),
    )
}

#[test]
fn binding_another_key_of_a_store_leaves_its_publication_current() {
    let frames = frames();
    let replay = recording(&frames);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("published");
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    publish_pages(&store, &key, &frames);
    let complete = |source: &Source| {
        let answered = reply(source, "series-summary", summary(0, 29)).unwrap();
        answered["data"]["coverage"]["complete"].clone()
    };
    assert_eq!(complete(&source), true);
    // A copy of the recording beside it is the same campaign in the same folder: another key of
    // the same store.
    let path = fixture.root.join("copy.battleplan");
    std::fs::write(&path, replay.bytes()).unwrap();
    let copy = open(&path, "copy");
    let (other, other_key) = crate::prepared::files(&copy, None).unwrap();
    assert_eq!(other.folder(), store.folder());
    assert!(other_key != key);
    assert_eq!(complete(&copy), false, "nothing is published to the copy");
    assert_eq!(
        complete(&source),
        true,
        "the store's publication stays current for its own key"
    );
}

#[test]
fn no_reader_reads_the_manifest_of_a_store_publishing_nothing_to_its_key() {
    use crate::products::peace_deals::{prepared, test_fixtures as peace};
    let frames = vec![
        peace::frame(100, 3, false, &[9]),
        peace::frame(101, 3, true, &[]),
    ];
    let replay = peace::replay(frames.clone());
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("unpublished");
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let read = |to: i32| {
        let page = json!({"branchId":"1","family":{"kind":"peace-deals"},"to":to});
        let before = store::counters::now();
        let answered = reply(&source, "events-page", page).unwrap();
        assert_eq!(answered["product"], "events-page");
        store::counters::now() - before
    };
    // Before the first commit, its readers read an empty publication.
    let work = read(100);
    assert_eq!((work.manifest_reads, work.stats), (0, 0), "{work:?}");
    let flag = AtomicBool::new(false);
    let mut observer = prepared::Prepared::new(&replay, 1);
    observer.observe(&frames[0]).unwrap();
    let mut writer = store.replace(key.clone()).unwrap();
    observer.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let work = read(101);
    assert_eq!((work.manifest_reads, work.stats), (0, 0), "{work:?}");
    // After a Clear, as before the first commit.
    store.clear().unwrap();
    let work = read(99);
    assert_eq!((work.manifest_reads, work.stats), (0, 0), "{work:?}");
}

/// A country's fuel from hour 0 to 29, another amount every hour.
fn fuel_frames() -> Vec<DecodedFrame> {
    (0..30)
        .map(|hour| {
            let mut frame = crate::test_fixtures::scale_frame(1, hour, 1);
            frame.metrics.table_ns[TableId::CountryFuel.index()] = 1;
            *frame.tables[TableId::CountryFuel.index()] =
                TableView::Complete(TableRows::CountryFuel(vec![
                    replay_data::rows::CountryFuelRow {
                        country: 100_000,
                        current_raw: 1_000 + i64::from(hour),
                        maximum_raw: 32768,
                        ..Default::default()
                    },
                ]));
            frame
        })
        .collect()
}

/// A fixture whose fuel family prepared hours 0 to 29 as `segments` say, and its country's tag.
fn fuel_published(name: &str, segments: &[Made]) -> (Fixture, Arc<Source>, String) {
    assert_eq!(plan::current().gating("country-fuel"), Some("fuel"));
    let frames = fuel_frames();
    let replay = recording(&frames);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source(name);
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let flag = AtomicBool::new(false);
    let mut fuel = crate::products::country_fuel::prepared::Prepared::default();
    let mut identities = crate::products::country_history::HistoryScan::new(false, &[]);
    for frame in &frames {
        fuel.observe(frame, replay.format, &flag).unwrap();
        identities.observe(frame);
    }
    let mut writer = store.replace(key.clone()).unwrap();
    fuel.flush(&mut writer, 1, replay.format, &flag).unwrap();
    json(&mut writer, &gates::identities_section(1), &identities).unwrap();
    writer.commit().unwrap();
    publish_root(&store, &key, "fuel", 0, segments);
    let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
    (fixture, source, tag)
}

fn fuel_states(tag: &str, hours: &[i32]) -> Value {
    let reads: Vec<_> = hours
        .iter()
        .map(|hour| json!({"tag":tag,"hour":hour}))
        .collect();
    json!({"branchId":"1","family":{"kind":"country-fuel"},"reads":reads})
}

#[test]
fn a_family_s_own_series_reader_answers_before_a_refusal_and_stops_before_a_hole() {
    let refused = Made::Refused {
        hour: 20,
        reason: "Refused for the test".into(),
        from: None,
    };
    let (_fixture, source, tag) =
        fuel_published("fuel-refusal", &[Made::Prepared, Made::Prepared, refused]);
    let before = reply(&source, "series-state", fuel_states(&tag, &[15])).unwrap();
    assert_eq!(before["data"]["states"][0]["hour"], 15, "{before}");
    for across in [&[25][..], &[15, 25][..]] {
        let error = reply(&source, "series-state", fuel_states(&tag, across)).unwrap_err();
        assert_eq!(error.to_string(), "Refused for the test");
    }

    let (_fixture, source, tag) =
        fuel_published("fuel-hole", &[Made::Prepared, lost(), Made::Prepared]);
    let read = reply(&source, "series-state", fuel_states(&tag, &[5, 25])).unwrap();
    assert_eq!(read["holes"][0]["family"], "fuel", "{read}");
    assert_eq!(read["holes"][0]["firstHour"], 10);
    let states = read["data"]["states"].as_array().unwrap();
    assert_eq!(
        states
            .iter()
            .map(|state| state["hour"].clone())
            .collect::<Vec<_>>(),
        [json!(5)],
        "no hour from the hole on is read, sampled or not"
    );
    let drawn = reply(
        &source,
        "series-summary",
        json!({"branchId":"1","family":{"kind":"country-fuel"},"tags":[tag],"from":0,"to":29,
            "width":30}),
    )
    .unwrap();
    assert_eq!(drawn["data"]["coverage"]["lastHour"], 9, "{drawn}");
    let line = &drawn["data"]["series"][0]["summary"];
    for spans in ["runs", "approximate"] {
        assert!(
            line[spans]
                .as_array()
                .unwrap()
                .iter()
                .all(|span| span["lastHour"].as_f64().is_some_and(|hour| hour < 10.0)),
            "{spans} from the hole on: {drawn}"
        );
    }
}

/// Publishes `family`'s refusal of branch 1 from `hour` in the coverage page its queries read:
/// its own while it is prepared on its own, the hourly walk's otherwise; that page covers
/// `coverage` when none is published yet.
fn refuse(
    store: &store::Store,
    key: &store::SourceKey,
    family: &str,
    hour: i32,
    coverage: Coverage,
) {
    use dashboard_base::prepared::coverage::{Page, Refusal};
    let section = if independent::registered(family) {
        independent::coverage_section(family, 1)
    } else {
        coverage_section(1)
    };
    let mut page = match store.read(key, &section, 0).unwrap() {
        Some(mut range) => {
            let mut bytes = Vec::new();
            range.read_to_end(&mut bytes).unwrap();
            Page::parse(&bytes).unwrap()
        }
        None => Page {
            coverage,
            refused: Default::default(),
        },
    };
    page.refused.insert(
        family.to_owned(),
        Refusal {
            hour,
            error: "Refused for the test".into(),
            from: None,
        },
    );
    let mut writer = match store.saved_key().unwrap() {
        Some(saved) if saved == *key => store.extend(key).unwrap(),
        _ => store.replace(key.clone()).unwrap(),
    };
    json(&mut writer, &section, &page).unwrap();
    writer.commit().unwrap();
}

/// A subject of GER's whose autonomy rises every hour.
fn autonomy_frame(hour: i32) -> DecodedFrame {
    let mut frame = frame(1, hour, 2);
    frame.tables[TableId::Subjects.index()] =
        TableView::Complete(TableRows::Subjects(vec![replay_data::rows::SubjectRow {
            overlord: 100_000,
            subject: 100_001,
            autonomy_ratio: i64::from(hour),
            ..Default::default()
        }]))
        .into();
    frame.tables[TableId::Autonomy.index()] =
        TableView::Complete(TableRows::Autonomy(vec![replay_data::rows::AutonomyRow {
            subject: 100_001,
            state: b"mod/low".to_vec(),
            score: i64::from(hour) * 100_000 + 1,
            lower_score: 0,
            upper_score: 10_000_000,
            ..Default::default()
        }]))
        .into();
    frame
}

#[test]
fn autonomy_before_its_refusal_answers_and_across_it_refuses() {
    let frames: Vec<_> = (10..30).map(autonomy_frame).collect();
    let mut fixture = Fixture::new(&recording(&frames));
    let source = fixture.source("autonomy-refusal");
    run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let nothing = Coverage {
        schema: SCHEMA,
        first_hour: 10,
        last_hour: 10,
        complete: false,
    };
    refuse(&store, &key, "autonomy", 20, nothing);
    let state = |hours: &[i32]| {
        let reads: Vec<_> = hours.iter().map(|hour| json!({"hour":hour})).collect();
        reply(
            &source,
            "series-state",
            json!({"branchId":"1","family":{"kind":"autonomy"},"subjects":[],"overlords":[],
                "reads":reads}),
        )
    };
    let before = state(&[15]).unwrap();
    assert!(before["data"].is_object(), "{before}");
    for across in [&[25][..], &[15, 25][..]] {
        let error = state(across).unwrap_err();
        assert_eq!(error.to_string(), "Refused for the test");
    }
}

#[test]
fn a_point_query_before_its_family_s_refusal_answers_and_across_it_refuses() {
    let frames = frames();
    let mut fixture = Fixture::new(&recording(&frames));
    let source = fixture.source("point-refusal");
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let prepared = Coverage {
        schema: SCHEMA,
        first_hour: 0,
        last_hour: 29,
        complete: true,
    };
    refuse(&store, &key, "focus", 20, prepared);
    let unavailable = |hour: i32| {
        reply(
            &source,
            "focus-unavailable",
            json!({"branchId":"1","hour":hour}),
        )
    };
    let before = unavailable(15).unwrap();
    assert_eq!(before["product"], "focus-unavailable", "{before}");
    let error = unavailable(25).unwrap_err();
    assert_eq!(error.to_string(), "Refused for the test");
}

#[test]
fn special_project_history_ends_where_its_admitted_prefix_does() {
    use super::independent_evaluated::{BASE, projects_at};
    let frames: Vec<_> = (0..30).map(|at| projects_at(1, BASE + at)).collect();
    let mut fixture = Fixture::new(&recording(&frames));
    let source = fixture.source("projects-hole");
    run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    publish_root(
        &store,
        &key,
        "projects",
        BASE,
        &[Made::Prepared, lost(), Made::Prepared],
    );
    let hole = i64::from(BASE + 10);
    // Current cards at an hour before the hole, and the history alone: neither reads an episode
    // from the hole on.
    for hour in [Some(BASE + 5), None] {
        let mut params = json!({"branchId":"1","tag":"GER"});
        if let Some(hour) = hour {
            params["hour"] = json!(hour);
        }
        let answered = reply(&source, "special-projects", params).unwrap();
        assert_eq!(answered["holes"][0]["firstHour"], hole, "{answered}");
        let data = &answered["data"];
        let read: Vec<_> = data["readHours"]
            .as_array()
            .unwrap()
            .iter()
            .map(|hour| hour.as_i64().unwrap())
            .collect();
        assert!(!read.is_empty(), "{answered}");
        assert!(read.iter().all(|&hour| hour < hole), "{read:?}");
        assert!(
            data["available"]
                .as_array()
                .unwrap()
                .iter()
                .all(|span| span["lastHour"].as_i64().is_some_and(|last| last < hole)),
            "{answered}"
        );
    }
}

/// GER's stability recording, with the intelligence family's own publication lost at 10–19.
fn intelligence_hole(name: &str) -> (Fixture, Arc<Source>) {
    let mut fixture = Fixture::new(&recording(&frames()));
    let source = fixture.source(name);
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    publish_root(
        &store,
        &key,
        "intelligence",
        0,
        &[Made::Prepared, lost(), Made::Prepared],
    );
    (fixture, source)
}

#[test]
fn espionage_requests_keep_only_the_fields_their_type_has() {
    let (_fixture, source) = intelligence_hole("espionage-hole");
    // Its first day, 0 to 23, reaches the hole: it answers nothing, before any reader runs.
    let day = reply(&source, "espionage-log", json!({"branchId":"1","window":0})).unwrap();
    assert!(day["data"].is_null(), "{day}");
    assert_eq!(day["holes"][0]["firstHour"], 10, "{day}");
    // The manifest goes to its reader as its type names it, with no field added.
    if let Err(error) = reply(&source, "espionage-log", json!({"branchId":"1"})) {
        assert!(!error.to_string().contains("unknown field"), "{error}");
    }
}

#[test]
fn an_intelligence_range_from_its_first_hole_on_answers_nothing_before_dispatch() {
    let (_fixture, source) = intelligence_hole("intelligence-hole");
    for product in ["decryption", "operative-placements", "operatives-roster"] {
        let mut params = json!({"branchId":"1","firstHour":25,"lastHour":29});
        if product == "operatives-roster" {
            params["tags"] = json!([]);
        }
        let answered = reply(&source, product, params).unwrap();
        assert!(answered["data"].is_null(), "{product}: {answered}");
        assert_eq!(answered["holes"][0]["firstHour"], 10, "{product}");
    }
}
