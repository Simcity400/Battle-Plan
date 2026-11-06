//! Real queries answer from the one publication they pinned, as its decoded gates hold it: a
//! series, by any family's reader (a segment family's from its root), stops before its suppliers'
//! first hole and answers before a refusal, no query reads publication metadata, nor any reader the manifest, a family of
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
use crate::test_fixtures::{stability_frame, stability_inputs};
use dashboard_base::prepared::{
    gates,
    roots::{self, Binding},
};
use dashboard_forces::products::{army_size, supply_health};
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

/// The stability family's facts of GER at `hour`: its stability another value every hour, read
/// at that hour; a segment's first hour names it again.
fn stability_hour(hour: i32, first: bool) -> crate::products::country_stability::facts::Hour {
    use crate::products::country_stability::facts::{Country, Hour};
    Hour {
        branch: 1,
        hour,
        named: if first {
            vec![(0, Arc::from("GER"))]
        } else {
            Vec::new()
        },
        valid: first.then(|| vec![0]),
        countries: vec![Country {
            index: 0,
            holder: (1, 1),
            read: hour,
            values: [Some(50_000 + i64::from(hour) * 100), Some(0), Some(0)],
        }],
    }
}

/// Publishes the stability family's root, ten hours a segment from hour 0, each made as
/// `segments` says: its emit writes the hours each prepared into the root, as the segment driver
/// does, a refused segment's hours before its refusal. Each segment is a batch; one `queued` is
/// handed off uncommitted as the next begins, as a lane leaves it while no sealer takes it.
fn publish_stability(
    store: &store::Store,
    key: &store::SourceKey,
    segments: &[Made],
    queued: bool,
) {
    let family = Family::planned(plan::rows(), &["stability"]).unwrap()[0];
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
    let mut emit = crate::products::country_stability::emit::Emit::new();
    for (index, made) in (0_u32..).zip(segments) {
        let first = index * 10;
        let (first_hour, last_hour) = (first as i32, first as i32 + 9);
        let emitted = match made {
            Made::Prepared => Some(last_hour),
            Made::Refused { hour, .. } => (*hour > first_hour).then(|| hour - 1),
            _ => None,
        };
        if let Some(emitted) = emitted {
            let writer = stage.series(1, 0).unwrap();
            for hour in first_hour..=emitted {
                emit.hour(writer, stability_hour(hour, hour == first_hour))
                    .unwrap();
            }
            emit.cut(writer, emitted).unwrap();
        }
        let mut state = Vec::new();
        emit.save(&mut state).unwrap();
        let hours = Hours {
            branch: 1,
            run: 0,
            first,
            last: first + 9,
            first_hour,
            last_hour,
            ends_segment: true,
            ends_branch: false,
            proof: [index as u8; 32],
        };
        stage
            .processed(hours, made.clone(), Some(state), None)
            .unwrap();
        if queued {
            stage.send().unwrap();
        } else {
            stage.submit().unwrap();
        }
    }
    stage.settle().unwrap();
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
    published_as(name, segments, false)
}

/// [`published`], each batch `queued` behind the next as it begins.
fn published_as(name: &str, segments: &[Made], queued: bool) -> (Fixture, Arc<Source>) {
    assert_eq!(
        plan::current().gating("country-stability"),
        Some("stability")
    );
    let mut fixture = Fixture::new(&recording(&frames()));
    let source = fixture.source(name);
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    publish_stability(&store, &key, segments, queued);
    (fixture, source)
}

/// Publishes the stability family's root over `segments` alone, replacing whatever `store`
/// published.
fn republish(store: &store::Store, key: &store::SourceKey, segments: &[Made]) {
    store.clear().unwrap();
    publish_stability(store, key, segments, false);
}

