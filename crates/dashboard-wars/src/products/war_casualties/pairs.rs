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
#[derive(Clone, Serialize, Deserialize)]
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
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct HolderRun {
    pub(super) first_hour: i32,
    pub(super) last_hour: i32,
    pub(super) live: [bool; 2],
    pub(super) incomplete: bool,
}
#[derive(Default, Serialize, Deserialize)]
pub(super) struct PairHistory {
    /// Endpoint presence and uncertainty, independent of counter changes and lifetimes.
    #[serde(with = "dashboard_core::map_pairs")]
    pub(super) coverage: BTreeMap<(Identity, Identity), Vec<HolderRun>>,
    /// Each key's index in `reads`.
    #[serde(with = "dashboard_core::map_pairs")]
    keys: BTreeMap<Key, usize>,
    reads: Vec<Vec<PairRead>>,
    /// By `HeldPair::ordinal`, looked up again only when an endpoint's tag changes.
    #[serde(skip)]
    current: Vec<Option<CurrentKey>>,
    /// By country index: the last tag it held and the last hour it was live.
    last_tags: Vec<Option<(Arc<str>, i32)>>,
    #[serde(skip)]
    live: Vec<bool>,
    #[serde(skip)]
    adjacent: Vec<Vec<PairKey>>,
    #[serde(skip)]
    joined: bool,
    frontier: Option<i32>,
    supplying: bool,
    supplying_hour: Option<i32>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub(super) work: dashboard_base::prepared::observer_tests::Work,
}
impl dashboard_base::prepared::prepass::Held for HolderRun {
    fn held(&self, _: &mut dashboard_base::prepared::prepass::HeldBytes) {}
}

impl dashboard_base::prepared::prepass::Held for PairRead {
    fn held(&self, _: &mut dashboard_base::prepared::prepass::HeldBytes) {}
}

/// Every pair's coverage runs and reads, its keys and the tags they name
/// (shared tags once), and the per-country and per-pair indexes.
impl dashboard_base::prepared::prepass::Held for PairHistory {
    fn held(&self, bytes: &mut dashboard_base::prepared::prepass::HeldBytes) {
        bytes
            .heap(&self.coverage)
            .heap(&self.keys)
            .heap(&self.reads)
            .heap(&self.current)
            .heap(&self.last_tags)
            .heap(&self.live)
            .heap(&self.adjacent);
    }
}

