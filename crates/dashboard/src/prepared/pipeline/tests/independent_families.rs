//! Families prepared on their own through the generic phase: synthetic
//! families publish at once what they publish one after another, one that
//! fails or refuses leaves the others committed, the registry alone decides
//! what the hourly walk skips, keeps and gates, and a family's deadline
//! splits the quiet spans of blocks read by their changes. Group commits
//! publish what each family committing alone publishes, in fewer store
//! commits, and a family refused at adoption or failing its write inside a
//! group leaves the others' batches committed.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::{
    frame_changes::FrameChanges,
    prepass::{self, Extract, Family, Folded},
};
use independent::{Block, Branch, Load, Observed, Own};
use replay::{
    container::{QuietHours, TableFields},
    format::FormatVersion,
};
use serde::{Deserialize, Serialize};
use std::{any::Any, ops::Range, sync::Mutex, thread::ThreadId};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// One block's facts of a synthetic family: every hour of the block, and the
/// hours its extract was given a frame for.
struct Facts {
    hours: Vec<i32>,
    observed: Vec<i32>,
}

/// Lists every hour; with `every`, keeps a deadline at each multiple of it.
struct HoursExtract {
    facts: Facts,
    every: Option<i32>,
}

impl Extract for HoursExtract {
    fn observe(&mut self, _: &Replay, frame: &DecodedFrame, _: &FrameChanges) -> io::Result<()> {
        self.facts.hours.push(frame.hour);
        self.facts.observed.push(frame.hour);
        Ok(())
    }

    fn deadline(&self) -> Option<i32> {
        let every = self.every?;
        let last = *self.facts.hours.last()?;
        Some((last / every + 1) * every)
    }

    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        self.facts.hours.extend_from_slice(&hours.hours()[span]);
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(self.facts))
    }
}

fn whole(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(HoursExtract {
        facts: Facts {
            hours: Vec::new(),
            observed: Vec::new(),
        },
        every: None,
    })
}

fn by_changes(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(HoursExtract {
        facts: Facts {
            hours: Vec::new(),
            observed: Vec::new(),
        },
        every: Some(3),
    })
}

/// The countries' keys: unchanged in these recordings, whose balances change
/// every hour.
fn country_keys() -> io::Result<Vec<TableFields>> {
    Ok(vec![TableFields::keys(TableId::Countries)])
}

/// Reads every hour whole.
static ALPHA: Family = Family {
    name: "alpha",
    extract: "alpha.extract",
    sequential: "alpha.sequential",
    fallback: "alpha.fallback",
    tables: &[TableId::Countries],
    fields: None,
    start: whole,
};

/// Reads the hours the countries' keys change, and a deadline every third
/// hour.
static BETA: Family = Family {
    name: "beta",
    extract: "beta.extract",
    sequential: "beta.sequential",
    fallback: "beta.fallback",
    tables: &[TableId::Countries],
    fields: Some(country_keys),
    start: by_changes,
};

/// Takes the subjects' name, so the hourly walk skips them.
static SUBJECTS: Family = Family {
    name: "subjects",
    extract: "subjects.extract",
    sequential: "subjects.sequential",
    fallback: "subjects.fallback",
    tables: &[TableId::Countries],
    fields: None,
    start: whole,
};

/// A synthetic family prepared on its own: its pages list every hour it
/// folded on a branch. `key` names its folds in [`SEEN`]; it refuses or
/// fails at the hours given.
struct Hours {
    family: &'static Family,
    timing: &'static str,
    gates: &'static [&'static str],
    key: &'static str,
    refuse: Option<i32>,
    fail: Option<i32>,
    kept: &'static [(&'static str, &'static str)],
    read_by: &'static [&'static str],
    /// Fails its commit's write once it has folded this hour.
    fail_commit: Option<i32>,
    /// A section it writes at every commit beside its own.
    overlap: Option<&'static str>,
    on_fold: Option<fn(i32)>,
}

/// Each fold's family key, thread and observed hours.
static SEEN: Mutex<Vec<(&'static str, ThreadId, Vec<i32>)>> = Mutex::new(Vec::new());
static PARITY_TEST: Mutex<()> = Mutex::new(());

fn seen(key: &str) -> Vec<(ThreadId, Vec<i32>)> {
    SEEN.lock()
        .unwrap()
        .iter()
        .filter(|(seen, _, _)| *seen == key)
        .map(|(_, thread, hours)| (*thread, hours.clone()))
        .collect()
}

fn forget(key: &str) {
    SEEN.lock().unwrap().retain(|(seen, _, _)| *seen != key);
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    hours: Vec<i32>,
}

struct HoursBranch {
    name: &'static str,
    key: &'static str,
    refuse: Option<i32>,
    fail: Option<i32>,
    fail_commit: Option<i32>,
    overlap: Option<&'static str>,
    saved: Saved,
    /// The last hour of each block it went through since its last commit,
    /// each a page of its own, as hourly preparation publishes per block.
    ended: Vec<i32>,
    on_fold: Option<fn(i32)>,
}

impl HoursBranch {
    fn hour(&mut self, hour: i32) -> io::Result<()> {
        if self.fail == Some(hour) {
            return Err(io::Error::other(format!(
                "Injected failure of {} at hour {hour}",
                self.name
            )));
        }
        if self.refuse == Some(hour) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Injected refusal of {} at hour {hour}", self.name),
            ));
        }
        self.saved.hours.push(hour);
        Ok(())
    }

    fn hours(
        &mut self,
        hours: Vec<i32>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let mut reached = None;
        for hour in hours {
            check()?;
            reached = Some(hour);
            let result = self.hour(hour);
            if !observed(hour, result)? {
                break;
            }
        }
        Ok(reached)
    }
}