/// A family's batch goes on from the batch its lane queued before, though that batch was not yet
/// committed as the next began: the series its emit writes into resumes that batch's root, whose
/// keys are the ones the emit holds, and every reply equals one of batches committed one by one.
#[test]
fn a_batch_goes_on_from_the_batch_queued_before_it() {
    let segments = [Made::Prepared, Made::Prepared, Made::Prepared];
    let (_committed, one_by_one) = published_as("committed", &segments, false);
    let (_queued, behind) = published_as("queued", &segments, true);
    for (product, params) in [
        ("series-summary", summary(0, 29)),
        ("series-state", states(&[5, 15, 25])),
    ] {
        let replies = [&one_by_one, &behind].map(|source| {
            let mut reply = reply(source, product, params.clone()).unwrap();
            reply["data"].take()
        });
        assert_eq!(replies[0], replies[1], "{product}");
    }
    let reply = reply(&behind, "series-summary", summary(0, 29)).unwrap();
    assert_eq!(runs(&reply).last().map(|run| run.1), Some(29));
}

#[test]
fn a_series_history_goes_on_past_a_lost_run_and_answers_nothing_within_it() {
    let (_fixture, source) = published("hole", &[Made::Prepared, lost(), Made::Prepared]);
    let across = reply(&source, "series-summary", summary(0, 29)).unwrap();
    assert_eq!(
        across["holes"],
        json!([{"firstHour":10,"lastHour":19,"family":"stability","unavailable":"lost",
            "reason":"A damaged unit","gap":true}])
    );
    // Hours before the run answer whole.
    let before = reply(&source, "series-summary", summary(0, 9)).unwrap();
    assert!(before.get("holes").is_none(), "{before}");
    // A state reads each hour from its own stretch, and none within the run.
    let read = reply(&source, "series-state", states(&[5, 15, 25])).unwrap();
    let hours: Vec<_> = read["data"]["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|state| state["hour"].clone())
        .collect();
    assert_eq!(hours, [json!(5), json!(25)]);
    assert_eq!(read["holes"][0]["firstHour"], 10);
    let within = reply(&source, "series-state", states(&[12, 15])).unwrap();
    assert!(within["data"].is_null(), "{within}");
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
    let mut fixture = Fixture::new(&recording(&frames()));
    let source = fixture.source("metadata");
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    republish(&store, &key, &[Made::Prepared]);
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
    republish(&store, &key, &[Made::Prepared, Made::Prepared, Made::Prepared]);
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
    let mut fixture = Fixture::new(&recording(&frames()));
    let source = fixture.source("restart");
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    republish(&store, &key, &[Made::Prepared]);
    let attempts = Rc::new(Cell::new(0));
    let mut later = Some({
        let (store, key) = (Arc::clone(&store), key.clone());
        move || {
            // Preparation publishes on its own thread, and its next writer removes the files of
            // the publication the query pinned.
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        republish(&store, &key, &[Made::Prepared, Made::Prepared, Made::Prepared]);
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

/// Prepares the countries' recording through the production pipeline.
fn prepared(name: &str) -> (Fixture, Arc<Source>) {
    let frames = country_frames();
    let mut fixture = Fixture::new(&recording(&frames));
    let source = fixture.source(name);
    run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
    (fixture, source)
}

#[test]
fn a_family_published_under_another_version_is_pending_through_every_gate() {
    let segments = [Made::Prepared, Made::Prepared, Made::Prepared];
    let (_fixture, source) = published("versions", &segments);
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
    let walked = plan::test_plan(&[bumped("land")], &own).unwrap();
    assert_ne!(walked.walk_stamp(), plan::rows().walk_stamp());
    let moved: BTreeSet<&str> = own
        .iter()
        .copied()
        .filter(|name| walked.stamp(name) != plan::rows().stamp(name))
        .collect();
    assert!(!moved.contains("repairs"));
    let kept = owned(&store, &key, "repairs");
    assert!(!kept.is_empty());
    let republished = Arc::new(Mutex::new(BTreeSet::new()));
    let seen = Arc::clone(&republished);
    plan::with_test_plan(walked, || {
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
    assert_eq!(owned(&store, &key, "repairs"), kept);
    let current = gates::current(&store, &key).unwrap();
    let versions = current.versions().unwrap().unwrap();
    assert_eq!(
        versions.walk,
        walked.walk_stamp(),
        "the walk published again"
    );
}

#[test]
fn a_publication_under_a_grown_recording_s_key_is_decoded_as_it_was_committed() {
    let frames = frames();
    let shorter = recording(&frames[..10]);
    let mut fixture = Fixture::new(&shorter);
    let first = fixture.source("first");
    let (store, key) = crate::prepared::files(&first, None).unwrap();
    republish(&store, &key, &[Made::Prepared]);
    let read = |source: &Source| reply(source, "series-summary", summary(0, 29)).unwrap();
    assert_eq!(read(&first)["data"]["coverage"]["lastHour"], 9);
    // Binding the grown recording checks its key against the shorter one's manifest, which
    // refuses it until its own first commit.
    let longer = recording(&frames);
    fixture.replace(&longer);
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
    republish(&store, &key, &[Made::Prepared, Made::Prepared, Made::Prepared]);
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
    let replay = recording(&frames());
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("published");
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    republish(&store, &key, &[Made::Prepared, Made::Prepared, Made::Prepared]);
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
    use crate::products::peace_deals::test_fixtures as peace;
    let replay = peace::replay(vec![
        peace::frame(100, 3, false, &[9]),
        peace::frame(101, 3, true, &[]),
    ]);
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
    let writer = store.replace(key.clone()).unwrap();
    writer.commit().unwrap();
    let work = read(101);
    assert_eq!((work.manifest_reads, work.stats), (0, 0), "{work:?}");
    // After a Clear, as before the first commit.
    store.clear().unwrap();
    let work = read(99);
    assert_eq!((work.manifest_reads, work.stats), (0, 0), "{work:?}");
}

/// A country's factories from hour 0 to 29, another count every hour.
fn country_frames() -> Vec<DecodedFrame> {
    (0..30)
        .map(|hour| {
            let mut frame = crate::test_fixtures::scale_frame(1, hour, 1);
            frame.metrics.table_ns[TableId::CountryFactories.index()] = 1;
            *frame.tables[TableId::CountryFactories.index()] =
                TableView::Complete(TableRows::CountryFactories(vec![
                    replay_data::rows::CountryFactoriesRow {
                        country: 100_000,
                        military_total_raw: (1_000 + i64::from(hour)) * 100_000,
                        ..Default::default()
                    },
                ]));
            frame
        })
        .collect()
}

/// Publishes the fuel family's root, ten hours a segment from hour 0, each made as `segments`
/// says: GER's fuel read every hour at another level, its emit writing the hours each prepared
/// into the root as the segment driver does.
fn publish_fuel(store: &store::Store, key: &store::SourceKey, segments: &[Made]) {
    use crate::products::country_fuel::{emit::Emit, facts};
    let family = Family::planned(plan::rows(), &["fuel"]).unwrap()[0];
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
    let mut emit = Emit::new(replay_data::format::FormatVersion::CURRENT);
    for (index, made) in (0_u32..).zip(segments) {
        let first = index * 10;
        let (first_hour, last_hour) = (first as i32, first as i32 + 9);
        let emitted = match made {
            Made::Prepared => Some(last_hour),
            Made::Refused { hour, .. } => (*hour > first_hour).then(|| hour - 1),
            _ => None,
        };
        if let Some(emitted) = emitted {
            let writer = stage.series(1, 0).unwrap();
            for hour in first_hour..=emitted {
                let level = (1_000 + i64::from(hour), 32_768);
                let projected =
                    facts::Hour::test_read(1, hour, "GER", (1, 1), hour == first_hour, level);
                emit.hour(writer, projected).unwrap();
            }
            emit.cut(writer, emitted).unwrap();
        }
        let mut state = Vec::new();
        emit.save(&mut state).unwrap();
        let hours = Hours {
            branch: 1,
            run: 0,
            first,
            last: first + 9,
            first_hour,
            last_hour,
            ends_segment: true,
            ends_branch: false,
            proof: [index as u8; 32],
        };
        stage
            .processed(hours, made.clone(), Some(state), None)
            .unwrap();
        stage.submit().unwrap();
    }
    stage.settle().unwrap();
}

/// A fixture whose fuel family prepared hours 0 to 29 as `segments` say, and its country's tag.
fn fuel_published(name: &str, segments: &[Made]) -> (Fixture, Arc<Source>, String) {
    assert_eq!(plan::current().gating("country-fuel"), Some("fuel"));
    let mut fixture = Fixture::new(&recording(&frames()));
    let source = fixture.source(name);
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    publish_fuel(&store, &key, segments);
    (fixture, source, "GER".into())
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
    // Its first day, 0 to 23, goes across the lost run its family went on past, and names it.
    let day = reply(&source, "espionage-log", json!({"branchId":"1","window":0})).unwrap();
    assert_eq!(day["holes"][0]["firstHour"], 10, "{day}");
    assert_eq!(day["holes"][0]["gap"], true, "{day}");
    // The manifest goes to its reader as its type names it, with no field added.
    if let Err(error) = reply(&source, "espionage-log", json!({"branchId":"1"})) {
        assert!(!error.to_string().contains("unknown field"), "{error}");
    }
}

#[test]
fn an_intelligence_range_within_a_lost_run_answers_nothing_before_dispatch() {
    let (_fixture, source) = intelligence_hole("intelligence-hole");
    for product in ["decryption", "operative-placements", "operatives-roster"] {
        let mut params = json!({"branchId":"1","firstHour":12,"lastHour":18});
        if product == "operatives-roster" {
            params["tags"] = json!([]);
        }
        let answered = reply(&source, product, params).unwrap();
        assert!(answered["data"].is_null(), "{product}: {answered}");
        assert_eq!(answered["holes"][0]["firstHour"], 10, "{product}");
    }
}

/// A series family's emit, writing GER's hours: a segment's first names it again.
trait Writes {
    fn hour(&mut self, series: &mut roots::Series, hour: i32, first: bool) -> io::Result<()>;
    fn cut(&mut self, series: &mut roots::Series, last: i32) -> io::Result<()>;
}

impl Writes for crate::products::country_stability::emit::Emit {
    fn hour(&mut self, series: &mut roots::Series, hour: i32, first: bool) -> io::Result<()> {
        crate::products::country_stability::emit::Emit::hour(
            self,
            series,
            stability_hour(hour, first),
        )
    }

    fn cut(&mut self, series: &mut roots::Series, last: i32) -> io::Result<()> {
        crate::products::country_stability::emit::Emit::cut(self, series, last)
    }
}

impl Writes for army_size::emit::Emit {
    fn hour(&mut self, series: &mut roots::Series, hour: i32, first: bool) -> io::Result<()> {
        let divisions = i128::from(hour) + 1;
        let fact = army_size::facts::Hour {
            branch: 1,
            hour,
            named: if first {
                vec![(0, Arc::from("GER"))]
            } else {
                Vec::new()
            },
            live: first.then(|| vec![0]),
            countries: vec![army_size::facts::Country {
                index: 0,
                holder: (1, 1),
                totals: Some([Some(divisions); 8]),
                mix: None,
            }],
            divisions: true,
            strength: true,
            manpower: true,
            battalions: true,
            fallback: false,
            unreadable: 0,
            composition_unreadable: 0,
            invalid: Vec::new(),
            aid: army_size::aid::Projection::default(),
        };
        army_size::emit::Emit::hour(self, series, fact)
    }

    fn cut(&mut self, series: &mut roots::Series, last: i32) -> io::Result<()> {
        army_size::emit::Emit::cut(self, series, last)
    }
}

impl Writes for supply_health::emit::Emit {
    fn hour(&mut self, series: &mut roots::Series, hour: i32, first: bool) -> io::Result<()> {
        let fact = supply_health::facts::Hour {
            branch: 1,
            hour,
            named: if first {
                vec![(0, Arc::from("GER"))]
            } else {
                Vec::new()
            },
            whole: first,
            countries: vec![supply_health::facts::Country {
                index: 0,
                holder: Some((1, 1)),
                counts: Some([1, 2, hour as u32]),
            }],
            land_present: true,
        };
        supply_health::emit::Emit::hour(self, series, fact)
    }

    fn cut(&mut self, series: &mut roots::Series, last: i32) -> io::Result<()> {
        supply_health::emit::Emit::cut(self, series, last)
    }
}

/// Publishes `family`'s root through its stage, ten hours a segment from hour 0 over `segments`
/// segments of one branch, each a batch, `emit` writing every hour; each batch `queued` is left
/// uncommitted as the next begins. What the store then publishes of the family.
fn publish_series(
    source: &Source,
    family: &str,
    emit: &mut dyn Writes,
    segments: u32,
    queued: bool,
) -> roots::Published {
    let (store, key) = crate::prepared::files(source, None).unwrap();
    let family = Family::planned(plan::rows(), &[family]).unwrap()[0];
    let (publisher, _) = Publisher::open(
        &store,
        key.clone(),
        Binding::new(b"publication gates", b"no content"),
        plan::rows(),
        &[family],
        &|_, _, _| true,
    )
    .unwrap();
    let mut stage = publisher.stage(family.id).unwrap();
    for index in 0..segments {
        let first = index * 10;
        let (first_hour, last_hour) = (first as i32, first as i32 + 9);
        let writer = stage.series(1, 0).unwrap();
        for hour in first_hour..=last_hour {
            emit.hour(writer, hour, hour == first_hour).unwrap();
        }
        emit.cut(writer, last_hour).unwrap();
        let hours = Hours {
            branch: 1,
            run: 0,
            first,
            last: first + 9,
            first_hour,
            last_hour,
            ends_segment: true,
            ends_branch: index + 1 == segments,
            proof: [index as u8; 32],
        };
        stage.processed(hours, Made::Prepared, None, None).unwrap();
        if queued {
            stage.send().unwrap();
        } else {
            stage.submit().unwrap();
        }
    }
    stage.settle().unwrap();
    drop(stage);
    drop(publisher);
    let snapshot = store.current().unwrap().unwrap();
    roots::read(&mut &snapshot, family.id.name(), 1)
        .unwrap()
        .unwrap()
}

/// Every family on the series engine goes on through four segments of one branch, each batch
/// from the one before though that one was not yet committed as it began: no key its emit holds
/// is unknown to the series it writes, and what it publishes equals what batches committed one
/// by one publish.
#[test]
fn every_series_family_goes_on_from_its_queued_batch_through_its_segments() {
    let mut fixture = Fixture::new(&recording(&frames()));
    let configured = supply_health::Configured::read(&fixture.source("content"), None).unwrap();
    for family in ["stability", "army", "supply"] {
        let published = [false, true].map(|queued| {
            let source = fixture.source(&format!("{family}-{queued}"));
            let mut emit: Box<dyn Writes> = match family {
                "stability" => Box::new(crate::products::country_stability::emit::Emit::new()),
                "army" => Box::new(army_size::emit::Emit::new()),
                _ => Box::new(configured.emit()),
            };
            publish_series(&source, family, &mut *emit, 4, queued)
        });
        let frontier = &published[1].frontier;
        assert_eq!(
            (frontier.first_hour, frontier.last_hour, frontier.segments),
            (0, 39, 4),
            "{family}"
        );
        assert!(frontier.complete, "{family}");
        assert_eq!(published[0], published[1], "{family}");
    }
}