impl PairHistory {
    /// Every native lifetime's runs, in key order.
    pub(super) fn runs(&self) -> impl Iterator<Item = (&Key, &[PairRead])> + Clone {
        self.keys
            .iter()
            .map(|(key, &index)| (key, self.reads[index].as_slice()))
    }
    /// Releases growth capacity once the branch is scanned.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn shrink_to_fit(&mut self) {
        self.seal();
        for runs in &mut self.reads {
            runs.shrink_to_fit();
        }
    }
    #[cfg(any(test, feature = "test-support"))]
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
        self.observe_keys(scan, scan.pairs.keys().copied());
        self.frontier = Some(hour);
        self.supplying = read_hour == hour;
        self.supplying_hour = Some(read_hour);
    }

    /// The existing ordinal join also finds ended lifetimes of changed holders.
    /// Its one rebuild is runtime-only; no duplicate lifetime scan is retained.
    pub(super) fn observe_changes(&mut self, builder: &ScanFrames) {
        let scan = &builder.scan;
        let (Some(hour), Some(read_hour)) = (scan.hour, scan.last_read) else {
            self.seal();
            self.frontier = None;
            return;
        };
        let rebuilding = !self.joined;
        let before = self.current.len();
        self.current.resize(scan.pairs.len(), None);
        self.adjacent.resize_with(scan.identities.len(), Vec::new);
        self.live.resize(scan.identities.len(), false);
        self.last_tags.resize(scan.identities.len(), None);
        if rebuilding {
            for &key in scan.pairs.keys() {
                let (first, second, _) = key;
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.history_visits += 1;
                }
                self.adjacent[first].push(key);
                self.adjacent[second].push(key);
            }
            self.joined = true;
        } else {
            for &key in &builder.changed_pairs {
                let (first, second, _) = key;
                if scan.pairs[&key].ordinal >= before {
                    self.adjacent[first].push(key);
                    self.adjacent[second].push(key);
                }
            }
        }
        let mut endpoints = builder.changed_endpoints.clone();
        if rebuilding || self.frontier.is_none() {
            endpoints.extend(0..scan.identities.len());
        }
        for index in endpoints.iter().copied() {
            match builder.current_tags.get(index).and_then(Option::as_ref) {
                Some(tag) => {
                    if self.last_tags[index]
                        .as_ref()
                        .is_none_or(|(last, _)| **last != **tag)
                    {
                        self.last_tags[index] = Some((Arc::from(tag.as_str()), hour));
                    } else {
                        self.last_tags[index].as_mut().unwrap().1 = hour;
                    }
                    self.live[index] = true;
                }
                None => {
                    if self.live[index]
                        && let (Some((_, at)), Some(frontier)) =
                            (&mut self.last_tags[index], self.frontier)
                    {
                        *at = frontier;
                    }
                    self.live[index] = false;
                }
            }
        }
        let hourly = read_hour == hour;
        let all = rebuilding
            || self.frontier.is_none()
            || self
                .frontier
                .is_some_and(|frontier| frontier.checked_add(1) != Some(hour))
            || self.supplying != hourly
            || (!hourly && self.supplying_hour != Some(read_hour));
        let mut keys = builder.changed_pairs.clone();
        if all {
            keys.extend(scan.pairs.keys().copied());
        } else {
            for index in endpoints {
                keys.extend(self.adjacent[index].iter().copied());
            }
        }
        if let Some(frontier) = self.frontier {
            for &key in &keys {
                self.materialize(scan, key, frontier);
            }
        }
        self.observe_keys(scan, keys.into_iter());
        self.frontier = Some(hour);
        self.supplying = hourly;
        self.supplying_hour = Some(read_hour);
    }

    fn materialize(&mut self, scan: &Scan, key: PairKey, hour: i32) {
        if let Some((_, _, index)) = &self.current[scan.pairs[&key].ordinal]
            && let Some(last) = self.reads[*index].last_mut()
        {
            last.last_hour = hour;
        }
        let (first, second, _) = key;
        let (a, b) = (scan.identities[first], scan.identities[second]);
        let identities = (a.min(b), a.max(b));
        if let Some(last) = self
            .coverage
            .get_mut(&identities)
            .and_then(|runs| runs.last_mut())
        {
            last.last_hour = hour;
        }
    }
    pub(super) fn seal(&mut self) {
        let Some(hour) = self.frontier else {
            return;
        };
        for (index, last) in self.last_tags.iter_mut().enumerate() {
            if self.live.get(index) == Some(&true)
                && let Some((_, at)) = last
            {
                *at = hour;
            }
        }
        for (_, _, index) in self.current.iter().flatten() {
            if let Some(last) = self.reads[*index].last_mut() {
                last.last_hour = hour;
            }
        }
        for runs in self.coverage.values_mut() {
            if let Some(last) = runs.last_mut() {
                last.last_hour = hour;
            }
        }
    }
    fn observe_keys(&mut self, scan: &Scan, keys: impl Iterator<Item = PairKey>) {
        let hour = scan.hour.expect("pair observation");
        let read_hour = scan.last_read.expect("pair supplying read");
        for key @ (first, second, start) in keys {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.history_visits += 1;
            }
            let pair = &scan.pairs[&key];
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
            let incomplete = scan.countries[first].at_least || scan.countries[second].at_least;
            let coverage = self
                .coverage
                .entry((scan.identities[first], scan.identities[second]))
                .or_default();
            let live = [first_live, second_live];
            if let Some(last) = coverage.last_mut()
                && (last.last_hour == hour || last.last_hour.checked_add(1) == Some(hour))
                && last.live == live
                && last.incomplete == incomplete
            {
                last.last_hour = hour;
            } else {
                coverage.push(HolderRun {
                    first_hour: hour,
                    last_hour: hour,
                    live,
                    incomplete,
                });
            }
            let (first_tag_hour, second_tag_hour) = (
                if first_live { hour } else { *first_tag_hour },
                if second_live { hour } else { *second_tag_hour },
            );
            let [first_taken, second_taken] = values;
            let hourly = read_hour == hour;
            let relation_hour = scan.relation_hour(pair);
            let relation_hourly = relation_hour == hour;
            let runs = &mut self.reads[index];
            if let Some(last) = runs.last_mut()
                && last.last_hour.checked_add(1) == Some(hour)
                && last.first_taken == first_taken
                && last.second_taken == second_taken
                && last.incomplete == incomplete
                && last.hourly == hourly
                && (hourly || last.read_hour == read_hour)
                && last.relation_hourly == relation_hourly
                && (relation_hourly || last.relation_read_hour == relation_hour)
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
                    relation_read_hour: relation_hour,
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
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        _context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        Self::validate(params).map_err(io::Error::other)?;
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse().expect("validated branch");
        let replay = source.replay();
        check_cadence(cadence::cadence_in(TableId::WarRelations, replay.format))?;
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
            dashboard_core::blocks::scan_blocks(
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
        build_window(
            params,
            &mut reply,
            &cached.holders,
            cached.pairs.runs(),
            cancelled,
        )?;
        check_reply_bytes(json_bytes(&reply)?)?;
        Ok(reply)
    }
}