impl Branch for HoursBranch {
    fn begin_block(&mut self) {}

    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        _: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let facts = *facts
            .downcast::<Facts>()
            .map_err(|_| io::Error::other("Facts of another family"))?;
        if let Some(on_fold) = self.on_fold {
            on_fold(facts.hours[0]);
        }
        SEEN.lock()
            .unwrap()
            .push((self.key, std::thread::current().id(), facts.observed));
        Ok((Folded::Fast, self.hours(facts.hours, observed, check)?))
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let mut hours = Vec::new();
        block.replay.read_block_alone(
            block.index,
            &[TableId::Countries],
            |_, frame| {
                hours.push(frame.hour);
                Ok(())
            },
            || Ok(()),
        )?;
        self.hours(hours, observed, check)
    }

    fn end_block(&mut self, _: &Replay, _: u64) {
        if let Some(&hour) = self.saved.hours.last() {
            self.ended.push(hour);
        }
    }

    fn publish(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        _: &AtomicBool,
    ) -> io::Result<()> {
        if let (Some(at), Some(&last)) = (self.fail_commit, self.saved.hours.last())
            && last >= at
        {
            return Err(io::Error::other(format!(
                "Injected commit failure of {} after hour {last}",
                self.name
            )));
        }
        if let Some(section) = self.overlap {
            writer.write_at(section, 0, self.name.as_bytes())?;
        }
        for hour in std::mem::take(&mut self.ended) {
            writer.write_at(
                &format!("synthetic-{}-ended-{branch}-{hour}", self.name),
                0,
                b"ended".as_slice(),
            )?;
        }
        Ok(())
    }

    fn pending(&self) -> usize {
        0
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        _: &AtomicBool,
    ) -> io::Result<()> {
        // The hours so far, whatever the batches: a run resumed after a
        // cancellation at any commit publishes what an uncancelled one does.
        let hours = serde_json::to_vec(&self.saved.hours.len())?;
        writer.write_at(
            &format!("synthetic-{}-count-{branch}", self.name),
            0,
            hours.as_slice(),
        )
    }

    fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        let page = serde_json::to_vec(&self.saved.hours)?;
        Ok(writer.write_at(
            &format!("synthetic-{}-{branch}", self.name),
            0,
            page.as_slice(),
        ))
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        json(
            writer,
            &format!("synthetic-{}-state-{branch}", self.name),
            &self.saved,
        )
    }
}

impl Own for Hours {
    fn family(&self) -> &'static Family {
        self.family
    }

    fn timing(&self) -> &'static str {
        self.timing
    }

    fn gates(&self) -> &'static [&'static str] {
        self.gates
    }

    fn kept(&self) -> &'static [(&'static str, &'static str)] {
        self.kept
    }

    fn read_by(&self) -> &'static [&'static str] {
        self.read_by
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let section = format!("synthetic-{}-state-{branch}", self.family.name);
        let saved = match load.read(&section)? {
            Some(file) => serde_json::from_reader(io::BufReader::new(file))?,
            None => Saved::default(),
        };
        Ok(Box::new(HoursBranch {
            name: self.family.name,
            key: self.key,
            refuse: self.refuse,
            fail: self.fail,
            fail_commit: self.fail_commit,
            overlap: self.overlap,
            saved,
            ended: Vec::new(),
            on_fold: self.on_fold,
        }))
    }
}

const fn hours(
    family: &'static Family,
    timing: &'static str,
    gates: &'static [&'static str],
    key: &'static str,
) -> Hours {
    Hours {
        family,
        timing,
        gates,
        key,
        refuse: None,
        fail: None,
        kept: &[],
        read_by: &[],
        fail_commit: None,
        overlap: None,
        on_fold: None,
    }
}

/// Kept for a reader of its state in the same hour.
static KEPT_ALPHA: Hours = Hours {
    kept: &[("alpha-section", "reader-alpha-section")],
    read_by: &["reader"],
    ..hours(
        &ALPHA,
        "independent.alpha",
        &["alpha-product"],
        "kept-alpha",
    )
};

/// The reader, once it is prepared on its own too.
static READER: Family = Family {
    name: "reader",
    extract: "reader.extract",
    sequential: "reader.sequential",
    fallback: "reader.fallback",
    tables: &[TableId::Countries],
    fields: None,
    start: whole,
};
static OWN_READER: Hours = hours(&READER, "independent.reader", &[], "own-reader");

static PARITY_ALPHA: Hours = hours(
    &ALPHA,
    "independent.alpha",
    &["alpha-product"],
    "parity-alpha",
);
static PARITY_BETA: Hours = hours(&BETA, "independent.beta", &["beta-product"], "parity-beta");
static WELL_ALPHA: Hours = hours(
    &ALPHA,
    "independent.alpha",
    &["alpha-product"],
    "well-alpha",
);
static FAILING_ALPHA: Hours = Hours {
    fail: Some(105),
    ..hours(
        &ALPHA,
        "independent.alpha",
        &["alpha-product"],
        "failing-alpha",
    )
};
static REFUSING_ALPHA: Hours = Hours {
    refuse: Some(105),
    ..hours(
        &ALPHA,
        "independent.alpha",
        &["alpha-product"],
        "refusing-alpha",
    )
};
static OTHER_BETA: Hours = hours(&BETA, "independent.beta", &["beta-product"], "other-beta");
static REFUSING_SUBJECTS: Hours = Hours {
    refuse: Some(105),
    ..hours(
        &SUBJECTS,
        "independent.subjects",
        &["subjects"],
        "refusing-subjects",
    )
};
static OWN_SUBJECTS: Hours = hours(
    &SUBJECTS,
    "independent.subjects",
    &["subjects"],
    "own-subjects",
);

fn registered(families: &[&'static Hours]) -> Vec<&'static dyn Own> {
    families
        .iter()
        .map(|&family| family as &'static dyn Own)
        .collect()
}

/// Branch 1 in `blocks` blocks of four hours (108 and 109 a gap before the
/// third), then branch 2's one block when `second`.
fn recording(blocks: usize, second: bool) -> Replay {
    let hours: [&[i32]; 3] = [
        &[100, 101, 102, 103],
        &[104, 105, 106, 107],
        &[110, 111, 112, 113],
    ];
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours.iter().map(|&hour| frame(1, hour, 2)).collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(hours[0]))]).unwrap();
    for &hours in &hours[1..blocks] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(hours))])
            .unwrap();
    }
    if second {
        replay = replay
            .append(&[records(
                2,
                (20..=23).map(|hour| frame(2, hour, 2)).collect(),
            )])
            .unwrap();
    }
    replay
}

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// Every block committed on its own.
fn measured(source: &Source) -> io::Result<()> {
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
    .map(|_| ())
}

/// The hours a page lists.
fn page(payloads: &Payloads, section: &str) -> Vec<i32> {
    serde_json::from_slice(&payloads[&(section.to_owned(), 0)]).unwrap()
}

