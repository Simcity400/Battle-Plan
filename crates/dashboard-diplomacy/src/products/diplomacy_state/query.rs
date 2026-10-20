//! Prepared selected-page queries; only a reached opinion snapshot reads recording frames.
use super::{
    block,
    prepared::{self, Catalogue, Prepared, country_key, pair_key},
    *,
};
use dashboard_base::{
    Source, catalogue,
    prepared::{
        prefix::Prefix,
        query::progressive,
        series::{self, Reader, Run},
    },
    products::{ContentOptions, Product, check_reply_budget},
};
use dashboard_text::metadata::check_cancel;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

type Keys = BTreeMap<String, u64>;
fn value(reader: &mut Reader<'_>, keys: &Keys, name: &str, hour: i32) -> io::Result<Option<Run>> {
    keys.get(name)
        .map(|&key| reader.value_at(key, hour))
        .transpose()
        .map(Option::flatten)
}
fn memory(scan: &Prepared, name: &str, hour: i32) -> Option<Run> {
    scan.keys
        .get(name)
        .and_then(|&key| scan.series.state(key))
        .and_then(|state| state.last)
        .filter(|run| run.first_hour <= hour && run.last_hour >= hour)
}
fn selected_at(
    countries: &BTreeSet<DiplomacyCountry>,
    tag: &str,
    hour: i32,
    at: &mut impl FnMut(&str, i32) -> io::Result<Option<Run>>,
) -> io::Result<Option<(DiplomacyCountry, Run)>> {
    let mut found = None;
    for country in countries
        .range(
            DiplomacyCountry {
                tag: tag.to_owned(),
                slot: 0,
                country_id: 0,
            }..,
        )
        .take_while(|country| country.tag == tag)
    {
        if let Some(run) = at(&country_key("i", country), hour)? {
            if found.is_some() {
                return Ok(None);
            }
            found = Some((country.clone(), run));
        }
    }
    Ok(found)
}
#[allow(clippy::too_many_arguments)]
fn opinion_reply(
    hour: i32,
    tags: &[String],
    keys: &Keys,
    countries: &BTreeSet<DiplomacyCountry>,
    mut at: impl FnMut(&str, i32) -> io::Result<Option<Run>>,
    prefix: &Prefix,
    snapshot: bool,
    cancelled: &AtomicBool,
) -> io::Result<DiplomacyOpinionsReply> {
    preflight(
        tags.iter()
            .map(|tag| tag.len() + std::mem::size_of::<DiplomacyOwner>())
            .sum(),
    )?;
    let mut targets = BTreeMap::new();
    let mut target_start = i32::MIN;
    let mut targets_known = true;
    for tag in tags {
        check_cancel(cancelled)?;
        if let Some((country, run)) = selected_at(countries, tag, hour, &mut at)? {
            target_start = target_start.max(run.first_hour);
            targets.insert(country.tag.clone(), country);
        } else {
            targets_known = false;
        }
    }
    let mut unresolved = at("unresolved", hour)?.map_or(0, |run| run.value as usize);
    let mut owners = Vec::with_capacity(tags.len());
    let mut reply_bytes = tags
        .iter()
        .map(|tag| tag.len() + std::mem::size_of::<DiplomacyOwner>())
        .sum::<usize>();
    for tag in tags {
        check_cancel(cancelled)?;
        let identity = targets.get(tag).cloned();
        let read = identity
            .as_ref()
            .map(|country| at(&country_key("o", country), hour))
            .transpose()?
            .flatten();
        let supplying = identity
            .as_ref()
            .map(|country| at(&country_key("r", country), hour))
            .transpose()?
            .flatten()
            .map(|run| run.value as i32);
        let known =
            targets_known && read.is_some_and(|read| read.value >= i128::from(target_start));
        if identity.is_none() {
            unresolved += 1;
        }
        let mut opinions = Vec::new();
        if known && let Some(owner) = identity.as_ref() {
            let start = format!("p:{}:", serde_json::to_string(owner)?);
            for (name, _) in keys.range(start.clone()..) {
                if !name.starts_with(&start) {
                    break;
                }
                let target: DiplomacyCountry = serde_json::from_str(&name[start.len()..])?;
                if targets.get(&target.tag) == Some(&target)
                    && let Some(run) = at(name, hour)?
                {
                    reply_bytes = reply_bytes.saturating_add(target.tag.len() + 80);
                    preflight(reply_bytes)?;
                    opinions.push(DiplomacyOpinion {
                        target: target.tag,
                        value: i32::try_from(run.value)
                            .map_err(|_| io::Error::other("Opinion exceeds signed i32"))?,
                    });
                }
            }
        }
        owners.push(DiplomacyOwner {
            tag: tag.clone(),
            identity,
            read_hour: supplying,
            known,
            opinions,
        });
    }
    let warnings = if unresolved > 0 {
        vec![format!(
            "Unresolved or ambiguous diplomatic opinion identities/rows (count: {unresolved})"
        )]
    } else {
        Vec::new()
    };
    let mut answered = prefix.clone();
    if snapshot {
        answered.answered(&[HourSpan {
            first_hour: hour,
            last_hour: hour,
        }]);
    }
    Ok(DiplomacyOpinionsReply {
        hour,
        owners,
        unresolved,
        snapshot,
        coverage: prefix.coverage.clone(),
        pending: answered.pending,
        warnings,
    })
}
fn preflight(bytes: usize) -> io::Result<()> {
    if bytes > dashboard_base::products::MAX_PRODUCT_REPLY_BYTES {
        Err(io::Error::other(
            "Diplomacy selection exceeds the complete reply budget",
        ))
    } else {
        Ok(())
    }
}