/// Selection and native-lifetime summation are shared by the reference and
/// the stored unfiltered block records.
fn build_window<'a>(
    params: &CasualtyPairsParams,
    reply: &mut CasualtyPairsReply,
    holders: &BTreeMap<String, HashSet<Identity>>,
    records: impl Iterator<Item = (&'a Key, &'a [PairRead])>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    // The window answered: the requested one, or the prepared prefix's newest.
    let span = reply
        .windows
        .get(
            reply
                .window
                .ok_or_else(|| dashboard_core::invalid("Missing casualty window"))?,
        )
        .ok_or_else(|| dashboard_core::invalid("Unknown casualty window"))?;
    // Manual country picks follow their native identities through renames.
    let selected: HashSet<_> = params
        .tags
        .iter()
        .filter_map(|tag| holders.get(tag))
        .flatten()
        .copied()
        .collect();
    let mut pairs: BTreeMap<_, Vec<&[PairRead]>> = BTreeMap::new();
    for ((first, second, a, b, _), runs) in records {
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
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub(super) struct Window {
    pub(super) observed: Vec<HourSpan>,
    pub(super) records: Vec<(Key, Vec<PairRead>)>,
}
impl PairHistory {
    pub(super) fn buffered_bytes(&self) -> usize {
        self.reads
            .iter()
            .map(|runs| runs.len() * std::mem::size_of::<PairRead>())
            .sum()
    }
    #[cfg(test)]
    pub(super) fn publish_window(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        ordinal: u64,
        span: HourSpan,
        observed: &[HourSpan],
    ) -> io::Result<u64> {
        let window = self.window(span, observed);
        let count = window.records.len() as u64;
        dashboard_base::prepared::series::write_reply(
            writer,
            &format!("casualty-pairs-{branch}"),
            ordinal,
            &window,
        )?;
        Ok(count)
    }
    /// The block window `span` publishes: every lifetime's runs within it
    /// and the observed reads it covers. Only each lifetime's open run, whose
    /// original boundary supplies breakdown dates before the next window,
    /// survives it.
    pub(super) fn window(&mut self, span: HourSpan, observed: &[HourSpan]) -> Window {
        self.seal();
        let records: Vec<_> = self
            .runs()
            .filter_map(|(key, runs)| {
                let first = runs.partition_point(|run| run.last_hour < span.first_hour);
                let runs: Vec<_> = runs[first..]
                    .iter()
                    .take_while(|run| run.first_hour <= span.last_hour)
                    .map(|run| PairRead {
                        last_hour: run.last_hour.min(span.last_hour),
                        ..run.clone()
                    })
                    .collect();
                (!runs.is_empty()).then(|| (key.clone(), runs))
            })
            .collect();
        let window = Window {
            records,
            observed: observed
                .iter()
                .filter_map(|read| {
                    let first_hour = read.first_hour.max(span.first_hour);
                    let last_hour = read.last_hour.min(span.last_hour);
                    (first_hour <= last_hour).then_some(HourSpan {
                        first_hour,
                        last_hour,
                    })
                })
                .collect(),
        };
        // The open run's original boundary supplies breakdown dates before
        // the next window. Closed history does not survive publication.
        for runs in &mut self.reads {
            if runs.len() > 1 {
                runs.drain(..runs.len() - 1);
            }
        }
        window
    }
    /// Whether hours from `first` continue the hour this history observed
    /// last, as a supplying hourly read with every join in place: then
    /// observing a later hour that changes no relation, endpoint or read
    /// mode only moves its frontier ([`Self::advance_quiet`]).
    pub(super) fn quiet_continues(&self, first: i32) -> bool {
        self.joined
            && self.supplying
            && self
                .frontier
                .is_some_and(|frontier| frontier.checked_add(1) == Some(first))
    }
    /// Hours through `last` that [`Self::quiet_continues`] allows, each an
    /// hourly supplying read changing nothing: in constant time.
    pub(super) fn advance_quiet(&mut self, last: i32) {
        self.frontier = Some(last);
        self.supplying = true;
        self.supplying_hour = Some(last);
    }
    /// Whether this history, having observed a block's first hour `first` on
    /// the carried scan, can take `block`'s, observed from `first` on a scan of
    /// the block alone ([`Self::carry`]): it observed `first` as an hourly
    /// supplying read with every join in place, each lifetime and holder pair
    /// `block` met at `first` has its last run here ending there, and each
    /// lifetime `block` met later is new here.
    pub(super) fn follows(&self, block: &PairHistory, first: i32) -> bool {
        self.joined
            && self.supplying
            && self.frontier == Some(first)
            && block.keys.iter().all(|(key, &index)| {
                let Some(run) = block.reads.get(index).and_then(|runs| runs.first()) else {
                    return false;
                };
                match self.keys.get(key) {
                    Some(&own) => {
                        run.first_hour == first
                            && self
                                .reads
                                .get(own)
                                .and_then(|runs| runs.last())
                                .is_some_and(|last| last.last_hour == first)
                    }
                    None => run.first_hour > first,
                }
            })
            && block.coverage.iter().all(|(pair, runs)| {
                runs.first().is_some_and(|run| {
                    run.first_hour > first
                        || (run.first_hour == first
                            && self
                                .coverage
                                .get(pair)
                                .and_then(|runs| runs.last())
                                .is_some_and(|last| last.last_hour == first))
                })
            })
    }
    /// Take `block`'s history after the block's first hour `first`, which
    /// [`Self::follows`] allows. Each lifetime's and holder pair's runs after
    /// the one they had at `first` continue this history's; a holder pair
    /// `block` met first later continues its last run here as observing it
    /// then does (the run reaches the hour before, and goes on when it
    /// agrees). Every run is marked incomplete as `scan` (the carried scan)
    /// marks its endpoints, which the block's own scan cannot know and which
    /// no hour after `first` changes. Lifetimes new here are numbered as this
    /// history meets them: by hour, then in `scan`'s relation key order.
    /// `ordinals` holds each of `block`'s relation ordinals with `scan`'s,
    /// and its key there.
    pub(super) fn carry(
        &mut self,
        block: PairHistory,
        first: i32,
        scan: &Scan,
        ordinals: &[(usize, usize, PairKey)],
    ) -> io::Result<()> {
        let PairHistory {
            coverage,
            keys,
            mut reads,
            current,
            frontier,
            supplying,
            supplying_hour,
            ..
        } = block;
        let incomplete = |identity: &Identity| {
            scan.indices
                .get(identity)
                .is_some_and(|&index| scan.countries[index].at_least)
        };
        for (pair, mut runs) in coverage {
            let flag = incomplete(&pair.0) || incomplete(&pair.1);
            for run in &mut runs {
                run.incomplete = flag;
            }
            let own = self.coverage.entry(pair).or_default();
            let mut runs = runs.into_iter();
            if let Some(run) = runs.next() {
                match own.last_mut() {
                    Some(last) if run.first_hour == first => last.last_hour = run.last_hour,
                    Some(last) => {
                        last.last_hour = run.first_hour - 1;
                        if last.live == run.live && last.incomplete == run.incomplete {
                            last.last_hour = run.last_hour;
                        } else {
                            own.push(run);
                        }
                    }
                    None => own.push(run),
                }
            }
            own.extend(runs);
        }
        // Each of the block's lifetimes by its index there, and this scan's
        // relation key of each one current there.
        let mut named: Vec<Option<Key>> = vec![None; reads.len()];
        for (key, &index) in &keys {
            if let Some(slot) = named.get_mut(index) {
                *slot = Some(key.clone());
            }
        }
        let mut relations: BTreeMap<usize, PairKey> = BTreeMap::new();
        for &(ordinal, _, key) in ordinals {
            if let Some(Some((_, _, index))) = current.get(ordinal) {
                relations.insert(*index, key);
            }
        }
        let mut new = Vec::new();
        for (key, index) in keys {
            let mut runs = std::mem::take(&mut reads[index]);
            let flag = incomplete(&key.2) || incomplete(&key.3);
            for run in &mut runs {
                run.incomplete = flag;
            }
            match self.keys.get(&key) {
                Some(&own) => {
                    let own = &mut self.reads[own];
                    let mut runs = runs.into_iter();
                    if let Some(run) = runs.next() {
                        match own.last_mut() {
                            Some(last) => last.last_hour = run.last_hour,
                            None => own.push(run),
                        }
                    }
                    own.extend(runs);
                }
                None => {
                    let relation = relations.get(&index).copied().ok_or_else(|| {
                        dashboard_core::invalid("A casualty block's lifetime has no relation")
                    })?;
                    let met = runs.first().map_or(first, |run| run.first_hour);
                    new.push((met, relation, key, runs));
                }
            }
        }
        new.sort_unstable_by_key(|(met, relation, _, _)| (*met, *relation));
        for (_, _, key, runs) in new {
            self.keys.insert(key, self.reads.len());
            self.reads.push(runs);
        }
        // The relations' current lifetimes, and the joins of those first
        // held in the block, as observing each hour leaves them.
        let before = self.current.len();
        self.current.resize(scan.pairs.len(), None);
        self.adjacent.resize_with(scan.identities.len(), Vec::new);
        let tag = |identity: Identity, held: &Arc<str>| -> Arc<str> {
            scan.indices
                .get(&identity)
                .and_then(|&index| self.last_tags.get(index))
                .and_then(Option::as_ref)
                .filter(|(tag, _)| **tag == **held)
                .map_or_else(|| held.clone(), |(tag, _)| tag.clone())
        };
        let mut joined = Vec::new();
        for &(ordinal, own, key) in ordinals {
            let Some(Some((_, _, index))) = current.get(ordinal) else {
                continue;
            };
            let Some(Some(name)) = named.get(*index) else {
                continue;
            };
            let Some(&carried) = self.keys.get(name) else {
                continue;
            };
            joined.push((own, (tag(name.2, &name.0), tag(name.3, &name.1), carried)));
            if own >= before {
                self.adjacent[key.0].push(key);
                self.adjacent[key.1].push(key);
            }
        }
        for (own, current) in joined {
            if let Some(slot) = self.current.get_mut(own) {
                *slot = Some(current);
            }
        }
        self.frontier = frontier;
        self.supplying = supplying;
        self.supplying_hour = supplying_hour;
        Ok(())
    }
}

pub fn prepared_query(
    source: &Source,
    params: &CasualtyPairsParams,
    content: Option<&dashboard_core::config::ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    use dashboard_base::prepared::series;
    CasualtyPairs::validate(params).map_err(dashboard_core::invalid)?;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid casualty branch"))?;
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|recorded| recorded.id == branch)
    {
        return Err(dashboard_core::invalid("Unknown casualty branch"));
    }
    let (store, key) = dashboard_base::prepared::files(source, content)?;
    let (_, catalogue) =
        series::read_catalogue::<prepared::Catalogue>(&store, &key, "casualty", branch)?
            .unwrap_or_default();
    let windows = source.block_windows(branch).to_vec();
    let available = catalogue.pair_available;
    let prefix = |requested: Option<i32>| {
        dashboard_base::prepared::prefix::read_or_empty(
            source,
            content,
            "casualty-pairs",
            branch,
            requested,
            cancelled,
        )
    };
    // While preparation runs, a reply its pages cannot answer yet answers
    // from the prepared prefix, naming the hours after it pending.
    let initial = prefix(None)?;
    let mut marked = (!initial.coverage.complete && params.window.is_none()).then_some(initial);
    let mut reply = CasualtyPairsReply {
        branch_id: params.branch_id.clone(),
        available,
        windows,
        window: params.window,
        observed: Vec::new(),
        pairs: Vec::new(),
        has_gap: false,
        warnings: Vec::new(),
    };
    if let Some(ordinal) = params.window {
        if ordinal >= reply.windows.len() {
            return Err(dashboard_core::invalid("Unknown casualty window"));
        }
        let section = format!("casualty-pairs-{branch}");
        let window = match series::read_reply::<Window>(&store, &key, &section, ordinal as u64)? {
            Some(window) => window,
            None => {
                // A window not yet published: each pair counter is a lifetime
                // counter, so the prefix's newest window shows the flows at
                // its last hour, exactly; the hours through the requested
                // window's end are pending.
                let prefix = prefix(Some(reply.windows[ordinal].last_hour))?;
                let Some(newest) = reply.windows[..=ordinal]
                    .iter()
                    .rposition(|window| window.last_hour <= prefix.coverage.last_hour)
                else {
                    let mut value = serde_json::to_value(reply)?;
                    prefix.mark(&mut value)?;
                    return Ok(value);
                };
                let Some(window) =
                    series::read_reply::<Window>(&store, &key, &section, newest as u64)?
                else {
                    return Ok(serde_json::Value::Null);
                };
                reply.window = Some(newest);
                marked = Some(prefix);
                window
            }
        };
        reply.observed = window.observed;
        reply.has_gap = catalogue.incomplete;
        build_window(
            params,
            &mut reply,
            &catalogue.holders,
            window
                .records
                .iter()
                .map(|(key, runs)| (key, runs.as_slice())),
            cancelled,
        )?;
    }
    check_reply_bytes(json_bytes(&reply)?)?;
    let mut value = serde_json::to_value(reply)?;
    if let Some(prefix) = marked {
        prefix.mark(&mut value)?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::prepared::series::tests::Fixture;

    #[test]
    fn unfiltered_windows_match_parameter_selection_after_resume_and_append() {
        let mut fixture = Fixture::new();
        let mut scan = Scan::new();
        let a = scan.country((1, 10));
        let b = scan.country((2, 20));
        let replacement = scan.country((1, 11));
        let mut reference = PairHistory::default();
        let mut prepared = PairHistory::default();
        let windows = vec![
            HourSpan {
                first_hour: 100,
                last_hour: 101,
            },
            HourSpan {
                first_hour: 103,
                last_hour: 104,
            },
            HourSpan {
                first_hour: 105,
                last_hour: 106,
            },
        ];
        for (ordinal, span) in windows.iter().enumerate() {
            for hour in span.first_hour..=span.last_hour {
                let first = if hour >= 105 { replacement } else { a };
                let tag = if hour >= 103 {
                    "mod renamed"
                } else {
                    "mod original"
                };
                let live = BTreeMap::from([(tag.into(), first), ("mod enemy".into(), b)]);
                let changed = scan.live != live;
                scan.begin_hour(hour, &live, changed, true);
                scan.relation(
                    first,
                    b,
                    if hour >= 105 { 104 } else { 90 },
                    [i64::from(hour) - 102, -i64::from(hour), 0, 0],
                );
                scan.end_hour(true);
                reference.observe(&scan);
                prepared.observe(&scan);
            }
            let mut writer = if ordinal == 2 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            prepared
                .publish_window(&mut writer, 1, ordinal as u64, *span, &scan.observed)
                .unwrap();
            writer.commit().unwrap();
            assert!(prepared.reads.iter().all(|runs| runs.len() <= 1));
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        }
        for (ordinal, span) in windows.iter().enumerate() {
            let window: Window = dashboard_base::prepared::series::read_reply(
                &fixture.store,
                &fixture.key,
                "casualty-pairs-1",
                ordinal as u64,
            )
            .unwrap()
            .unwrap();
            for (tags, between) in [
                (Vec::new(), false),
                (vec!["mod original".into()], false),
                (vec!["mod original".into()], true),
                (vec!["mod original".into(), "mod enemy".into()], true),
                (vec!["mod renamed".into()], false),
            ] {
                let params = CasualtyPairsParams {
                    branch_id: "1".into(),
                    tags,
                    between,
                    window: Some(ordinal),
                };
                let reply = || CasualtyPairsReply {
                    branch_id: "1".into(),
                    available: true,
                    windows: windows.clone(),
                    window: Some(ordinal),
                    observed: scan
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
                        .collect(),
                    pairs: Vec::new(),
                    has_gap: true,
                    warnings: Vec::new(),
                };
                let mut expected = reply();
                build_window(
                    &params,
                    &mut expected,
                    &scan.holders,
                    reference.runs(),
                    &AtomicBool::new(false),
                )
                .unwrap();
                let mut actual = reply();
                actual.observed.clone_from(&window.observed);
                build_window(
                    &params,
                    &mut actual,
                    &scan.holders,
                    window
                        .records
                        .iter()
                        .map(|(key, runs)| (key, runs.as_slice())),
                    &AtomicBool::new(false),
                )
                .unwrap();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(expected).unwrap(),
                    "window {ordinal}, tags {:?}, between {}",
                    params.tags,
                    params.between
                );
            }
        }
    }

    #[test]
    fn unfiltered_window_preserves_five_thousand_countries_without_a_pair_cap() {
        let fixture = Fixture::new();
        let mut scan = Scan::new();
        let live: BTreeMap<_, _> = (0..5000)
            .map(|index| {
                (
                    format!("mod country {index}"),
                    scan.country((index, index + 100_000)),
                )
            })
            .collect();
        let mut history = PairHistory::default();
        for hour in 100..=101 {
            scan.begin_hour(hour, &live, true, true);
            for pair in 0..2500 {
                scan.relation(pair * 2, pair * 2 + 1, 0, [i64::MAX, i64::MIN, 0, 0]);
            }
            scan.end_hour(true);
            history.observe(&scan);
        }
        let span = HourSpan {
            first_hour: 100,
            last_hour: 101,
        };
        let mut writer = fixture.writer();
        assert_eq!(
            history
                .publish_window(&mut writer, 1, 0, span, &scan.observed)
                .unwrap(),
            2500
        );
        writer.commit().unwrap();
        let window: Window = dashboard_base::prepared::series::read_reply(
            &fixture.store,
            &fixture.key,
            "casualty-pairs-1",
            0,
        )
        .unwrap()
        .unwrap();
        assert_eq!(window.records.len(), 2500);
        assert!(
            window
                .records
                .iter()
                .all(|(_, runs)| runs.len() == 1 && runs[0].last_hour == 101)
        );
        let params = CasualtyPairsParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            between: false,
            window: Some(0),
        };
        let mut reply = CasualtyPairsReply {
            branch_id: "1".into(),
            available: true,
            windows: vec![span],
            window: Some(0),
            observed: window.observed,
            pairs: Vec::new(),
            has_gap: false,
            warnings: Vec::new(),
        };
        build_window(
            &params,
            &mut reply,
            &scan.holders,
            window
                .records
                .iter()
                .map(|(key, runs)| (key, runs.as_slice())),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reply.pairs.len(), 2500);
        assert!(
            reply
                .pairs
                .iter()
                .all(|pair| pair.runs[0].first_taken == i64::MAX.to_string()
                    && pair.runs[0].second_taken == i64::MIN.to_string())
        );
    }

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
        use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
        use replay_data::{
            rows::CountryLinkRow, rows::CountryRow, rows::TableRows, rows::WarRelationRow,
        };
        use replay_frame::frame::TableView;
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
            dashboard_base::source::SourceKey {
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