fn family_coverage(payloads: &Payloads, family: &str, branch: u64) -> Option<CoveragePage> {
    payloads
        .get(&(independent::coverage_section(family, branch), 0))
        .map(|bytes| serde_json::from_slice(bytes).unwrap())
}

#[test]
fn families_prepared_on_their_own_at_once_publish_what_they_publish_one_after_another() {
    let _test = PARITY_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let first = recording(2, false);
    let grown = recording(3, true);
    let families = registered(&[&PARITY_ALPHA, &PARITY_BETA]);
    let branch_1: Vec<i32> = (100..=107).chain(110..=113).collect();
    let branch_2: Vec<i32> = (20..=23).collect();
    let mut fixture = Fixture::new(&first);
    let test = std::thread::current().id();
    for workers in [1, 3] {
        for resume in [false, true] {
            // Prepared on the first recording, then on the grown one; with
            // `resume`, each run is first cancelled at its first accepted
            // commit and run again.
            let both = |fixture: &mut Fixture| -> Payloads {
                for replay in [&first, &grown] {
                    fixture.replace(replay);
                    let source = fixture.source("families");
                    if resume {
                        let flag = AtomicBool::new(false);
                        match run(&source, None, &flag, |_| {
                            flag.store(true, Ordering::Release)
                        }) {
                            Ok(_) => {}
                            Err(error) => assert_eq!(error.kind(), io::ErrorKind::Interrupted),
                        }
                    }
                    measured(&source).unwrap();
                    if std::ptr::eq(replay, &grown) {
                        let payloads = payloads(&source, None);
                        clear(source);
                        return payloads;
                    }
                }
                unreachable!("the grown recording is prepared last")
            };
            let label = format!("{workers} workers, resumed {resume}");
            forget("parity-alpha");
            forget("parity-beta");
            let apart = prepass::with_test_prepass(workers, None, || {
                independent::with_test_families(&families, || {
                    independent::with_test_one_after_another(|| both(&mut fixture))
                })
            });
            for key in ["parity-alpha", "parity-beta"] {
                assert!(
                    seen(key).iter().all(|(thread, _)| *thread == test),
                    "{label}: {key} folded on this thread one after another"
                );
            }
            forget("parity-alpha");
            forget("parity-beta");
            let together = prepass::with_test_prepass(workers, None, || {
                independent::with_test_families(&families, || both(&mut fixture))
            });
            // At once, each family folds on a thread of its own (a resumed
            // run may leave one family alone with blocks to prepare).
            let (alpha, beta) = (seen("parity-alpha"), seen("parity-beta"));
            assert!(!alpha.is_empty() && !beta.is_empty(), "{label}");
            if !resume {
                for (thread, _) in &alpha {
                    assert!(
                        *thread != test && beta.iter().all(|(other, _)| other != thread),
                        "{label}: the families folded on threads of their own"
                    );
                }
            }
            let shared = |payloads: &Payloads| -> Payloads {
                payloads
                    .iter()
                    .filter(|((name, _), _)| name != "preparation-checkpoint-v1")
                    .map(|(key, bytes)| (key.clone(), bytes.clone()))
                    .collect()
            };
            assert_payloads_equal(&shared(&together), &shared(&apart), &label);
            for family in ["alpha", "beta"] {
                assert_eq!(page(&together, &format!("synthetic-{family}-1")), branch_1);
                assert_eq!(page(&together, &format!("synthetic-{family}-2")), branch_2);
                for branch in [1, 2] {
                    let coverage = family_coverage(&together, family, branch)
                        .unwrap_or_else(|| panic!("{label}: {family} covers branch {branch}"));
                    assert!(coverage.coverage.complete, "{label}: {family} {branch}");
                }
            }
            // Beta's blocks are read only at their first hour and at its
            // deadlines (every third hour), the hours between kept quiet.
            let mut observed: Vec<Vec<i32>> = beta.into_iter().map(|(_, hours)| hours).collect();
            observed.sort();
            observed.dedup();
            for expected in [vec![100, 102], vec![104, 105], vec![110, 111], vec![20, 21]] {
                assert!(
                    observed.contains(&expected),
                    "{label}: beta observed {observed:?}"
                );
            }
            assert!(
                observed.iter().all(|hours| hours.len() == 2),
                "{label}: beta observed {observed:?}"
            );
        }
    }
}

#[test]
fn a_family_that_fails_or_refuses_leaves_the_others_coverage_and_pages_committed() {
    let replay = recording(3, true);
    let mut fixture = Fixture::new(&replay);
    let beta_sections = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                name.starts_with("synthetic-beta-") || name.starts_with("preparation-family-beta-")
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    let reference =
        independent::with_test_families(&registered(&[&WELL_ALPHA, &OTHER_BETA]), || {
            let source = fixture.source("family-failure");
            measured(&source).unwrap();
            let payloads = payloads(&source, None);
            clear(source);
            beta_sections(&payloads)
        });
    assert!(!reference.is_empty());
    for alpha in [&FAILING_ALPHA, &REFUSING_ALPHA] {
        let source = fixture.source("family-failure");
        let result = prepass::with_test_prepass(2, None, || {
            independent::with_test_families(&registered(&[alpha, &OTHER_BETA]), || {
                measured(&source)
            })
        });
        let actual = payloads(&source, None);
        let label = alpha.key;
        assert_payloads_equal(&beta_sections(&actual), &reference, label);
        let flag = AtomicBool::new(false);
        let coverage = family_coverage(&actual, "alpha", 1).expect("alpha's first block");
        if alpha.fail.is_some() {
            let error = result.unwrap_err();
            assert_ne!(error.kind(), io::ErrorKind::InvalidData, "{label}");
            assert_eq!(error.to_string(), "Injected failure of alpha at hour 105");
            // Alpha committed its first block, before the one it failed in.
            assert_eq!(coverage.coverage.last_hour, 103, "{label}");
            assert!(!coverage.coverage.complete, "{label}");
            assert_eq!(page(&actual, "synthetic-alpha-1"), vec![100, 101, 102, 103]);
        } else {
            result.unwrap();
            assert_eq!(coverage.refused.0["alpha"].hour, 105, "{label}");
            assert!(coverage.coverage.complete, "{label}");
            // A refused branch's pages are not saved again.
            assert_eq!(page(&actual, "synthetic-alpha-1"), vec![100, 101, 102, 103]);
            assert_eq!(page(&actual, "synthetic-alpha-2"), vec![20, 21, 22, 23]);
            // Its gated product refuses from its own page, on that branch only.
            independent::with_test_families(&registered(&[alpha, &OTHER_BETA]), || {
                let error = refusal(&source, None, 1, "alpha-product", &flag).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                assert_eq!(error.to_string(), "Injected refusal of alpha at hour 105");
                refusal(&source, None, 2, "alpha-product", &flag).unwrap();
            });
            // In one batch of the whole branch, the block it went through
            // before refusing keeps its page, as hourly preparation wrote it
            // at that block's end; the refusing block has none.
            clear(source);
            let source = fixture.source("family-failure");
            independent::with_test_families(&registered(&[alpha, &OTHER_BETA]), || {
                run(&source, None, &flag, |_| {}).unwrap();
            });
            let batched = payloads(&source, None);
            let ended =
                |hour: i32| batched.contains_key(&(format!("synthetic-alpha-ended-1-{hour}"), 0));
            assert!(
                ended(103),
                "{label}: the block before the refusal published"
            );
            assert!(!ended(107) && !ended(113), "{label}");
            assert!(batched.contains_key(&("synthetic-alpha-ended-2-23".to_owned(), 0)));
            clear(source);
            continue;
        }
        clear(source);
    }
}

