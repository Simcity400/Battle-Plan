//! Diplomacy's replies, read from the family's series publication ([`super::emit`]) through the
//! gate's prefix: opinions at an hour, a pair's history, and the pact and proposal inventories
//! with the reads that prove them.
use super::{
    emit::{
        FAMILY, GONE, IDENTITY, KINDS, OPINION, OWNER, PACTS, PROPOSALS, QUEUE_COVERAGE,
        QUEUE_DUPLICATES, RAW_READS, TUPLE_COVERAGE, TUPLES, UNRESOLVED, country_of,
    },
    *,
};
use dashboard_base::{
    Source, catalogue,
    prepared::{engine, prefix::Prefix},
    products::{ContentOptions, check_reply_budget, country_history::RecordedHistories},
};
use dashboard_series::{
    KeyHandle, PageRead, Snapshot,
    family::{ACTORS, Directory, Scalar},
};
use dashboard_text::metadata::check_cancel;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};

/// A run as a reply reads it: clipped where it was read over a span, whole at an hour.
#[derive(Clone, Copy, Debug)]
struct Run {
    first: i32,
    last: i32,
    value: i128,
}

/// The run of `key` holding `hour`, unclipped.
fn at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<KeyHandle>,
    hour: i32,
) -> io::Result<Option<Run>> {
    let Some(key) = key else {
        return Ok(None);
    };
    Ok(engine::run_at(snapshot, key, hour)?.map(|(run, _)| Run {
        first: run.span.first,
        last: run.span.last,
        value: run.value,
    }))
}

/// `key`'s runs over the prefix's coverage, clipped to it.
fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<KeyHandle>,
    prefix: &Prefix,
) -> io::Result<Vec<Run>> {
    let (first, last) = (prefix.coverage.first_hour, prefix.coverage.last_hour);
    let Some(key) = key.filter(|_| first <= last) else {
        return Ok(Vec::new());
    };
    Ok(engine::runs(snapshot, key, first, last)?
        .into_iter()
        .map(|run| Run {
            first: run.span.first,
            last: run.span.last,
            value: run.value,
        })
        .collect())
}

/// An owner's opinion read: its supplying hour, and whether it is known.
fn read_of(run: &Run) -> (i32, bool) {
    ((run.value >> 1) as i32, run.value & 1 == 1)
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

fn corrupt() -> io::Error {
    dashboard_core::invalid("Prepared diplomacy record is not valid")
}

/// The publication's countries, each with its actor, ascending.
fn countries(directory: &Directory) -> Vec<(DiplomacyCountry, u64)> {
    let mut countries: Vec<(DiplomacyCountry, u64)> = directory
        .actors
        .iter()
        .enumerate()
        .filter_map(|(actor, described)| Some((country_of(&described.name)?, actor as u64)))
        .collect();
    countries.sort_unstable();
    countries
}

/// What a reply reads of the publication.
struct Read<'a> {
    directory: &'a Directory,
    countries: Vec<(DiplomacyCountry, u64)>,
    by_actor: HashMap<u64, usize>,
}

impl<'a> Read<'a> {
    fn new(directory: &'a Directory) -> Self {
        let countries = countries(directory);
        let by_actor = countries
            .iter()
            .enumerate()
            .map(|(at, &(_, actor))| (actor, at))
            .collect();
        Self {
            directory,
            countries,
            by_actor,
        }
    }

    fn country(&self, actor: u64) -> Option<&DiplomacyCountry> {
        Some(&self.countries[*self.by_actor.get(&actor)?].0)
    }

    fn key(&self, actor: u64, measure: u64) -> Option<KeyHandle> {
        self.directory.key(actor, measure, None)
    }

    fn family(&self, measure: u64) -> Option<KeyHandle> {
        self.directory.key(FAMILY, measure, None)
    }