pub fn query(
    source: &Source,
    params: &DiplomacyParams,
    content: Option<&ContentOptions>,
    store: &store::Store,
    key: &store::SourceKey,
    prefix: Prefix,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    DiplomacyState::validate(params).map_err(dashboard_core::invalid)?;
    check_cancel(cancelled)?;
    let branch = params.branch();
    let (keys, meta) =
        series::read_catalogue::<Catalogue>(store, key, prepared::FAMILY_NAME, branch)?
            .unwrap_or_default();
    let mut reader = Reader::new(store, key, prepared::section(branch), cancelled)?;
    reader.through_hour(prefix.coverage.last_hour)?;
    let query = Query {
        source,
        content,
        keys: &keys,
        meta: &meta,
        prefix: &prefix,
        cancelled,
    };
    let reply = match params {
        DiplomacyParams::Opinions { hour, tags, .. } => {
            DiplomacyReply::Opinions(query.opinions(&mut reader, branch, *hour, tags)?)
        }
        DiplomacyParams::Pair {
            first,
            second,
            hour,
            ..
        } => DiplomacyReply::Pair(pair(
            source,
            branch,
            first,
            second,
            *hour,
            store,
            key,
            &keys,
            &meta,
            &mut reader,
            &prefix,
            cancelled,
        )?),
        DiplomacyParams::Pacts { tags, .. } => {
            DiplomacyReply::Pacts(query.pacts(&mut reader, tags.as_deref())?)
        }
        DiplomacyParams::Proposals { tags, .. } => {
            DiplomacyReply::Proposals(query.proposals(&mut reader, tags.as_deref())?)
        }
    };
    check_cancel(cancelled)?;
    check_reply_budget(&reply)?;
    Ok(serde_json::to_value(reply)?)
}