#[test]
fn the_registry_decides_what_the_hourly_walk_skips_keeps_and_gates() {
    // Every registered family's readers are prepared on their own too,
    // including territory and the event feed's reads of the identities.
    // The walk therefore keeps none of these families for another reader.
    let kept: [&str; 0] = [];
    let names = independent::names();
    assert!(
        ["identities", "territory"]
            .iter()
            .all(|name| names.contains(name))
    );
    for &name in &names {
        assert!(independent::registered(name));
        assert_eq!(independent::hourly(name), kept.contains(&name), "{name}");
        assert_eq!(
            Refused::default().live(name),
            kept.contains(&name),
            "{name}"
        );
        assert!(Refused::default().open(name), "{name}");
    }
    assert!(!independent::registered("subjects-stub"));
    assert_eq!(
        independent::kept_as("country-groups"),
        "air-superiority-groups"
    );
    assert_eq!(
        independent::kept_as("country-group-changes"),
        "air-superiority-group-changes"
    );
    assert_eq!(independent::gating("country-groups"), Some("groups"));
    independent::with_test_families(&registered(&[&OWN_SUBJECTS, &PARITY_BETA]), || {
        assert_eq!(independent::names(), ["beta", "subjects"]);
        // Read by no other family: skipped, without having refused.
        assert!(!independent::hourly("subjects"));
        let refused = Refused::default();
        assert!(refused.open("subjects") && !refused.live("subjects"));
        // The groups are prepared hourly again, under their own names.
        assert!(independent::hourly("groups"));
        assert_eq!(independent::kept_as("country-groups"), "country-groups");
        assert_eq!(independent::gating("subjects"), Some("subjects"));
        assert_eq!(independent::gating("beta-product"), Some("beta"));
        assert_eq!(independent::gating("country-groups"), None);
    });
    // A family kept for a reader stays in the walk, under the reader's names,
    // until the reader is prepared on its own too and makes its own read.
    independent::with_test_families(&registered(&[&KEPT_ALPHA]), || {
        assert!(independent::hourly("alpha"));
        assert_eq!(
            independent::kept_as("alpha-section"),
            "reader-alpha-section"
        );
    });
    independent::with_test_families(&registered(&[&KEPT_ALPHA, &OWN_READER]), || {
        assert!(!independent::hourly("alpha"));
        assert!(!independent::hourly("reader"));
    });
    independent::with_test_hourly(|| {
        assert!(independent::names().is_empty());
        assert_eq!(independent::kept_as("country-groups"), "country-groups");
        assert_eq!(independent::gating("country-groups"), None);
    });
    // End to end: the hourly walk publishes nothing of a family registered on
    // its own, whose product answers from that family's coverage.
    let replay = recording(2, true);
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    let hourly = independent::with_test_hourly(|| {
        let source = fixture.source("skipped");
        run(&source, None, &flag, |_| {}).unwrap();
        let payloads = payloads(&source, None);
        clear(source);
        payloads
    });
    let subjects = |payloads: &Payloads| {
        payloads
            .keys()
            .filter(|(name, _)| {
                name.starts_with("subjects-") || name.starts_with("subject-changes")
            })
            .count()
    };
    assert!(subjects(&hourly) > 0);
    let source = fixture.source("skipped");
    independent::with_test_families(&registered(&[&OWN_SUBJECTS]), || {
        // On lanes, so the parallel flush's helpers skip it too.
        lanes::with_test_lanes(4, None, || run(&source, None, &flag, |_| {})).unwrap();
        let own = payloads(&source, None);
        assert_eq!(subjects(&own), 0, "the hourly walk skipped the subjects");
        assert_eq!(
            page(&own, "synthetic-subjects-1"),
            (100..=107).collect::<Vec<_>>()
        );
        let coverage = product_coverage(&source, None, 1, "subjects", &flag)
            .unwrap()
            .unwrap();
        let own_page = family_coverage(&own, "subjects", 1).unwrap().coverage;
        assert_eq!(
            (coverage.first_hour, coverage.last_hour, coverage.complete),
            (own_page.first_hour, own_page.last_hour, true)
        );
    });
    clear(source);
    // A query reading a family prepared on its own (the event feed reads the
    // subjects) refuses with that family's refusal, kept only in its own page.
    let source = fixture.source("skipped");
    independent::with_test_families(&registered(&[&REFUSING_SUBJECTS]), || {
        run(&source, None, &flag, |_| {}).unwrap();
        let error = refusal(&source, None, 1, "event-feed", &flag).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "Injected refusal of subjects at hour 105"
        );
        refusal(&source, None, 2, "event-feed", &flag).unwrap();
    });
    clear(source);
}

