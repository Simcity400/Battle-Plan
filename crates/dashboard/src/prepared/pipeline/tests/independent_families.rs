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
}

/// Each fold's family key, thread and observed hours.
static SEEN: Mutex<Vec<(&'static str, ThreadId, Vec<i32>)>> = Mutex::new(Vec::new());

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
    // The identities: prepared on their own, still observed hourly for
    // territory under its names. The groups' reader, air superiority, is
    // prepared on its own with its own read of them, and every other
    // registered family is read by no other family in the same hour, so the
    // walk skips it.
    let kept = ["identities"];
    let names = independent::names();
    assert!(kept.iter().all(|name| names.contains(name)));
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
        assert!(progress.get("beta").is_none(), "{label}");
        // Four store commits: the first held alpha's and the reader's
        // batches, then alpha committed alone at each later block.
        assert_eq!(publications, (4, 5), "{label}");
        clear(source);
    }
}