struct Query<'a> {
    source: &'a Source,
    content: Option<&'a ContentOptions>,
    keys: &'a Keys,
    meta: &'a Catalogue,
    prefix: &'a Prefix,
    cancelled: &'a AtomicBool,
}
impl Query<'_> {
    fn opinions(
        &self,
        reader: &mut Reader<'_>,
        branch: u64,
        hour: i32,
        tags: &[String],
    ) -> io::Result<DiplomacyOpinionsReply> {
        if hour > self.prefix.coverage.last_hour {
            let mut scan = snapshot(self.source, branch, hour, self.cancelled)?;
            scan.series.seal();
            opinion_reply(
                hour,
                tags,
                &scan.keys,
                &scan.catalogue.countries,
                |name, hour| Ok(memory(&scan, name, hour)),
                self.prefix,
                true,
                self.cancelled,
            )
        } else {
            opinion_reply(
                hour,
                tags,
                self.keys,
                &self.meta.countries,
                |name, hour| value(reader, self.keys, name, hour),
                self.prefix,
                false,
                self.cancelled,
            )
        }
    }
    fn inventory<T: serde::de::DeserializeOwned>(
        &self,
        reader: &mut Reader<'_>,
        queue: bool,
        tags: Option<&[String]>,
    ) -> io::Result<(Vec<T>, usize)> {
        let entries = entries(self.meta, queue, tags)?;
        preflight(entries.len() * std::mem::size_of::<T>())?;
        let mut rows = Vec::with_capacity(entries.len());
        let mut reply_bytes = 0usize;
        for entry in entries {
            check_cancel(self.cancelled)?;
            if let Some(token) = entry.unresolved_kind {
                return Err(io::Error::other(format!(
                    "Recorded diplomacy type {token} has no supplying LexicalTokens spelling (count: 1)"
                )));
            }

            if let Some((_, row)) =
                reader.latest_event(entry.key, self.prefix.coverage.last_hour)?
            {
                reply_bytes = reply_bytes.saturating_add(serde_json::to_vec(&row)?.len());
                preflight(reply_bytes)?;
                rows.push(serde_json::from_value::<T>(row)?);
            }
        }
        Ok((rows, reply_bytes))
    }
    fn kinds(&self, pacts: &[DiplomacyPact]) -> (Vec<DiplomacyKind>, Vec<String>) {
        let source = self.source;
        let content = self.content;
        let meta = self.meta;
        let mut counts = BTreeMap::<String, usize>::new();
        for pact in pacts {
            *counts.entry(pact.kind.clone()).or_default() += 1;
        }
        let catalogue = content
            .map(|options| catalogue::catalogue(source, options))
            .transpose();
        let mut kinds = Vec::new();
        let mut warnings = Vec::new();
        for (kind, count) in counts {
            let token = meta.kinds.get(&kind).copied().unwrap_or(0);
            let result = installed_content_core::diplomacy::relation_title_key(token, &kind)
                .map_err(|reason| reason.replace("(count: 1)", &format!("(count: {count})")))
                .and_then(|key| match &catalogue {
                    Ok(Some(catalogue)) => catalogue
                        .localised(key, catalogue::View::Plain)
                        .ok_or_else(|| {
                            format!("Relation {kind}: title {key} is unavailable (count: {count})")
                        }),
                    Ok(None) => Err(format!(
                        "Relation {kind}: recorded content is not configured (count: {count})"
                    )),
                    Err(error) => Err(format!("Relation {kind}: {error} (count: {count})")),
                });
            let (name, reason) = match result {
                Ok(name) => (Some(name), None),
                Err(reason) => {
                    warnings.push(format!(
                        "Relation title unavailable for {count} occurrences: {reason}"
                    ));
                    (None, Some(reason))
                }
            };
            kinds.push(DiplomacyKind {
                kind,
                name,
                reason,
                count,
            });
        }
        (kinds, warnings)
    }
    fn tuple_reads(
        &self,
        reader: &mut Reader<'_>,
        tags: Option<&[String]>,
        pacts: &[DiplomacyPact],
        reply_bytes: &mut usize,
    ) -> io::Result<Vec<DiplomacyReadRun>> {
        let owners: BTreeSet<_> = pacts
            .iter()
            .filter_map(|pact| pact.identities.first()?.as_ref())
            .collect();
        let mut reads = Vec::new();
        for country in &self.meta.countries {
            if tags.is_none_or(|tags| tags.contains(&country.tag)) || owners.contains(country) {
                for run in read_runs(reader, self.keys, &country_key("t", country), self.prefix)? {
                    check_cancel(self.cancelled)?;
                    append_read(&mut reads, Some(country), &run, reply_bytes)?;
                }
            }
        }
        Ok(reads)
    }
    fn pacts(
        &self,
        reader: &mut Reader<'_>,
        tags: Option<&[String]>,
    ) -> io::Result<DiplomacyPactsReply> {
        let (pacts, mut reply_bytes) = self.inventory::<DiplomacyPact>(reader, false, tags)?;
        let (kinds, mut warnings) = self.kinds(&pacts);
        let unknown = pacts
            .iter()
            .filter(|p| {
                p.identities.iter().any(Option::is_none)
                    || p.cancelled.is_none()
                    || p.retired.is_none()
            })
            .count();
        if unknown > 0 {
            warnings.push(format!(
                "Pacts with unresolved endpoints or non-Boolean raw flags (count: {unknown})"
            ));
        }

        let reads = self.tuple_reads(reader, tags, &pacts, &mut reply_bytes)?;
        let unobserved = unavailable(reader, self.keys, "tuples:coverage", self.prefix)?;
        Ok(DiplomacyPactsReply {
            kinds,
            pacts,
            reads,
            unobserved,
            coverage: self.prefix.coverage.clone(),
            pending: self.prefix.pending.clone(),
            warnings,
        })
    }
    fn refuse_duplicate_queue_handles(&self, reader: &mut Reader<'_>) -> io::Result<()> {
        let refused = read_runs(reader, self.keys, "queue:duplicates", self.prefix)?;
        if !refused.is_empty() {
            let count: i128 = refused.iter().map(|run| run.value).sum();
            return Err(io::Error::other(format!(
                "Ambiguous incoming diplomatic queue handles (count: {count})"
            )));
        }
        Ok(())
    }
    fn queue_reads(
        &self,
        reader: &mut Reader<'_>,
        proposals: &[DiplomacyProposal],
        reply_bytes: &mut usize,
    ) -> io::Result<Vec<DiplomacyReadRun>> {
        let runs = read_runs(reader, self.keys, "queue:coverage", self.prefix)?;
        let mut reads = Vec::new();
        // Whole-table reads also prove empty and unresolved owner queues.
        for run in &runs {
            check_cancel(self.cancelled)?;
            append_read(&mut reads, None, run, reply_bytes)?;
        }
        let owners: BTreeSet<_> = proposals
            .iter()
            .filter_map(|proposal| proposal.identities.first()?.as_ref())
            .collect();
        // A whole-queue read belongs only to the identity present at its supplying hour.
        for owner in owners {
            check_cancel(self.cancelled)?;
            let identities = read_runs(reader, self.keys, &country_key("i", owner), self.prefix)?;
            for run in &runs {
                check_cancel(self.cancelled)?;
                let read = run.value as i32;
                let at = identities.partition_point(|identity| identity.last_hour < read);
                let Some(identity) = identities
                    .get(at)
                    .filter(|identity| identity.first_hour <= read)
                else {
                    continue;
                };
                let mut proven = *run;
                proven.first_hour = proven.first_hour.max(identity.first_hour);
                proven.last_hour = proven.last_hour.min(identity.last_hour);
                if proven.first_hour <= proven.last_hour {
                    append_read(&mut reads, Some(owner), &proven, reply_bytes)?;
                }
            }
        }
        Ok(reads)
    }
    fn proposals(
        &self,
        reader: &mut Reader<'_>,
        tags: Option<&[String]>,
    ) -> io::Result<DiplomacyProposalsReply> {
        self.refuse_duplicate_queue_handles(reader)?;
        let (proposals, mut reply_bytes) =
            self.inventory::<DiplomacyProposal>(reader, true, tags)?;
        let count = proposals.len();
        let mut warnings = Vec::new();
        if count > 0 {
            warnings.push(format!(
                "Diplomatic proposal kind and outcome are not recorded (count: {count})"
            ));
        }
        let unknown = proposals
            .iter()
            .filter(|p| p.identities.iter().any(Option::is_none))
            .count();
        if unknown > 0 {
            warnings.push(format!(
                "Proposals with unresolved recorded country roles (count: {unknown})"
            ));
        }

        let reads = self.queue_reads(reader, &proposals, &mut reply_bytes)?;
        let unobserved = unavailable(reader, self.keys, "queue:coverage", self.prefix)?;
        Ok(DiplomacyProposalsReply {
            proposals,
            reads,
            unobserved,
            coverage: self.prefix.coverage.clone(),
            pending: self.prefix.pending.clone(),
            warnings,
        })
    }
}
fn append_read(
    reads: &mut Vec<DiplomacyReadRun>,
    owner: Option<&DiplomacyCountry>,
    run: &Run,
    reply_bytes: &mut usize,
) -> io::Result<()> {
    *reply_bytes = reply_bytes.saturating_add(owner.map_or(0, |owner| owner.tag.len()) + 160);
    preflight(*reply_bytes)?;
    reads.push(DiplomacyReadRun {
        owner: owner.cloned(),
        first_hour: run.first_hour,
        last_hour: run.last_hour,
        read_hour: run.value as i32,
    });
    Ok(())
}
fn entries<'a>(
    meta: &'a Catalogue,
    queue: bool,
    tags: Option<&[String]>,
) -> io::Result<Vec<&'a prepared::Entry>> {
    let tags = tags.map(|tags| tags.iter().collect::<BTreeSet<_>>());
    let mut selected = Vec::new();
    let mut ambiguous = 0;
    for entry in meta.entries.iter().filter(|entry| entry.queue == queue) {
        if tags.as_ref().is_none_or(|tags| {
            entry
                .roles
                .iter()
                .flatten()
                .any(|role| tags.contains(&role.tag))
        }) {
            selected.push(entry);
        } else if tags.as_ref().is_some_and(|tags| !tags.is_empty())
            && entry
                .roles
                .iter()
                .zip(&entry.raw_roles)
                .any(|(role, raw)| role.is_none() && *raw != 0)
        {
            ambiguous += 1;
        }
    }
    if ambiguous > 0 {
        return Err(io::Error::other(format!(
            "Diplomacy selection cannot exclude unresolved recorded country roles (count: {ambiguous})"
        )));
    }
    Ok(selected)
}
pub(super) fn snapshot(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Prepared> {
    let mut scan = Prepared::default();
    let first = hour
        .saturating_sub(replay_data::cadence::STAGGER_PERIOD - 1)
        .max(
            source
                .block_windows(branch)
                .iter()
                .map(|window| window.first_hour)
                .min()
                .unwrap_or(hour),
        );
    // One cadence-bounded window is required for certified rowless carries.
    // Frames and decoded sections share the source's normal bounded caches.
    let frames = progressive::read_window(
        source,
        branch,
        first,
        hour,
        block::OPINION_TABLES,
        cancelled,
    )?;
    for frame in frames {
        check_cancel(cancelled)?;
        let changes = dashboard_base::prepared::frame_changes::FrameChanges::alone(
            &frame,
            source.replay_ref().format,
        );
        scan.observe(&frame, source.replay_ref().format, &changes, cancelled)?;
    }
    Ok(scan)
}

#[allow(clippy::too_many_arguments)]
fn pair(
    source: &Source,
    branch: u64,
    first: &str,
    second: &str,
    hour: i32,
    store: &store::Store,
    key: &store::SourceKey,
    keys: &Keys,
    meta: &Catalogue,
    reader: &mut Reader<'_>,
    prefix: &Prefix,
    cancelled: &AtomicBool,
) -> io::Result<DiplomacyPairReply> {
    let history = progressive::histories(
        store,
        key,
        branch,
        Some(&prefix.coverage),
        &BTreeMap::new(),
        &BTreeMap::new(),
        source.replay_ref(),
        &[],
        cancelled,
    )?;
    // Each tag is the holder it named when the pair was chosen; another holder of
    // the tag, earlier or later, adds nothing to this pair's history.
    let choose = |tag: &str| {
        history
            .countries
            .get(tag)
            .and_then(|c| {
                c.identities
                    .iter()
                    .find(|i| i.first_hour <= hour && hour <= i.last_hour)
            })
            .map(|i| (i.slot, i.country_id))
    };
    let a = choose(first);
    let b = choose(second);
    let selected = |native: Option<(u32, u32)>| {
        meta.countries
            .iter()
            .filter(|c| Some((c.slot, c.country_id)) == native)
            .cloned()
            .collect::<Vec<_>>()
    };
    let aa = selected(a);
    let bb = selected(b);
    let from = prefix.coverage.first_hour;
    let to = prefix.coverage.last_hour;
    let mut series = BTreeMap::<String, Vec<Run>>::new();
    let mut boundaries = BTreeSet::new();
    boundaries.insert(i64::from(from));
    boundaries.insert(i64::from(to) + 1);
    if from <= to {
        for c in aa.iter().chain(bb.iter()) {
            for prefix in ["i", "o"] {
                let name = country_key(prefix, c);
                if let Some(&key) = keys.get(&name) {
                    series.insert(name, reader.runs(key, from, to)?);
                }
            }
        }
        for (owners, target_native) in [(&aa, b), (&bb, a)] {
            for owner in owners {
                let start = format!("p:{}:", serde_json::to_string(owner)?);
                for (name, &key) in keys.range(start.clone()..) {
                    if !name.starts_with(&start) {
                        break;
                    }
                    let target: DiplomacyCountry = serde_json::from_str(&name[start.len()..])?;
                    if Some((target.slot, target.country_id)) == target_native
                        && !series.contains_key(name)
                    {
                        series.insert(name.clone(), reader.runs(key, from, to)?);
                    }
                }
            }
        }
    }
    for runs in series.values() {
        for run in runs {
            boundaries.insert(i64::from(run.first_hour));
            boundaries.insert(i64::from(run.last_hour) + 1);
        }
    }
    let mut forward = Vec::new();
    let mut reverse = Vec::new();
    let mut unobserved = Vec::new();
    let boundaries: Vec<_> = boundaries.into_iter().collect();
    let mut previous: Option<(Option<DiplomacyCountry>, Option<DiplomacyCountry>)> = None;
    for bounds in boundaries.windows(2) {
        check_cancel(cancelled)?;
        let lo = bounds[0].max(i64::from(from));
        let hi = (bounds[1] - 1).min(i64::from(to));
        if lo > hi {
            continue;
        }
        let lo = lo as i32;
        let hi = hi as i32;
        let at = |name: &str| {
            series.get(name).and_then(|runs| {
                let i = runs.partition_point(|r| r.first_hour <= lo);
                i.checked_sub(1)
                    .and_then(|i| runs.get(i))
                    .filter(|r| r.last_hour >= lo)
            })
        };
        let active = |countries: &[DiplomacyCountry]| {
            countries
                .iter()
                .find(|c| at(&country_key("i", c)).is_some())
                .cloned()
        };
        let ca = active(&aa);
        let cb = active(&bb);
        let identities = (ca.clone(), cb.clone());
        let starts = previous.as_ref() != Some(&identities);
        let direction =
            |owner: &Option<DiplomacyCountry>, target: &Option<DiplomacyCountry>| -> Option<i32> {
                let (owner, target) = (owner.as_ref()?, target.as_ref()?);
                let read = at(&country_key("o", owner))?;
                let target_run = at(&country_key("i", target))?;
                let owner_run = at(&country_key("i", owner))?;
                if read.value < i128::from(target_run.first_hour.max(owner_run.first_hour)) {
                    return None;
                }
                Some(at(&pair_key(owner, target)).map_or(0, |r| r.value as i32))
            };
        let f = direction(&ca, &cb);
        let r = direction(&cb, &ca);
        push(&mut forward, lo, hi, f, starts);
        push(&mut reverse, lo, hi, r, starts);
        if f.is_none() && r.is_none() {
            dashboard_base::products::country_groups::mark_unobserved(&mut unobserved, lo, hi);
        }
        previous = Some(identities);
    }
    Ok(DiplomacyPairReply {
        forward,
        reverse,
        unobserved,
        first: aa.iter().find(|c| c.tag == first).cloned(),
        second: bb.iter().find(|c| c.tag == second).cloned(),
        coverage: prefix.coverage.clone(),
        pending: prefix.pending.clone(),
        warnings: Vec::new(),
    })
}
fn push(runs: &mut Vec<DiplomacyRun>, first: i32, last: i32, value: Option<i32>, starts: bool) {
    if !starts
        && let Some(run) = runs.last_mut()
        && run.last_hour.checked_add(1) == Some(first)
        && run.value == value
    {
        run.last_hour = last;
        return;
    }
    let starts_segment = starts || runs.last().is_none_or(|run| run.value.is_none());
    runs.push(DiplomacyRun {
        first_hour: first,
        last_hour: last,
        value,
        starts_segment,
    });
}

fn read_runs(
    reader: &mut Reader<'_>,
    keys: &Keys,
    name: &str,
    prefix: &Prefix,
) -> io::Result<Vec<Run>> {
    let first = prefix.coverage.first_hour;
    let last = prefix.coverage.last_hour;
    if first > last {
        return Ok(Vec::new());
    }
    keys.get(name)
        .map_or_else(|| Ok(Vec::new()), |&key| reader.runs(key, first, last))
}
fn unavailable(
    reader: &mut Reader<'_>,
    keys: &Keys,
    name: &str,
    prefix: &Prefix,
) -> io::Result<Vec<HourSpan>> {
    let first = prefix.coverage.first_hour;
    let last = prefix.coverage.last_hour;
    if first > last {
        return Ok(Vec::new());
    }
    let known: Vec<_> = read_runs(reader, keys, name, prefix)?
        .into_iter()
        .map(|run| HourSpan {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
        })
        .collect();
    let mut missing = prefix.clone();
    missing.pending = vec![HourSpan {
        first_hour: first,
        last_hour: last,
    }];
    missing.answered(&known);
    Ok(missing.pending)
}