static GROUPED_READER: Hours = hours(&READER, "independent.reader", &[], "grouped-reader");
/// Both write `synthetic-shared` at every commit: adopted into one group,
/// the second is refused.
static SHARING_ALPHA: Hours = Hours {
    overlap: Some("synthetic-shared"),
    ..hours(
        &ALPHA,
        "independent.alpha",
        &["alpha-product"],
        "sharing-alpha",
    )
};
static SHARING_BETA: Hours = Hours {
    overlap: Some("synthetic-shared"),
    ..hours(&BETA, "independent.beta", &["beta-product"], "sharing-beta")
};
/// Its second commit's write fails.
static FAILING_READER: Hours = Hours {
    fail_commit: Some(104),
    ..hours(&READER, "independent.reader", &[], "failing-reader")
};

/// Prepares the first recording, then the grown one, each first cancelled at
/// its first accepted commit and run again when `resume`; returns what the
/// grown one published.
fn prepared_twice(fixture: &mut Fixture, first: &Replay, grown: &Replay, resume: bool) -> Payloads {
    for replay in [first, grown] {
        fixture.replace(replay);
        let source = fixture.source("grouped");
        if resume {
            let flag = AtomicBool::new(false);
            match run(&source, None, &flag, |_| {
                flag.store(true, Ordering::Release)
            }) {
                Ok(_) => {}
                Err(error) => assert_eq!(error.kind(), io::ErrorKind::Interrupted),
            }
        }
        measured(&source).unwrap();
        if std::ptr::eq(replay, grown) {
            let payloads = payloads(&source, None);
            clear(source);
            return payloads;
        }
    }
    unreachable!("the grown recording is prepared last")
}

#[test]
fn group_commits_publish_what_each_family_committing_alone_publishes() {
    let _test = PARITY_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let first = recording(2, false);
    let grown = recording(3, true);
    let families = registered(&[&PARITY_ALPHA, &PARITY_BETA, &GROUPED_READER]);
    let mut fixture = Fixture::new(&first);
    for workers in [1, 3] {
        for resume in [false, true] {
            let label = format!("{workers} workers, resumed {resume}");
            let mut prepare = |grouping| {
                independent::take_test_publications();
                let payloads = prepass::with_test_prepass(workers, None, || {
                    independent::with_test_families(&families, || {
                        independent::with_test_grouping(grouping, || {
                            prepared_twice(&mut fixture, &first, &grown, resume)
                        })
                    })
                });
                (payloads, independent::take_test_publications())
            };
            // A resumed run's cancelled commits follow timing; what both
            // runs end with does not, saved state included when not resumed.
            let compared = |payloads: &Payloads| -> Payloads {
                payloads
                    .iter()
                    .filter(|((name, _), _)| !resume || name != "preparation-checkpoint-v1")
                    .map(|(key, bytes)| (key.clone(), bytes.clone()))
                    .collect()
            };
            let (alone, (stores, commits)) = prepare(independent::Grouping::Alone);
            assert!(commits > 0, "{label}");
            assert_eq!(stores, commits, "{label}: alone, each commit is a store's");
            for family in ["alpha", "beta", "reader"] {
                assert_eq!(
                    page(&alone, &format!("synthetic-{family}-1")),
                    (100..=107).chain(110..=113).collect::<Vec<_>>(),
                    "{label}: {family}"
                );
            }
            for grouping in [
                independent::Grouping::Natural,
                independent::Grouping::Together,
            ] {
                let label = format!("{label}, {grouping:?}");
                let (grouped, (grouped_stores, grouped_commits)) = prepare(grouping);
                assert_payloads_equal(&compared(&grouped), &compared(&alone), &label);
                assert!(grouped_stores <= grouped_commits, "{label}");
                if resume {
                    continue;
                }
                // Batches end by the recording, whoever commits together.
                assert_eq!(grouped_commits, commits, "{label}");
                if grouping == independent::Grouping::Together {
                    // Each family commits at every block, so every group
                    // holds all three: one store commit per block.
                    assert_eq!(grouped_stores * 3, grouped_commits, "{label}");
                }
            }
        }
    }
}

#[test]
fn a_family_failing_inside_a_group_commit_leaves_the_others_batches_committed() {
    let replay = recording(3, true);
    let mut fixture = Fixture::new(&replay);
    let alpha_sections = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                name.starts_with("synthetic-alpha-")
                    || name.starts_with("preparation-family-alpha-")
                    || name == "synthetic-shared"
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    let reference = independent::with_test_families(&registered(&[&SHARING_ALPHA]), || {
        let source = fixture.source("group-failure");
        measured(&source).unwrap();
        let payloads = payloads(&source, None);
        clear(source);
        alpha_sections(&payloads)
    });
    assert_eq!(reference[&("synthetic-shared".to_owned(), 0)], b"alpha");
    let families = registered(&[&SHARING_ALPHA, &SHARING_BETA, &FAILING_READER]);
    for workers in [1, 3] {
        let label = format!("{workers} workers");
        let source = fixture.source("group-failure");
        independent::take_test_publications();
        // Every group waits for every family still preparing, so the first
        // holds all three: alpha (adopted first, in registry order), beta
        // (refused, since alpha's batch created the section it shares) and
        // the reader. The reader's second write fails; alpha goes on alone.
        let result = prepass::with_test_prepass(workers, None, || {
            independent::with_test_families(&families, || {
                independent::with_test_grouping(independent::Grouping::Together, || {
                    measured(&source)
                })
            })
        });
        let publications = independent::take_test_publications();
        let actual = payloads(&source, None);
        // Beta's refusal at adoption is its own error, reported first in
        // registry order; the reader's failure is its own too.
        let error = result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other, "{label}");
        assert_eq!(
            error.to_string(),
            "parallel prepared writers overlapped in section synthetic-shared",
            "{label}"
        );
        // Nothing of beta's batch is listed.
        assert!(
            actual
                .keys()
                .all(|(name, _)| !name.starts_with("synthetic-beta-")
                    && !name.starts_with("preparation-family-beta-")),
            "{label}"
        );
        // Alpha published, through every group, what it publishes alone.
        assert_payloads_equal(&alpha_sections(&actual), &reference, &label);
        // The reader's first batch committed with alpha's; its second, whose
        // write failed, is not listed.
        assert_eq!(
            page(&actual, "synthetic-reader-1"),
            vec![100, 101, 102, 103]
        );
        let coverage = family_coverage(&actual, "reader", 1).expect("the reader's first batch");
        assert_eq!(coverage.coverage.last_hour, 103, "{label}");
        assert!(!coverage.coverage.complete, "{label}");
        let checkpoint: serde_json::Value =
            serde_json::from_slice(&actual[&("preparation-checkpoint-v1".to_owned(), 0)]).unwrap();
        let progress = &checkpoint["independent"];
        assert_eq!(progress["alpha"]["next_block"], 4, "{label}");
        assert_eq!(progress["reader"]["next_block"], 1, "{label}");
        assert_eq!(
            progress["beta"],
            serde_json::to_value(independent::Progress::default()).unwrap(),
            "{label}: the rejected family remains pending without an accepted batch"
        );
        // Four store commits: the first held alpha's and the reader's
        // batches, then alpha committed alone at each later block.
        assert_eq!(publications, (4, 5), "{label}");
        clear(source);
    }
}