    /// A role's country, none where the record names no actor.
    fn role(&self, cell: &Scalar) -> io::Result<Option<DiplomacyCountry>> {
        match cell {
            Scalar::Null => Ok(None),
            cell => cell
                .integer()
                .and_then(|actor| u64::try_from(actor).ok())
                .and_then(|actor| self.country(actor))
                .cloned()
                .map(Some)
                .ok_or_else(corrupt),
        }
    }
}

fn integer<T: TryFrom<i128>>(cell: &Scalar) -> io::Result<T> {
    cell.integer()
        .and_then(|value| T::try_from(value).ok())
        .ok_or_else(corrupt)
}

fn optional<T: TryFrom<i128>>(cell: &Scalar) -> io::Result<Option<T>> {
    match cell {
        Scalar::Null => Ok(None),
        cell => integer(cell).map(Some),
    }
}

fn boolean(cell: &Scalar) -> io::Result<bool> {
    match cell {
        Scalar::Boolean(value) => Ok(*value),
        _ => Err(corrupt()),
    }
}

fn flag(raw: u8) -> Option<bool> {
    match raw {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

fn tag(country: &Option<DiplomacyCountry>) -> Option<String> {
    country.as_ref().map(|country| country.tag.clone())
}

/// The reply before the family publishes the branch: every country unresolved, every accepted
/// hour unobserved.
pub fn unpublished(params: &DiplomacyParams, prefix: Prefix) -> io::Result<DiplomacyReply> {
    DiplomacyState::validate(params).map_err(dashboard_core::invalid)?;
    let (first, last) = (prefix.coverage.first_hour, prefix.coverage.last_hour);
    let unobserved = if first <= last {
        vec![HourSpan {
            first_hour: first,
            last_hour: last,
        }]
    } else {
        Vec::new()
    };
    let reply = match params {
        DiplomacyParams::Opinions { hour, tags, .. } => {
            let unresolved = tags.len();
            DiplomacyReply::Opinions(DiplomacyOpinionsReply {
                hour: *hour,
                owners: tags
                    .iter()
                    .map(|tag| DiplomacyOwner {
                        tag: tag.clone(),
                        identity: None,
                        read_hour: None,
                        known: false,
                        unknown_targets: Vec::new(),
                        opinions: Vec::new(),
                    })
                    .collect(),
                unresolved,
                snapshot: false,
                coverage: prefix.coverage,
                pending: prefix.pending,
                warnings: (unresolved > 0)
                    .then(|| {
                        format!(
                            "Unresolved or ambiguous diplomatic opinion identities/rows (count: {unresolved})"
                        )
                    })
                    .into_iter()
                    .collect(),
            })
        }
        DiplomacyParams::Pair { .. } => {
            let runs: Vec<DiplomacyRun> = unobserved
                .iter()
                .map(|span| DiplomacyRun {
                    first_hour: span.first_hour,
                    last_hour: span.last_hour,
                    value: None,
                    starts_segment: true,
                })
                .collect();
            DiplomacyReply::Pair(DiplomacyPairReply {
                forward: runs.clone(),
                reverse: runs,
                unobserved,
                first: None,
                second: None,
                coverage: prefix.coverage,
                pending: prefix.pending,
                warnings: Vec::new(),
            })
        }
        DiplomacyParams::Pacts { .. } => DiplomacyReply::Pacts(DiplomacyPactsReply {
            kinds: Vec::new(),
            pacts: Vec::new(),
            reads: Vec::new(),
            unobserved,
            coverage: prefix.coverage,
            pending: prefix.pending,
            warnings: Vec::new(),
        }),
        DiplomacyParams::Proposals { .. } => DiplomacyReply::Proposals(DiplomacyProposalsReply {
            proposals: Vec::new(),
            reads: Vec::new(),
            unobserved,
            coverage: prefix.coverage,
            pending: prefix.pending,
            warnings: Vec::new(),
        }),
    };
    check_reply_budget(&reply)?;
    Ok(reply)
}

/// Answers a diplomacy request from the publication through `prefix`.
#[allow(clippy::too_many_arguments)]
pub fn reply<P: PageRead>(
    source: &Source,
    params: &DiplomacyParams,
    content: Option<&ContentOptions>,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    history: Option<&RecordedHistories>,
    prefix: Prefix,
    cancelled: &AtomicBool,
) -> io::Result<DiplomacyReply> {
    DiplomacyState::validate(params).map_err(dashboard_core::invalid)?;
    check_cancel(cancelled)?;
    let read = Read::new(directory);
    let reply = match params {
        DiplomacyParams::Opinions { hour, tags, .. } => DiplomacyReply::Opinions(opinions(
            snapshot, &read, *hour, tags, &prefix, cancelled,
        )?),
        DiplomacyParams::Pair {
            first,
            second,
            hour,
            ..
        } => DiplomacyReply::Pair(pair(
            snapshot,
            &read,
            history.ok_or_else(|| io::Error::other("A pair reads the identity histories"))?,
            (first, second),
            *hour,
            &prefix,
            cancelled,
        )?),
        DiplomacyParams::Pacts { tags, .. } => DiplomacyReply::Pacts(pacts(
            source,
            content,
            snapshot,
            &read,
            tags.as_deref(),
            &prefix,
            cancelled,
        )?),
        DiplomacyParams::Proposals { tags, .. } => DiplomacyReply::Proposals(proposals(
            snapshot,
            &read,
            tags.as_deref(),
            &prefix,
            cancelled,
        )?),
    };
    check_cancel(cancelled)?;
    check_reply_budget(&reply)?;
    Ok(reply)
}

/// The one holder of `tag` whose identity holds `hour`, with its actor and its identity run's
/// first hour; none when no holder, or more than one, does.
fn selected_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    read: &Read<'_>,
    tag: &str,
    hour: i32,
) -> io::Result<Option<(DiplomacyCountry, u64, i32)>> {
    let first = read
        .countries
        .partition_point(|(country, _)| country.tag.as_str() < tag);
    let mut found = None;
    for (country, actor) in read.countries[first..]
        .iter()
        .take_while(|(country, _)| country.tag == tag)
    {
        if let Some(run) = at(snapshot, read.key(*actor, IDENTITY), hour)? {
            if found.is_some() {
                return Ok(None);
            }
            found = Some((country.clone(), *actor, run.first));
        }
    }
    Ok(found)
}

fn opinions<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    read: &Read<'_>,
    hour: i32,
    tags: &[String],
    prefix: &Prefix,
    cancelled: &AtomicBool,
) -> io::Result<DiplomacyOpinionsReply> {
    let mut reply_bytes = tags
        .iter()
        .map(|tag| tag.len() + std::mem::size_of::<DiplomacyOwner>())
        .sum::<usize>();
    preflight(reply_bytes)?;
    // Each selected tag's holder then, with the hour its identity run began.
    let mut targets = BTreeMap::new();
    for tag in tags {
        check_cancel(cancelled)?;
        if let Some(found) = selected_at(snapshot, read, tag, hour)? {
            targets.insert(found.0.tag.clone(), found);
        }
    }
    let mut unresolved =
        at(snapshot, read.family(UNRESOLVED), hour)?.map_or(0, |run| run.value as usize);
    let mut owners = Vec::with_capacity(tags.len());
    for tag in tags {
        check_cancel(cancelled)?;
        let selected = targets.get(tag);
        let owner = match selected {
            Some(&(_, actor, _)) => at(snapshot, read.key(actor, OWNER), hour)?.map(|run| read_of(&run)),
            None => None,
        };
        let supplying = owner.map(|(read, _)| read);
        let known_read = owner.and_then(|(read, known)| known.then_some(read));
        // The read proves a pair only toward a country whose identity existed at it.
        let proves = |start: i32| known_read.is_some_and(|read| read >= start);
        let known = selected.is_some_and(|&(_, _, start)| proves(start));
        let unknown_targets: BTreeSet<&str> = if known {
            tags.iter()
                .map(String::as_str)
                .filter(|target| !targets.get(*target).is_some_and(|&(_, _, start)| proves(start)))
                .collect()
        } else {
            BTreeSet::new()
        };
        if selected.is_none() {
            unresolved += 1;
        }
        let mut opinions = Vec::new();
        if known && let Some(&(_, actor, _)) = selected {
            for &(target, key) in read.directory.dimensions(actor, OPINION) {
                let Some(country) = read.country(target) else {
                    continue;
                };
                if targets
                    .get(&country.tag)
                    .is_some_and(|&(_, held, _)| held == target)
                    && !unknown_targets.contains(country.tag.as_str())
                    && let Some(run) = at(snapshot, Some(key), hour)?
                {
                    reply_bytes = reply_bytes.saturating_add(country.tag.len() + 80);
                    preflight(reply_bytes)?;
                    opinions.push(DiplomacyOpinion {
                        target: country.tag.clone(),
                        value: i32::try_from(run.value)
                            .map_err(|_| io::Error::other("Opinion exceeds signed i32"))?,
                    });
                }
            }
            opinions.sort_unstable_by(|left, right| left.target.cmp(&right.target));
        }
        reply_bytes = unknown_targets
            .iter()
            .fold(reply_bytes, |bytes, target| bytes.saturating_add(target.len() + 8));
        preflight(reply_bytes)?;
        owners.push(DiplomacyOwner {
            tag: tag.clone(),
            identity: selected.map(|(country, _, _)| country.clone()),
            read_hour: supplying,
            known,
            unknown_targets: unknown_targets.into_iter().map(str::to_owned).collect(),
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
    Ok(DiplomacyOpinionsReply {
        hour,
        owners,
        unresolved,
        snapshot: false,
        coverage: prefix.coverage.clone(),
        pending: prefix.pending.clone(),
        warnings,
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

fn pair<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    read: &Read<'_>,
    history: &RecordedHistories,
    (first, second): (&str, &str),
    hour: i32,
    prefix: &Prefix,
    cancelled: &AtomicBool,
) -> io::Result<DiplomacyPairReply> {
    // Each tag is the holder it named when the pair was chosen; another holder of the tag,
    // earlier or later, adds nothing to this pair's history.
    let choose = |tag: &str| {
        history
            .countries
            .get(tag)
            .and_then(|country| {
                country
                    .identities
                    .iter()
                    .find(|identity| identity.first_hour <= hour && hour <= identity.last_hour)
            })
            .map(|identity| (identity.slot, identity.country_id))
    };
    let (a, b) = (choose(first), choose(second));
    let selected = |native: Option<(u32, u32)>| {
        read.countries
            .iter()
            .filter(|(country, _)| Some((country.slot, country.country_id)) == native)
            .cloned()
            .collect::<Vec<_>>()
    };
    let (aa, bb) = (selected(a), selected(b));
    let (from, to) = (prefix.coverage.first_hour, prefix.coverage.last_hour);
    let mut series = HashMap::<KeyHandle, Vec<Run>>::new();
    let mut boundaries = BTreeSet::from([i64::from(from), i64::from(to) + 1]);
    if from <= to {
        for &(_, actor) in aa.iter().chain(&bb) {
            for measure in [IDENTITY, OWNER] {
                if let Some(key) = read.key(actor, measure) {
                    series.insert(key, runs(snapshot, Some(key), prefix)?);
                }
            }
        }
        for (owners, target_native) in [(&aa, b), (&bb, a)] {
            for &(_, owner) in owners.iter() {
                for &(target, key) in read.directory.dimensions(owner, OPINION) {
                    if read
                        .country(target)
                        .is_some_and(|target| Some((target.slot, target.country_id)) == target_native)
                        && !series.contains_key(&key)
                    {
                        series.insert(key, runs(snapshot, Some(key), prefix)?);
                    }
                }
            }
        }
    }
    for runs in series.values() {
        for run in runs {
            boundaries.insert(i64::from(run.first));
            boundaries.insert(i64::from(run.last) + 1);
        }
    }
    let mut forward = Vec::new();
    let mut reverse = Vec::new();
    let mut unobserved = Vec::new();
    let boundaries: Vec<_> = boundaries.into_iter().collect();
    let mut previous: Option<(Option<u64>, Option<u64>)> = None;
    for bounds in boundaries.windows(2) {
        check_cancel(cancelled)?;
        let lo = bounds[0].max(i64::from(from));
        let hi = (bounds[1] - 1).min(i64::from(to));
        if lo > hi {
            continue;
        }
        let (lo, hi) = (lo as i32, hi as i32);
        let at = |key: Option<KeyHandle>| {
            series.get(&key?).and_then(|runs| {
                let index = runs.partition_point(|run| run.first <= lo);
                index
                    .checked_sub(1)
                    .and_then(|index| runs.get(index))
                    .filter(|run| run.last >= lo)
            })
        };
        let active = |countries: &[(DiplomacyCountry, u64)]| {
            countries
                .iter()
                .find(|&&(_, actor)| at(read.key(actor, IDENTITY)).is_some())
                .map(|&(_, actor)| actor)
        };
        let identities = (active(&aa), active(&bb));
        let starts = previous != Some(identities);
        let direction = |owner: Option<u64>, target: Option<u64>| -> Option<i32> {
            let (owner, target) = (owner?, target?);
            let (supplying, known) = read_of(at(read.key(owner, OWNER))?);
            let target_run = at(read.key(target, IDENTITY))?;
            let owner_run = at(read.key(owner, IDENTITY))?;
            if !known || supplying < target_run.first.max(owner_run.first) {
                return None;
            }
            let key = read.directory.key(owner, OPINION, Some(target));
            Some(at(key).map_or(0, |run| run.value as i32))
        };
        let f = direction(identities.0, identities.1);
        let r = direction(identities.1, identities.0);
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
        first: aa
            .iter()
            .find(|(country, _)| country.tag == first)
            .map(|(country, _)| country.clone()),
        second: bb
            .iter()
            .find(|(country, _)| country.tag == second)
            .map(|(country, _)| country.clone()),
        coverage: prefix.coverage.clone(),
        pending: prefix.pending.clone(),
        warnings: Vec::new(),
    })
}

/// A pact or proposal record, its roles resolved: the countries and the raw ids they came from.
struct Record<'s> {
    fields: &'s [Scalar],
    roles: Vec<Option<DiplomacyCountry>>,
    raw: Vec<u32>,
}

/// Every record from `base` on, in the order they were first seen, with `width` roles at
/// `roles`; those `tags` select, refusing a selection their unresolved roles leave unclear.
fn records<'s>(
    read: &Read<'_>,
    directory: &'s Directory,
    base: u64,
    (at, width, length): (usize, usize, usize),
    tags: Option<&[String]>,
) -> io::Result<Vec<(u64, Record<'s>)>> {
    let tags = tags.map(|tags| tags.iter().map(String::as_str).collect::<BTreeSet<_>>());
    let mut selected = Vec::new();
    let mut ambiguous = 0;
    let mut ordinal = 0;
    while base + ordinal < ACTORS
        && let Some(scalar) = directory.scalar(base + ordinal)
    {
        let fields = scalar.list();
        if fields.len() != length {
            return Err(corrupt());
        }
        let raw = fields[at..at + width]
            .iter()
            .map(integer)
            .collect::<io::Result<Vec<u32>>>()?;
        let roles = fields[at + width..at + 2 * width]
            .iter()
            .map(|cell| read.role(cell))
            .collect::<io::Result<Vec<_>>>()?;
        if tags.as_ref().is_none_or(|tags| {
            roles
                .iter()
                .flatten()
                .any(|role| tags.contains(role.tag.as_str()))
        }) {
            selected.push((ordinal, Record { fields, roles, raw }));
        } else if tags.as_ref().is_some_and(|tags| !tags.is_empty())
            && roles
                .iter()
                .zip(&raw)
                .any(|(role, raw)| role.is_none() && *raw != 0)
        {
            ambiguous += 1;
        }
        ordinal += 1;
    }
    if ambiguous > 0 {
        return Err(io::Error::other(format!(
            "Diplomacy selection cannot exclude unresolved recorded country roles (count: {ambiguous})"
        )));
    }
    Ok(selected)
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
        first_hour: run.first,
        last_hour: run.last,
        read_hour: run.value as i32,
    });
    Ok(())
}

