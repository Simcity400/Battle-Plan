//! Country-pair counters projected through the existing casualty ingestion.
//! Unknown casualties have no attacker and cannot be assigned to a country pair.
//! Native relation lifetimes accumulate per recorded endpoint identity; a new
//! war never replaces an earlier war's recorded losses. Windows follow replay
//! blocks, with no application-selected period or country-count limit.

use super::*;

pub struct CasualtyPairs;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CasualtyPairsParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    /// Otherwise show each chosen country's casualties with every enemy.
    pub between: bool,
    /// None asks for the replay-block manifest, not campaign-length histories.
    pub window: Option<usize>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtyPairRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Equal values compress consecutive hourly reads. A carried run keeps its
    /// earlier read_hour instead of claiming an hourly confirmation.
    pub read_hour: i32,
    pub hourly: bool,
    pub first_taken: String,
    pub second_taken: String,
    pub incomplete: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtyPairSeries {
    pub identity: String,
    pub first_country: String,
    pub second_country: String,
    pub runs: Vec<CasualtyPairRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtyPairsReply {
    pub branch_id: String,
    pub available: bool,
    pub windows: Vec<HourSpan>,
    pub window: Option<usize>,
    pub observed: Vec<HourSpan>,
    pub pairs: Vec<CasualtyPairSeries>,
    pub has_gap: bool,
    pub warnings: Vec<String>,
}

// The lifetime is an operand, never the grouped war identity.
pub(super) type Key = (Arc<str>, Arc<str>, Identity, Identity, i32);
#[derive(Clone)]
pub(super) struct PairRead {
    pub(super) first_hour: i32,
    pub(super) last_hour: i32,
    pub(super) read_hour: i32,
    pub(super) hourly: bool,
    pub(super) relation_read_hour: i32,
    pub(super) relation_hourly: bool,
    pub(super) first_live: bool,
    pub(super) second_live: bool,
    pub(super) first_tag_hour: i32,
    pub(super) second_tag_hour: i32,
    /// One native lifetime's recorded counters.
    pub(super) first_taken: i64,
    pub(super) second_taken: i64,
    pub(super) incomplete: bool,
}
/// Distinct lifetimes' summed counters over consecutive hours.
struct PairTotal {
    first_hour: i32,
    last_hour: i32,
    read_hour: i32,
    hourly: bool,
    first_taken: i128,
    second_taken: i128,
    incomplete: bool,
}
/// The tags of a pair's current key and its index in `reads`.
type CurrentKey = (Arc<str>, Arc<str>, usize);
#[derive(Default)]
pub(super) struct PairHistory {
    /// Each key's index in `reads`.
    keys: BTreeMap<Key, usize>,
    reads: Vec<Vec<PairRead>>,
    /// By `HeldPair::ordinal`, looked up again only when an endpoint's tag changes.
    current: Vec<Option<CurrentKey>>,
    /// By country index: the last tag it held and the last hour it was live.
    last_tags: Vec<Option<(Arc<str>, i32)>>,
    live: Vec<bool>,
}
impl PairHistory {
    /// Every native lifetime's runs, in key order.
    pub(super) fn runs(&self) -> impl Iterator<Item = (&Key, &[PairRead])> {
        self.keys
            .iter()
            .map(|(key, &index)| (key, self.reads[index].as_slice()))
    }
    /// Releases growth capacity once the branch is scanned.
    pub(super) fn shrink_to_fit(&mut self) {
        for runs in &mut self.reads {
            runs.shrink_to_fit();
        }
    }
    pub(super) fn observe(&mut self, scan: &Scan) {
        let (Some(hour), Some(read_hour)) = (scan.hour, scan.last_read) else {
            return;
        };
        self.live.clear();
        self.live.resize(scan.identities.len(), false);
        self.last_tags.resize(scan.identities.len(), None);
        for (tag, &index) in &scan.live {
            self.live[index] = true;
            match &mut self.last_tags[index] {
                Some((last, at)) if **last == **tag => *at = hour,
                last => *last = Some((Arc::from(tag.as_str()), hour)),
            }
        }
        self.current.resize(scan.pairs.len(), None);
        for (&(first, second, start), pair) in &scan.pairs {
            // Canonical identity order also joins reversed native relations.
            let ((first, second), values) = if scan.identities[first] < scan.identities[second] {
                ((first, second), [pair.values[0], pair.values[1]])
            } else {
                ((second, first), [pair.values[1], pair.values[0]])
            };
            let (first_tag, first_tag_hour) =
                self.last_tags[first].as_ref().expect("endpoint seen live");
            let (second_tag, second_tag_hour) =
                self.last_tags[second].as_ref().expect("endpoint seen live");
            let index = match &mut self.current[pair.ordinal] {
                Some((a, b, index)) if Arc::ptr_eq(a, first_tag) && Arc::ptr_eq(b, second_tag) => {
                    *index
                }
                current => {
                    let reads = &mut self.reads;
                    let key = (
                        first_tag.clone(),
                        second_tag.clone(),
                        scan.identities[first],
                        scan.identities[second],
                        start,
                    );
                    let index = *self.keys.entry(key).or_insert_with(|| {
                        reads.push(Vec::new());
                        reads.len() - 1
                    });
                    *current = Some((first_tag.clone(), second_tag.clone(), index));
                    index
                }
            };
            let first_live = self.live[first];
            let second_live = self.live[second];
            let (first_tag_hour, second_tag_hour) = (*first_tag_hour, *second_tag_hour);
            let [first_taken, second_taken] = values;
            let incomplete = scan.countries[first].at_least || scan.countries[second].at_least;
            let hourly = read_hour == hour;
            let relation_hourly = pair.hour == hour;
            let runs = &mut self.reads[index];
            if let Some(last) = runs.last_mut()
                && last.last_hour.checked_add(1) == Some(hour)
                && last.first_taken == first_taken
                && last.second_taken == second_taken
                && last.incomplete == incomplete
                && last.hourly == hourly
                && (hourly || last.read_hour == read_hour)
                && last.relation_hourly == relation_hourly
                && (relation_hourly || last.relation_read_hour == pair.hour)
                && last.first_live == first_live
                && last.second_live == second_live
                && (first_live || last.first_tag_hour == first_tag_hour)
                && (second_live || last.second_tag_hour == second_tag_hour)
            {
                last.last_hour = hour;
            } else {
                runs.push(PairRead {
                    first_hour: hour,
                    last_hour: hour,
                    read_hour,
                    hourly,
                    relation_read_hour: pair.hour,
                    relation_hourly,
                    first_live,
                    second_live,
                    first_tag_hour,
                    second_tag_hour,
                    first_taken,
                    second_taken,
                    incomplete,
                });
            }
        }
    }
}

/// Distinct native lifetimes add only in the pair projection. The same retained
/// runs also feed the war breakdown; neither consumer rescans the recording.
fn summed_runs(histories: &[&[PairRead]], within: Option<&HourSpan>) -> Vec<PairTotal> {
    let mut changes: BTreeMap<i64, Vec<(usize, Option<&PairRead>)>> = BTreeMap::new();
    for (index, runs) in histories.iter().enumerate() {
        let first = within.map_or(0, |span| {
            runs.partition_point(|run| run.last_hour < span.first_hour)
        });
        for run in runs[first..]
            .iter()
            .take_while(|run| within.is_none_or(|span| run.first_hour <= span.last_hour))
        {
            // Preserve the pair widget's two-live-holder contract. The casualty
            // breakdown separately retains losses against an ended enemy.
            if !run.first_live || !run.second_live {
                continue;
            }
            let first = within.map_or(run.first_hour, |span| run.first_hour.max(span.first_hour));
            let last = within.map_or(run.last_hour, |span| run.last_hour.min(span.last_hour));
            changes
                .entry(i64::from(first))
                .or_default()
                .push((index, Some(run)));
            changes
                .entry(i64::from(last) + 1)
                .or_default()
                .push((index, None));
        }
    }
    let mut active = BTreeMap::new();
    let mut totals = [0_i128; 2];
    let mut incomplete = 0_usize;
    let mut reply: Vec<PairTotal> = Vec::new();
    let mut changes = changes.into_iter().peekable();
    while let Some((first, events)) = changes.next() {
        // End a run before starting its consecutive replacement.
        for (index, next) in events
            .iter()
            .filter(|(_, next)| next.is_none())
            .chain(events.iter().filter(|(_, next)| next.is_some()))
        {
            if let Some(old) = active.remove(index) {
                let old: &PairRead = old;
                totals[0] -= i128::from(old.first_taken);
                totals[1] -= i128::from(old.second_taken);
                incomplete -= usize::from(old.incomplete);
            }
            if let Some(next) = next {
                active.insert(*index, *next);
                totals[0] += i128::from(next.first_taken);
                totals[1] += i128::from(next.second_taken);
                incomplete += usize::from(next.incomplete);
            }
        }
        let (Some(&next), Some(read)) =
            (changes.peek().map(|(hour, _)| hour), active.values().next())
        else {
            continue;
        };
        let first = first as i32;
        let last = (next - 1) as i32;
        let read_hour = if read.hourly { first } else { read.read_hour };
        if let Some(old) = reply.last_mut()
            && old.last_hour.checked_add(1) == Some(first)
            && old.first_taken == totals[0]
            && old.second_taken == totals[1]
            && old.incomplete == (incomplete != 0)
            && old.hourly == read.hourly
            && (read.hourly || old.read_hour == read_hour)
        {
            old.last_hour = last;
        } else {
            reply.push(PairTotal {
                first_hour: first,
                last_hour: last,
                read_hour,
                hourly: read.hourly,
                first_taken: totals[0],
                second_taken: totals[1],
                incomplete: incomplete != 0,
            });
        }
    }
    reply
}

impl Product for CasualtyPairs {
    const KEY: &'static str = "casualty-pairs";
    const CONTENT_DEPENDENT: bool = false;
    type Params = CasualtyPairsParams;
    type Reply = CasualtyPairsReply;

    fn types() -> Vec<String> {
        vec![
            CasualtyPairsParams::decl(),
            CasualtyPairRun::decl(),
            CasualtyPairSeries::decl(),
            CasualtyPairsReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        _context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        Self::validate(params).map_err(io::Error::other)?;
        check_cancel(cancelled)?;
        check_cadence(cadence::cadence(TableId::WarRelations))?;
        let branch = params.branch_id.parse().expect("validated branch");
        let replay = source.replay();
        if !replay.branches.iter().any(|recorded| recorded.id == branch) {
            return Err(io::Error::other("Unknown casualty branch"));
        }
        let windows: Vec<_> = replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .map(|block| HourSpan {
                first_hour: block.first_hour,
                last_hour: block.last_hour,
            })
            .collect();
        let mut reply = CasualtyPairsReply {
            branch_id: params.branch_id.clone(),
            available: false,
            windows,
            window: params.window,
            observed: Vec::new(),
            pairs: Vec::new(),
            has_gap: false,
            warnings: Vec::new(),
        };
        // The capability request doesn't require a whole-branch pair scan.
        let Some(window) = params.window else {
            crate::blocks::scan_blocks(
                &replay,
                replay.index.iter().filter(|block| block.branch == branch),
                &SCAN_TABLES,
                cancelled,
                |blocks| {
                    for (_, frames) in blocks {
                        let mut decoder = FrameDecoder::for_tables(&SCAN_TABLES);
                        if let Ok(frames) = frames {
                            let mut builder = ScanFrames::new(&replay, branch);
                            for frame in frames {
                                check_cancel(cancelled)?;
                                let Ok(decoded) = decoder.advance(&frame) else {
                                    break;
                                };
                                builder.frame(decoded, &replay);
                                if !builder.scan.observed.is_empty() {
                                    reply.available = true;
                                    break;
                                }
                            }
                        }
                        if reply.available {
                            break;
                        }
                    }
                    Ok(())
                },
            )?;
            check_reply_bytes(json_bytes(&reply)?)?;
            return Ok(reply);
        };
        let span = reply
            .windows
            .get(window)
            .ok_or_else(|| io::Error::other("Unknown casualty window"))?;
        let cached = scanned_branch(source, branch, cancelled)?;
        reply.available = !cached.observed.is_empty();
        reply.has_gap = cached.incomplete;
        reply.observed = cached
            .observed
            .iter()
            .filter_map(|read| {
                let first_hour = read.first_hour.max(span.first_hour);
                let last_hour = read.last_hour.min(span.last_hour);
                (first_hour <= last_hour).then_some(HourSpan {
                    first_hour,
                    last_hour,
                })
            })
            .collect();
        // Manual country picks follow their native identities through renames.
        let selected = cached.selected_identities(&params.tags);
        let mut pairs: BTreeMap<_, Vec<&[PairRead]>> = BTreeMap::new();
        for ((first, second, a, b, _), runs) in cached.pairs.runs() {
            check_cancel(cancelled)?;
            if !params.tags.is_empty() {
                let matches = [selected.contains(a), selected.contains(b)];
                if !(if params.between {
                    matches[0] && matches[1]
                } else {
                    matches[0] || matches[1]
                }) {
                    continue;
                }
            }
            pairs.entry((first, second, a, b)).or_default().push(runs);
        }
        for ((first, second, a, b), histories) in pairs {
            check_cancel(cancelled)?;
            let runs: Vec<_> = summed_runs(&histories, Some(span))
                .iter()
                .filter_map(|run| {
                    let first_hour = run.first_hour.max(span.first_hour);
                    let last_hour = run.last_hour.min(span.last_hour);
                    (first_hour <= last_hour).then(|| CasualtyPairRun {
                        first_hour,
                        last_hour,
                        read_hour: run.read_hour,
                        hourly: run.hourly,
                        first_taken: run.first_taken.to_string(),
                        second_taken: run.second_taken.to_string(),
                        incomplete: run.incomplete,
                    })
                })
                .collect();
            if !runs.is_empty() {
                reply.pairs.push(CasualtyPairSeries {
                    identity: format!("{}:{}|{}:{}", a.0, a.1, b.0, b.1),
                    first_country: first.to_string(),
                    second_country: second.to_string(),
                    runs,
                });
            }
        }
        check_reply_bytes(json_bytes(&reply)?)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair_runs(history: &PairHistory) -> Vec<PairTotal> {
        summed_runs(
            &history.runs().map(|(_, runs)| runs).collect::<Vec<_>>(),
            None,
        )
    }

    fn observe(
        scan: &mut Scan,
        history: &mut PairHistory,
        hour: i32,
        values: Option<[i64; 4]>,
        read: bool,
    ) {
        let a = scan.country((1, 10));
        let b = scan.country((2, 20));
        let live = BTreeMap::from([("mod attacker".into(), a), ("mod victim".into(), b)]);
        let changed = scan.live != live;
        scan.begin_hour(hour, &live, changed, read);
        if let Some(values) = values {
            scan.relation(a, b, 0, values);
        }
        scan.end_hour(read);
        history.observe(scan);
    }
    #[test]
    fn casualty_pairs_retain_ended_relations_signed_corrections_and_gaps() {
        let mut scan = Scan::new();
        let mut history = PairHistory::default();
        observe(&mut scan, &mut history, 100, Some([12, 7, 99, 99]), true);
        observe(&mut scan, &mut history, 101, Some([10, -3, 99, 99]), true);
        observe(&mut scan, &mut history, 102, None, true);
        scan.gap();
        observe(&mut scan, &mut history, 105, None, true);
        let runs = pair_runs(&history);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1].first_taken, 10);
        assert_eq!(runs[1].second_taken, -3);
        assert_eq!(runs[1].last_hour, 102);
        assert_eq!(runs[2].first_hour, 105);
        assert!(
            !runs[2].incomplete,
            "a relation absent before the gap remains exact"
        );
    }
    #[test]
    fn casualty_pairs_do_not_redate_carried_values() {
        let mut scan = Scan::new();
        let mut history = PairHistory::default();
        observe(&mut scan, &mut history, 100, Some([12, 7, 0, 0]), true);
        observe(&mut scan, &mut history, 101, None, false);
        observe(&mut scan, &mut history, 102, Some([12, 7, 0, 0]), true);
        let runs = pair_runs(&history);
        assert_eq!(runs.len(), 3);
        assert!(!runs[1].hourly);
        assert_eq!(runs[1].read_hour, 100);
        assert_eq!(runs[2].read_hour, 102);
    }
    #[test]
    fn signed_native_hour_endpoints_preserve_a_gap_and_the_terminal_confirmation() {
        let mut scan = Scan::new();
        let mut history = PairHistory::default();
        observe(
            &mut scan,
            &mut history,
            i32::MIN,
            Some([12, -3, 0, 0]),
            true,
        );
        observe(
            &mut scan,
            &mut history,
            i32::MAX,
            Some([10, -2, 0, 0]),
            true,
        );
        let runs = pair_runs(&history);
        assert_eq!(runs.len(), 2);
        assert_eq!(
            (runs[0].first_hour, runs[0].last_hour),
            (i32::MIN, i32::MIN)
        );
        assert_eq!(
            (runs[1].first_hour, runs[1].last_hour),
            (i32::MAX, i32::MAX)
        );
        assert_eq!(
            (runs[1].read_hour, runs[1].first_taken, runs[1].second_taken),
            (i32::MAX, 10, -2)
        );
        assert!(scan.incomplete);
    }

    #[test]
    fn casualty_pairs_country_replacement_never_inherits_old_losses() {
        let mut scan = Scan::new();
        let mut history = PairHistory::default();
        observe(&mut scan, &mut history, 100, Some([12, 7, 0, 0]), true);
        let a = scan.country((1, 11));
        let b = scan.country((2, 20));
        let live = BTreeMap::from([("mod attacker".into(), a), ("mod victim".into(), b)]);
        scan.begin_hour(101, &live, true, true);
        scan.end_hour(true);
        history.observe(&scan);
        assert_eq!(pair_runs(&history)[0].last_hour, 100);
    }

    #[test]
    fn casualty_pairs_sum_separate_lifetimes_and_reversed_endpoints_in_wide_signed_arithmetic() {
        let mut scan = Scan::new();
        let mut history = PairHistory::default();
        observe(
            &mut scan,
            &mut history,
            100,
            Some([i64::MAX, -3, 99, 99]),
            true,
        );
        let a = scan.country((1, 10));
        let b = scan.country((2, 20));
        let live = scan.live.clone();
        scan.begin_hour(101, &live, false, true);
        scan.relation(b, a, 1, [7, i64::MAX, 999, 999]);
        scan.end_hour(true);
        history.observe(&scan);
        let runs = pair_runs(&history);
        let last = runs.last().unwrap();
        assert_eq!(last.first_taken, i128::from(i64::MAX) * 2);
        assert_eq!(last.second_taken, 4);
    }

    #[test]
    fn a_reversed_native_row_updates_one_lifetime_and_keeps_victim_orientation() {
        let mut scan = Scan::new();
        let mut history = PairHistory::default();
        observe(&mut scan, &mut history, 100, Some([12, 7, 0, 0]), true);
        let a = scan.country((1, 10));
        let b = scan.country((2, 20));
        let live = scan.live.clone();
        scan.begin_hour(101, &live, false, true);
        scan.relation(b, a, 0, [8, 13, 0, 0]);
        scan.end_hour(true);
        history.observe(&scan);
        assert_eq!(scan.pairs.len(), 1);
        assert_eq!(scan.countries[a].taken, 13);
        let runs = pair_runs(&history);
        assert_eq!(
            (
                runs.last().unwrap().first_taken,
                runs.last().unwrap().second_taken
            ),
            (13, 8)
        );
    }

    #[test]
    fn native_lifetimes_keep_a_removed_enemy_without_giving_its_replacement_the_old_facts() {
        let mut scan = Scan::new();
        let mut history = PairHistory::default();
        observe(&mut scan, &mut history, 100, Some([12, 7, 0, 0]), true);
        let a = scan.country((1, 10));
        let live = BTreeMap::from([("mod attacker".into(), a)]);
        scan.begin_hour(101, &live, true, true);
        scan.end_hour(true);
        history.observe(&scan);
        let old = history.runs().next().unwrap().1.last().unwrap();
        assert!(!old.second_live);
        assert_eq!(
            (old.second_tag_hour, old.relation_read_hour, old.first_taken),
            (100, 100, 12)
        );
        assert_eq!(pair_runs(&history).last().unwrap().last_hour, 100);
        let replacement = scan.country((2, 21));
        let live = BTreeMap::from([
            ("mod attacker".into(), a),
            ("mod victim".into(), replacement),
        ]);
        scan.begin_hour(102, &live, true, true);
        scan.end_hour(true);
        history.observe(&scan);
        let old = history.runs().next().unwrap().1.last().unwrap();
        assert!(!old.second_live);
        assert_eq!(old.second_tag_hour, 100);
    }

    #[test]
    fn casualty_pair_product_manifest_windows_selection_and_read_dates_agree() {
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{
            frame::TableView,
            rows::{CountryLinkRow, CountryRow, TableRows, WarRelationRow},
        };
        let frames = (100..=102)
            .map(|hour| {
                let mut frame = source(1, hour, None);
                *frame.tables[TableId::Countries.index()] =
                    TableView::Complete(TableRows::Countries(vec![
                        CountryRow {
                            id: 10,
                            canonical_slot: 1,
                            tag: b"mod attacker".to_vec(),
                            ..Default::default()
                        },
                        CountryRow {
                            id: 20,
                            canonical_slot: 2,
                            tag: b"mod victim".to_vec(),
                            ..Default::default()
                        },
                    ]));
                *frame.tables[TableId::CountryLinks.index()] =
                    TableView::Complete(TableRows::CountryLinks(vec![
                        CountryLinkRow {
                            id: 10,
                            canonical_slot: 1,
                        },
                        CountryLinkRow {
                            id: 20,
                            canonical_slot: 2,
                        },
                    ]));
                *frame.tables[TableId::WarRelations.index()] =
                    TableView::Complete(TableRows::WarRelations(if hour == 102 {
                        vec![]
                    } else {
                        vec![WarRelationRow {
                            first_country: 10,
                            second_country: 20,
                            first_casualties: i64::MAX,
                            second_casualties: if hour == 100 { 7 } else { -3 },
                            ..Default::default()
                        }]
                    }));
                frame
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "pairs".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let load = |params: &CasualtyPairsParams| {
            CasualtyPairs::load(
                &source,
                params,
                &LoadContext { content: None },
                &AtomicBool::new(false),
            )
            .unwrap()
        };
        let mut params = CasualtyPairsParams {
            branch_id: "1".into(),
            tags: vec!["mod attacker".into()],
            between: false,
            window: None,
        };
        let manifest = load(&params);
        assert!(manifest.available);
        assert!(manifest.pairs.is_empty());
        assert!(manifest.observed.is_empty());
        params.window = Some(0);
        let reply = load(&params);
        let cached = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
        assert_eq!(reply.pairs.len(), 1);
        assert_eq!(reply.pairs[0].runs.last().unwrap().second_taken, "-3");
        assert_eq!(reply.pairs[0].runs.last().unwrap().last_hour, 102);
        params.between = true;
        assert!(load(&params).pairs.is_empty());
        params.tags.push("mod victim".into());
        assert_eq!(load(&params).pairs.len(), 1);
        assert!(Arc::ptr_eq(
            &cached,
            &scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap()
        ));
        assert_eq!(
            cached
                .country_at("mod attacker", 102)
                .unwrap()
                .dealt
                .to_string(),
            reply.pairs[0].runs.last().unwrap().second_taken
        );
        params.window = Some(manifest.windows.len());
        assert!(
            CasualtyPairs::load(
                &source,
                &params,
                &LoadContext { content: None },
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }
}