#[derive(Default)]
struct Overlap {
    enabled: bool,
    at: i32,
    waiting: bool,
    walk_commits: usize,
    folds: usize,
}

static OVERLAP_TEST: Mutex<()> = Mutex::new(());
// Deadlock detection for event-order proofs, allowing other pipeline tests to
// use the shared compile/test machine. This is not a preparation speed bound.
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(30);

static OVERLAP: (Mutex<Overlap>, std::sync::Condvar) = (
    Mutex::new(Overlap {
        enabled: false,
        at: 0,
        waiting: false,
        walk_commits: 0,
        folds: 0,
    }),
    std::sync::Condvar::new(),
);

fn hold_second_fold(hour: i32) {
    let (state, changed) = &OVERLAP;
    let mut state = state.lock().unwrap();
    state.folds += 1;
    if !state.enabled || hour != state.at {
        return;
    }
    state.waiting = true;
    changed.notify_all();
    let (state, _) = changed
        .wait_timeout_while(state, HANDSHAKE_DEADLINE, |state| state.walk_commits == 0)
        .unwrap();
    let committed = state.walk_commits > 0;
    drop(state);
    assert!(committed, "the walk waited for the family's second fold");
}

static OVERLAPPING_ALPHA: Hours = Hours {
    on_fold: Some(hold_second_fold),
    ..hours(
        &ALPHA,
        "independent.alpha",
        &["alpha-product"],
        "overlapping-alpha",
    )
};

/// A counted walk commit releases a family blocked in its second fold.
/// This cannot pass when the walk runs after all the families. Every
/// product page matches the families-first reference.
#[test]
fn walk_commits_while_a_family_is_still_folding_and_keeps_every_page() {
    let _test = OVERLAP_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let replay = recording(3, false);
    let mut fixture = Fixture::new(&replay);
    let families = registered(&[&OVERLAPPING_ALPHA]);
    independent::with_test_families(&families, || {
        let source = fixture.source("walk-overlap");
        *OVERLAP.0.lock().unwrap() = Overlap::default();
        independent::with_test_one_after_another(|| measured(&source)).unwrap();
        let expected = payloads(&source, None);
        clear(source);

        let source = fixture.source("walk-overlap");
        *OVERLAP.0.lock().unwrap() = Overlap {
            enabled: true,
            at: 104,
            ..Default::default()
        };
        let mut observed = 0;
        let mut previous_walk = 0;
        let mut previous_family = 0;
        run_measured_through(
            &source,
            None,
            &AtomicBool::new(false),
            true,
            Duration::ZERO,
            Some(u64::MAX),
            |_| Ok(()),
            |next| {
                assert!(
                    next.next_block >= previous_walk,
                    "walk status moved backwards"
                );
                previous_walk = next.next_block;
                let covered = next.families.get("alpha");
                let complete =
                    covered.is_some_and(|branches| branches.values().all(|extent| extent.complete));
                if next.next_block == 0 {
                    return;
                }
                let (state, changed) = &OVERLAP;
                let state = state.lock().unwrap();
                if !state.enabled || state.walk_commits > 0 {
                    return;
                }
                let (mut state, _) = changed
                    .wait_timeout_while(state, HANDSHAKE_DEADLINE, |state| !state.waiting)
                    .unwrap();
                assert!(
                    state.waiting && !complete,
                    "family finished before the walk progressed"
                );
                state.walk_commits += 1;
                observed += 1;
                previous_family = state.folds;
                changed.notify_all();
            },
        )
        .unwrap();
        assert_eq!(observed, 1);
        assert_eq!(previous_family, 2);
        assert_eq!(OVERLAP.0.lock().unwrap().folds, 3);
        let actual = payloads(&source, None);
        let without_checkpoint = |pages: Payloads| {
            pages
                .into_iter()
                .filter(|((name, _), _)| name != CHECKPOINT)
                .collect::<Payloads>()
        };
        assert_payloads_equal(
            &without_checkpoint(actual),
            &without_checkpoint(expected),
            "overlapping walk vs families first",
        );
        clear(source);
    });
}