/// The hours of the prefix `key` holds no value at.
fn unavailable<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<KeyHandle>,
    prefix: &Prefix,
) -> io::Result<Vec<HourSpan>> {
    let (first, last) = (prefix.coverage.first_hour, prefix.coverage.last_hour);
    if first > last {
        return Ok(Vec::new());
    }
    let known: Vec<_> = runs(snapshot, key, prefix)?
        .into_iter()
        .map(|run| HourSpan {
            first_hour: run.first,
            last_hour: run.last,
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

/// Each kind's spelling and latest token, by id.
fn kinds(directory: &Directory) -> io::Result<Vec<(String, u32, bool)>> {
    let mut kinds = Vec::new();
    while let Some(scalar) = directory.scalar(KINDS + kinds.len() as u64) {
        let [text, token, unresolved] = scalar.list() else {
            return Err(corrupt());
        };
        let text = std::str::from_utf8(text.text().ok_or_else(corrupt)?).map_err(|_| corrupt())?;
        kinds.push((text.to_owned(), integer(token)?, boolean(unresolved)?));
    }
    Ok(kinds)
}

fn titles(
    source: &Source,
    content: Option<&ContentOptions>,
    tokens: &BTreeMap<&str, u32>,
    pacts: &[DiplomacyPact],
) -> (Vec<DiplomacyKind>, Vec<String>) {
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
        let token = tokens.get(kind.as_str()).copied().unwrap_or(0);
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

/// Each raw relation owner holding a present pact, with its last read.
fn raw_reads(directory: &Directory) -> io::Result<HashMap<u32, i32>> {
    let cells = directory.scalar(RAW_READS).map_or(&[][..], Scalar::list);
    if cells.len() % 2 != 0 {
        return Err(corrupt());
    }
    cells
        .chunks_exact(2)
        .map(|pair| Ok((integer(&pair[0])?, integer(&pair[1])?)))
        .collect()
}

fn pacts<P: PageRead>(
    source: &Source,
    content: Option<&ContentOptions>,
    snapshot: &mut Snapshot<'_, P>,
    read: &Read<'_>,
    tags: Option<&[String]>,
    prefix: &Prefix,
    cancelled: &AtomicBool,
) -> io::Result<DiplomacyPactsReply> {
    let directory = read.directory;
    let kinds = kinds(directory)?;
    let reads = raw_reads(directory)?;
    let entries = records(read, directory, PACTS, (8, 4, 18), tags)?;
    preflight(entries.len() * std::mem::size_of::<DiplomacyPact>())?;
    let mut pacts = Vec::with_capacity(entries.len());
    let mut reply_bytes = 0usize;
    for (ordinal, record) in entries {
        check_cancel(cancelled)?;
        let fields = record.fields;
        let kind: usize = integer(&fields[2])?;
        let (text, token, unresolved) = kinds.get(kind).ok_or_else(corrupt)?;
        if *unresolved {
            return Err(io::Error::other(format!(
                "Recorded diplomacy type {token} has no supplying LexicalTokens spelling (count: 1)"
            )));
        }
        let created: i32 = integer(&fields[0])?;
        let (cancel_raw, retired_raw): (u8, u8) = (integer(&fields[6])?, integer(&fields[7])?);
        let state: u8 = integer(&fields[16])?;
        let last_seen = match state {
            0 => *reads.get(&record.raw[0]).ok_or_else(corrupt)?,
            1 | 2 => integer(&fields[17])?,
            _ => return Err(corrupt()),
        };
        let roles = record.roles;
        let pact = DiplomacyPact {
            id: format!("pact:{created}:{ordinal}"),
            kind: text.clone(),
            owner: tag(&roles[0]),
            target: tag(&roles[1]),
            first: tag(&roles[2]),
            second: tag(&roles[3]),
            identities: roles,
            raw_countries: record.raw,
            ordinal: integer(&fields[3])?,
            start_hour: Some(integer(&fields[4])?),
            end_hour: Some(integer(&fields[5])?),
            cancelled: flag(cancel_raw),
            retired: flag(retired_raw),
            cancel_raw,
            retired_raw,
            first_seen: integer(&fields[1])?,
            last_seen,
            present_at_end: state == 0,
            ended_by: state == 2,
            read_hour: last_seen,
        };
        reply_bytes = reply_bytes.saturating_add(serde_json::to_vec(&pact)?.len());
        preflight(reply_bytes)?;
        pacts.push(pact);
    }
    let tokens: BTreeMap<&str, u32> = kinds
        .iter()
        .map(|(text, token, _)| (text.as_str(), *token))
        .collect();
    let (kinds, mut warnings) = titles(source, content, &tokens, &pacts);
    let unknown = pacts
        .iter()
        .filter(|pact| {
            pact.identities.iter().any(Option::is_none)
                || pact.cancelled.is_none()
                || pact.retired.is_none()
        })
        .count();
    if unknown > 0 {
        warnings.push(format!(
            "Pacts with unresolved endpoints or non-Boolean raw flags (count: {unknown})"
        ));
    }
    let owners: BTreeSet<_> = pacts
        .iter()
        .filter_map(|pact| pact.identities.first()?.as_ref())
        .collect();
    let mut tuple_reads = Vec::new();
    for (country, actor) in &read.countries {
        if tags.is_none_or(|tags| tags.contains(&country.tag)) || owners.contains(country) {
            for run in runs(snapshot, read.key(*actor, TUPLES), prefix)? {
                check_cancel(cancelled)?;
                append_read(&mut tuple_reads, Some(country), &run, &mut reply_bytes)?;
            }
        }
    }
    let unobserved = unavailable(snapshot, read.family(TUPLE_COVERAGE), prefix)?;
    Ok(DiplomacyPactsReply {
        kinds,
        pacts,
        reads: tuple_reads,
        unobserved,
        coverage: prefix.coverage.clone(),
        pending: prefix.pending.clone(),
        warnings,
    })
}

fn proposals<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    read: &Read<'_>,
    tags: Option<&[String]>,
    prefix: &Prefix,
    cancelled: &AtomicBool,
) -> io::Result<DiplomacyProposalsReply> {
    let refused = runs(snapshot, read.family(QUEUE_DUPLICATES), prefix)?;
    if !refused.is_empty() {
        let count: i128 = refused.iter().map(|run| run.value).sum();
        return Err(io::Error::other(format!(
            "Ambiguous incoming diplomatic queue handles (count: {count})"
        )));
    }
    let directory = read.directory;
    let entries = records(read, directory, PROPOSALS, (2, 5, 20), tags)?;
    preflight(entries.len() * std::mem::size_of::<DiplomacyProposal>())?;
    let last = snapshot.publication().accepted.last;
    let queue_read = at(snapshot, read.family(QUEUE_COVERAGE), last)?;
    let mut proposals = Vec::with_capacity(entries.len());
    let mut reply_bytes = 0usize;
    for (ordinal, record) in entries {
        check_cancel(cancelled)?;
        let fields = record.fields;
        let created: i32 = integer(&fields[0])?;
        let present = boolean(&fields[18])?;
        // A present proposal was listed by the queue's last fresh read.
        let last_seen = if present {
            queue_read.ok_or_else(corrupt)?.value as i32
        } else {
            integer(&fields[15])?
        };
        let handle: u64 = integer(&fields[1])?;
        let roles = record.roles;
        let proposal = DiplomacyProposal {
            id: format!("proposal:{created}:{ordinal}"),
            handle: handle.to_string(),
            owner: tag(&roles[0]),
            sender: tag(&roles[1]),
            recipient: tag(&roles[2]),
            original_sender: tag(&roles[3]),
            original_recipient: tag(&roles[4]),
            identities: roles,
            raw_countries: record.raw,
            stored_hour: integer(&fields[12])?,
            last_command_hour: integer(&fields[13])?,
            first_seen: integer(&fields[14])?,
            last_seen,
            gone_by: match optional::<i32>(&fields[16])? {
                Some(hour) => Some(hour),
                // Left awaiting a read without its handle: gone by the read its group is.
                None => optional::<u64>(&fields[19])?
                    .and_then(|group| directory.scalar(GONE + group))
                    .map(integer)
                    .transpose()?,
            },
            gone_by_bound: boolean(&fields[17])?,
            present_at_end: present,
            read_hour: last_seen,
        };
        reply_bytes = reply_bytes.saturating_add(serde_json::to_vec(&proposal)?.len());
        preflight(reply_bytes)?;
        proposals.push(proposal);
    }
    let count = proposals.len();
    let mut warnings = Vec::new();
    if count > 0 {
        warnings.push(format!(
            "Diplomatic proposal kind and outcome are not recorded (count: {count})"
        ));
    }
    let unknown = proposals
        .iter()
        .filter(|proposal| proposal.identities.iter().any(Option::is_none))
        .count();
    if unknown > 0 {
        warnings.push(format!(
            "Proposals with unresolved recorded country roles (count: {unknown})"
        ));
    }
    let coverage = runs(snapshot, read.family(QUEUE_COVERAGE), prefix)?;
    let mut reads = Vec::new();
    // Whole-table reads also prove empty and unresolved owner queues.
    for run in &coverage {
        check_cancel(cancelled)?;
        append_read(&mut reads, None, run, &mut reply_bytes)?;
    }
    let owners: BTreeSet<_> = proposals
        .iter()
        .filter_map(|proposal| proposal.identities.first()?.as_ref())
        .collect();
    // A whole-queue read belongs only to the identity present at its supplying hour.
    for owner in owners {
        check_cancel(cancelled)?;
        let actor = read
            .countries
            .binary_search_by(|(country, _)| country.cmp(owner))
            .map(|at| read.countries[at].1)
            .map_err(|_| corrupt())?;
        let identities = runs(snapshot, read.key(actor, IDENTITY), prefix)?;
        for run in &coverage {
            check_cancel(cancelled)?;
            let supplying = run.value as i32;
            let at = identities.partition_point(|identity| identity.last < supplying);
            let Some(identity) = identities
                .get(at)
                .filter(|identity| identity.first <= supplying)
            else {
                continue;
            };
            let proven = Run {
                first: run.first.max(identity.first),
                last: run.last.min(identity.last),
                value: run.value,
            };
            if proven.first <= proven.last {
                append_read(&mut reads, Some(owner), &proven, &mut reply_bytes)?;
            }
        }
    }
    let unobserved = unavailable(snapshot, read.family(QUEUE_COVERAGE), prefix)?;
    Ok(DiplomacyProposalsReply {
        proposals,
        reads,
        unobserved,
        coverage: prefix.coverage.clone(),
        pending: prefix.pending.clone(),
        warnings,
    })
}