/// Leave independently committed cursors on both sides of the walk, then
/// reopen the recording with or without growth. Count the actual folds and
/// walk blocks: rebuilding accepted work must fail even if the final pages agree.
fn resume_separate_prefixes(family_leads: bool, append: bool) {
    let _test = OVERLAP_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let replay = recording(3, false);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("separate-prefixes");
    let families = registered(&[&OVERLAPPING_ALPHA]);
    independent::with_test_families(&families, || {
        *OVERLAP.0.lock().unwrap() = Overlap {
            enabled: !family_leads,
            at: 110,
            ..Default::default()
        };
        let cancelled = AtomicBool::new(false);
        let stop = || {
            run_measured_through(
                &source,
                None,
                &cancelled,
                true,
                Duration::ZERO,
                Some(u64::MAX),
                |_| Ok(()),
                |next| {
                    if family_leads {
                        if next.next_block == 1 {
                            cancelled.store(true, Ordering::Release);
                        }
                        return;
                    }
                    if next.next_block != replay.index.len() {
                        return;
                    }
                    let (state, changed) = &OVERLAP;
                    let state = state.lock().unwrap();
                    let (mut state, _) = changed
                        .wait_timeout_while(state, HANDSHAKE_DEADLINE, |state| !state.waiting)
                        .unwrap();
                    assert!(state.waiting);
                    cancelled.store(true, Ordering::Release);
                    state.walk_commits = 1;
                    changed.notify_all();
                },
            )
        };
        let result = if family_leads {
            independent::with_test_one_after_another(stop)
        } else {
            stop()
        };
        assert_eq!(
            result.err().map(|error| error.kind()),
            Some(io::ErrorKind::Interrupted)
        );
        let (store, key) = super::super::super::files(&source, None).unwrap();
        let (saved, _) = checkpoint(&store, &key, Some(&key)).unwrap().unwrap();
        let (walk_next, family_next) = if family_leads { (1, 3) } else { (3, 2) };
        assert_eq!(saved.cursor.next_block, walk_next);
        assert_eq!(saved.independent["alpha"].next_block, family_next);
        assert!(metadata_prefix(&saved, &replay, None).unwrap());
        let mut invalid = saved.clone();
        invalid.independent.get_mut("alpha").unwrap().next_block = replay.index.len() + 1;
        assert!(!metadata_prefix(&invalid, &replay, None).unwrap());
        let flag = AtomicBool::new(false);
        assert!(restore(&source, None, &flag).unwrap().is_none());
        assert_eq!(
            unprepared_blocks(&source, None).unwrap(),
            Some(replay.index.len() - walk_next)
        );
        let walked = coverage(&source, None, 1, &flag).unwrap().unwrap();
        let own = product_coverage(&source, None, 1, "alpha-product", &flag)
            .unwrap()
            .unwrap();
        assert_eq!(walked.last_hour, replay.index[walk_next - 1].last_hour);
        assert_eq!(walked.complete, walk_next == replay.index.len());
        assert_eq!(own.last_hour, replay.index[family_next - 1].last_hour);
        assert_eq!(own.complete, family_next == replay.index.len());
        let status = family_coverages(&saved, &replay, &last_blocks(&replay));
        assert_eq!(status["alpha"]["1"].last_hour, own.last_hour);
        assert_eq!(status["alpha"]["1"].complete, own.complete);

        let grown = if append {
            replay
                .append(&[continuation_records(
                    &replay,
                    1,
                    (114..=117).map(|hour| frame(1, hour, 2)).collect(),
                )])
                .unwrap()
        } else {
            replay
        };
        if append {
            fixture.replace(&grown);
        }
        let resumed = fixture.source("separate-prefixes-resumed");
        assert!(metadata_prefix(&saved, &grown, None).unwrap());
        assert!(!metadata_prefix(&invalid, &grown, None).unwrap());
        assert_eq!(
            unprepared_blocks(&resumed, None).unwrap(),
            Some(grown.index.len() - walk_next)
        );
        assert!(restore(&resumed, None, &flag).unwrap().is_none());
        OVERLAP.0.lock().unwrap().enabled = false;
        forget("overlapping-alpha");
        let mut blocks = Vec::new();
        let (completed, timings) = run_measured_through(
            &resumed,
            None,
            &flag,
            true,
            Duration::ZERO,
            Some(u64::MAX),
            |block| {
                blocks.push(block.ordinal);
                Ok(())
            },
            |_| {},
        )
        .unwrap();
        let expected_walk: Vec<_> = (walk_next..grown.index.len()).collect();
        assert_eq!(
            blocks, expected_walk,
            "only the walk's unaccepted blocks are processed"
        );
        assert_eq!(completed.new_blocks, expected_walk.len());
        assert_eq!(timings.hours, expected_walk.len() * 4);
        let folded: Vec<_> = seen("overlapping-alpha")
            .into_iter()
            .map(|(_, hours)| hours)
            .collect();
        let expected_family: Vec<Vec<i32>> = grown.index[family_next..]
            .iter()
            .map(|block| (block.first_hour..=block.last_hour).collect())
            .collect();
        assert_eq!(
            folded, expected_family,
            "only the family's unaccepted blocks are folded"
        );
        assert!(completed.coverage["1"].complete);
        assert!(completed.families["alpha"]["1"].complete);
        assert!(restore(&resumed, None, &flag).unwrap().is_some());
        assert_eq!(unprepared_blocks(&resumed, None).unwrap(), Some(0));
        let (store, key) = super::super::super::files(&resumed, None).unwrap();
        let (saved, _) = checkpoint(&store, &key, Some(&key)).unwrap().unwrap();
        assert_eq!(saved.cursor.next_block, grown.index.len());
        assert_eq!(saved.independent["alpha"].next_block, grown.index.len());
        let expected_hours: Vec<_> = (100..=107)
            .chain(110..=grown.index.last().unwrap().last_hour)
            .collect();
        assert_eq!(
            page(&payloads(&resumed, None), "synthetic-alpha-1"),
            expected_hours
        );
        clear(resumed);
    });
}

#[test]
fn a_completed_walk_resumes_families_that_are_still_pending() {
    resume_separate_prefixes(false, false);
}

#[test]
fn append_resumes_a_family_behind_the_completed_walk() {
    resume_separate_prefixes(false, true);
}

#[test]
fn a_family_ahead_of_the_walk_resumes_only_its_own_tail() {
    resume_separate_prefixes(true, false);
}

#[test]
fn append_resumes_a_family_ahead_of_the_walk_from_its_own_prefix() {
    resume_separate_prefixes(true, true);
}

#[derive(Default)]
struct BatchOverlap {
    enabled: bool,
    walk_started: bool,
    publications: Vec<usize>,
}

static BATCH_OVERLAP: (Mutex<BatchOverlap>, std::sync::Condvar) = (
    Mutex::new(BatchOverlap {
        enabled: false,
        walk_started: false,
        publications: Vec::new(),
    }),
    std::sync::Condvar::new(),
);

fn wait_for_walk_batch(_: i32) {
    let (state, changed) = &BATCH_OVERLAP;
    let state = state.lock().unwrap();
    if !state.enabled {
        return;
    }
    let (state, _) = changed
        .wait_timeout_while(state, HANDSHAKE_DEADLINE, |state| !state.walk_started)
        .unwrap();
    let started = state.walk_started;
    drop(state);
    assert!(started, "walk batch never started");
}

static PUBLISHING_ALPHA: Hours = Hours {
    on_fold: Some(wait_for_walk_batch),
    ..hours(
        &ALPHA,
        "independent.alpha",
        &["alpha-product"],
        "publishing-alpha",
    )
};

/// Hold the first walk batch inside its first hour. Two complete family
/// publications must reach the manifest before that batch can continue.
/// An owned writer (or a fork counted throughout preparation) cannot pass.
#[test]
fn families_publish_multiple_batches_while_one_walk_batch_is_in_progress() {
    let _test = OVERLAP_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let replay = recording(3, false);
    let mut fixture = Fixture::new(&replay);
    let families = registered(&[&PUBLISHING_ALPHA]);
    independent::with_test_families(&families, || {
        let source = fixture.source("walk-batch-publications");
        *BATCH_OVERLAP.0.lock().unwrap() = BatchOverlap::default();
        independent::with_test_one_after_another(|| measured(&source)).unwrap();
        let without_checkpoint = |pages: Payloads| {
            pages
                .into_iter()
                .filter(|((name, _), _)| name != CHECKPOINT)
                .collect::<Payloads>()
        };
        let expected = without_checkpoint(payloads(&source, None));
        clear(source);
        for cancel in [false, true] {
            let source = fixture.source("walk-batch-publications");
            *BATCH_OVERLAP.0.lock().unwrap() = BatchOverlap {
                enabled: true,
                ..Default::default()
            };
            let cancelled = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&cancelled);
            let hook: lanes::Hook = Arc::new(move |family, frame, _| {
                if family != "coordinator" || frame.hour != 100 {
                    return lanes::Fault::Continue;
                }
                let (state, changed) = &BATCH_OVERLAP;
                let mut state = state.lock().unwrap();
                state.walk_started = true;
                changed.notify_all();
                let (state, _) = changed
                    .wait_timeout_while(state, HANDSHAKE_DEADLINE, |state| {
                        state.publications.len() < 2
                    })
                    .unwrap();
                assert!(
                    state.publications.len() >= 2,
                    "families could not publish during the open walk batch: {:?}",
                    state.publications
                );
                if cancel {
                    flag.store(true, Ordering::Release);
                }
                lanes::Fault::Continue
            });
            let result = independent::with_test_on_publish(
                |family, next| {
                    if family != "alpha" {
                        return;
                    }
                    let (state, changed) = &BATCH_OVERLAP;
                    let mut state = state.lock().unwrap();
                    assert!(state.walk_started);
                    state.publications.push(next);
                    changed.notify_all();
                },
                || {
                    super::super::with_test_batch_blocks(1, || {
                        lanes::with_test_lanes(4, Some(hook), || {
                            run(&source, None, &cancelled, |_| {})
                        })
                    })
                },
            );
            assert!(
                BATCH_OVERLAP
                    .0
                    .lock()
                    .unwrap()
                    .publications
                    .starts_with(&[1, 2])
            );
            BATCH_OVERLAP.0.lock().unwrap().enabled = false;
            if cancel {
                assert_eq!(
                    result.err().map(|error| error.kind()),
                    Some(io::ErrorKind::Interrupted)
                );
                let (store, key) = super::super::super::files(&source, None).unwrap();
                let (saved, _) = checkpoint(&store, &key, None).unwrap().unwrap();
                assert_eq!(
                    saved.cursor.next_block, 0,
                    "unaccepted walk pages stayed private"
                );
                assert!(saved.independent["alpha"].next_block >= 2);
                assert!(store.read(&key, &branch_section(1), 0).unwrap().is_none());
                cancelled.store(false, Ordering::Release);
                measured(&source).unwrap();
            } else {
                result.unwrap();
            }
            let (store, key) = super::super::super::files(&source, None).unwrap();
            let (saved, _) = checkpoint(&store, &key, None).unwrap().unwrap();
            assert!(
                saved.forward_complete(replay.index.len()),
                "walk and family cursors both retained"
            );
            assert!(
                restore(&source, None, &AtomicBool::new(false))
                    .unwrap()
                    .is_some(),
                "side publications finished with the forward publishers"
            );
            assert_payloads_equal(
                &without_checkpoint(payloads(&source, None)),
                &expected,
                "family commits during walk batch, with cancellation and resume",
            );
            clear(source);
        }
    });
}

/// Exercise the real persistent runner, rather than a normal-priority test
/// thread. A walk acceptance must release a family still folding its second
/// block, and the walk must run at the same job level as that family.
#[test]
#[cfg(windows)]
fn runner_walk_progresses_at_family_priority_while_a_family_is_still_running() {
    use dashboard_base::prepared::priority::JobPriority;
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_NORMAL,
    };
    let _test = OVERLAP_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    *OVERLAP.0.lock().unwrap() = Overlap {
        enabled: true,
        at: 104,
        ..Default::default()
    };
    let replay = recording(3, false);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("runner-walk-overlap");
    let worker_source = Arc::clone(&source);
    let runner = crate::prepared::runner::Runner::new().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    runner
        .submit(Arc::new(AtomicBool::new(false)), move |cancelled| {
            let result = JobPriority::run(Arc::new(AtomicBool::new(false)), || {
                independent::with_test_families(&registered(&[&OVERLAPPING_ALPHA]), || {
                    let mut counted = 0;
                    let result = run_measured_through(
                        &worker_source,
                        None,
                        cancelled,
                        true,
                        Duration::ZERO,
                        Some(u64::MAX),
                        |_| Ok(()),
                        |accepted| {
                            assert_eq!(
                                unsafe { GetThreadPriority(GetCurrentThread()) },
                                THREAD_PRIORITY_NORMAL,
                                "walk must not inherit persistent runner background mode"
                            );
                            if accepted.next_block == 0 {
                                return;
                            }
                            let (state, changed) = &OVERLAP;
                            let state = state.lock().unwrap();
                            if state.walk_commits > 0 {
                                return;
                            }
                            let (mut state, _) = changed
                                .wait_timeout_while(state, HANDSHAKE_DEADLINE, |state| {
                                    !state.waiting
                                })
                                .unwrap();
                            assert!(state.waiting && state.folds == 2);
                            counted += 1;
                            state.walk_commits += 1;
                            changed.notify_all();
                        },
                    );
                    result.map(|_| counted)
                })
            });
            let _ = send.send(result);
        })
        .unwrap();
    let counted = receive
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    runner.shutdown();
    assert_eq!(
        counted, 1,
        "accepted walk progress while the second family fold was held"
    );
    assert_eq!(OVERLAP.0.lock().unwrap().folds, 3);
    OVERLAP.0.lock().unwrap().enabled = false;
    clear(source);
}
