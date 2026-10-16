//! Score pages over shared series and bounded direct reads of unprepared hours.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use dashboard_base::products::country_groups::HourSpan;
use dashboard_core::protocol::validate_tag;
use serde_json::{Value, json};

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    series: Accumulator,
    keys: BTreeMap<String, u64>,
    metadata: Metadata,
    last: BTreeMap<u64, (i32, Facts)>,
    entities: BTreeMap<String, HourSpan>,
    stretches: BTreeMap<u64, (i32, Option<i32>, i32)>,
    active: std::collections::BTreeSet<u64>,
    frontier: Option<i32>,
    observed: Option<i32>,
    /// The observed hour's holders, so that an hour's changed countries are
    /// observed alone. A restored state reads them at its next whole hour.
    #[serde(skip)]
    holders: Option<Holders>,
    /// Each present warning's count per hour, added to the catalogue's
    /// observations when it changes and at publication.
    #[serde(skip)]
    counting: Option<Counting>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}
#[derive(Default, Serialize, Deserialize)]
struct Metadata {
    hints: BTreeMap<String, String>,
    warnings: Vec<GameScoreWarning>,
    content_warnings: Vec<String>,
    initialized: bool,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Facts {
    identity: Option<GameScoreIdentity>,
    missing: [Option<GameScoreMissing>; 6],
    read_each: [bool; 6],
    stretch: Option<String>,
}
#[derive(Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameScoreStateReply {
    pub reply: GameScoreReply,
    /// Native-holder continuity, including aliases, for the table/race row builder.
    pub stretches: BTreeMap<String, String>,
    /// Direct frame values beyond accepted preparation have no proven race continuity.
    pub approximate: bool,
}

/// Each country's native holder and whether its total is known, at the
/// observed hour.
#[derive(Default)]
struct Holders {
    /// Each country's holder key, when found, and whether its total is known.
    tags: HashMap<String, (Option<String>, bool)>,
    /// Countries by holder key.
    countries: HashMap<String, Vec<String>>,
    /// Countries of each holder whose total is known.
    live: HashMap<String, usize>,
}

impl Holders {
    fn insert(&mut self, tag: &str, holder: Option<String>, known: bool) {
        if let Some(holder) = &holder {
            self.countries
                .entry(holder.clone())
                .or_default()
                .push(tag.to_owned());
            if known {
                *self.live.entry(holder.clone()).or_default() += 1;
            }
        }
        self.tags.insert(tag.to_owned(), (holder, known));
    }

    /// Whether any of the holder's countries has a known total.
    fn is_live(&self, holder: &str) -> bool {
        self.live.get(holder).is_some_and(|count| *count > 0)
    }
}

/// Observed hours, and each present warning's per-hour count with the hour
/// count through which it was added to the catalogue.
#[derive(Default)]
struct Counting {
    hours: u64,
    counts: BTreeMap<(GameScoreMissingKind, String), (u64, u64)>,
}

/// The native holder a country's continuity is keyed by.
fn holder(identity: &GameScoreIdentity) -> io::Result<Option<String>> {
    match (identity.canonical_slot, identity.native_country) {
        (Some(slot), Some(country)) => Ok(Some(serde_json::to_string(&(slot, country))?)),
        _ => Ok(None),
    }
}

fn same_holder(before: Option<&GameScoreIdentity>, identity: &GameScoreIdentity) -> bool {
    before.is_some_and(|before| {
        before.canonical_slot == identity.canonical_slot
            && before.native_country == identity.native_country
    })
}

/// Add observations of one warning, keeping the catalogue in reason order.
fn add_observations(
    warnings: &mut Vec<GameScoreWarning>,
    (kind, reason): &(GameScoreMissingKind, String),
    observations: u64,
) {
    if observations == 0 {
        return;
    }
    match warnings.binary_search_by(|warning| (&warning.kind, &warning.reason).cmp(&(kind, reason)))
    {
        Ok(index) => warnings[index].observations += observations,
        Err(index) => warnings.insert(
            index,
            GameScoreWarning {
                kind: kind.clone(),
                reason: reason.clone(),
                observations,
            },
        ),
    }
}

impl Prepared {
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        observer: &mut ScoreObserver<'_>,
        naming: &Naming,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        if !self.metadata.initialized {
            let mut warnings = BTreeMap::new();
            self.metadata.hints = score_hints(naming.localisation.as_deref().ok(), &mut warnings);
            self.add_warnings(&warnings);
            self.metadata.content_warnings = naming.warnings.clone();
            self.metadata.initialized = true;
        }
        let (facts, change) = observer.observe_changed(
            frame,
            replay,
            &mut || {
                // Prepared recordings use the current format's captured multipliers.
                unreachable!("Prepared score inputs require the current recording format")
            },
            Some(changes),
        )?;
        self.count_warnings(facts, &change);
        let hour = frame.hour;
        let continuous = self
            .frontier
            .is_some_and(|last| last.checked_add(1) == Some(hour));
        self.frontier = Some(hour);
        self.series.advance(hour)?;
        if continuous
            && (matches!(change, Change::Same)
                || !SCORE_TABLES
                    .iter()
                    .any(|table| changes.rows_changed[table.index()]))
        {
            return Ok(());
        }
        match change {
            Change::Countries { tags, .. } if continuous && self.holders.is_some() => {
                self.observe_countries(hour, facts, &tags)
            }
            _ => self.observe_all(hour, facts, continuous),
        }
    }

    /// Every country, as at a gap, a new country set or a whole read.
    fn observe_all(&mut self, hour: i32, facts: &FrameScores, continuous: bool) -> io::Result<()> {
        self.normalize();
        let previous = continuous.then_some(self.observed).flatten();
        let mut holders = Holders::default();
        for (tag, (values, identity)) in &facts.countries {
            #[cfg(test)]
            {
                self.work.row_visits += 1;
            }
            let entity = holder(identity)?;
            let known = values.total.raw.is_some();
            if known && let Some(entity) = &entity {
                let span = self.entities.entry(entity.clone()).or_insert(HourSpan {
                    first_hour: hour,
                    last_hour: hour,
                });
                if span.last_hour != hour {
                    if Some(span.last_hour) != previous {
                        span.first_hour = hour;
                    }
                    span.last_hour = hour;
                }
            }
            holders.insert(tag, entity, known);
        }
        let boundary = self
            .last
            .get(&0)
            .is_none_or(|(last, _)| Some(*last) != previous);
        self.values(0, hour, &facts.multipliers, None, None, boundary)?;
        let mut active = std::collections::BTreeSet::new();
        for (tag, (values, identity)) in &facts.countries {
            #[cfg(test)]
            {
                self.work.row_visits += 1;
            }
            let base = if let Some(&base) = self.keys.get(tag) {
                base
            } else {
                let base = u64::try_from(self.keys.len())
                    .ok()
                    .and_then(|count| count.checked_add(1))
                    .and_then(|count| count.checked_mul(8))
                    .ok_or_else(|| dashboard_core::invalid("Score key overflow"))?;
                self.keys.insert(tag.clone(), base);
                base
            };
            active.insert(base);
            let same = self.last.get(&base).is_some_and(|(last, before)| {
                Some(*last) == previous && same_holder(before.identity.as_ref(), identity)
            });
            let entity = holders
                .tags
                .get(tag)
                .and_then(|(entity, _)| entity.as_deref());
            self.observe_country(base, hour, values, identity, same, entity)?;
        }
        for &base in self.active.difference(&active) {
            for key in base..=base + 6 {
                self.series.observe(key, hour, None, false);
                #[cfg(test)]
                {
                    self.work.emitted += 1;
                }
            }
        }
        self.active = active;
        self.observed = Some(hour);
        self.holders = Some(holders);
        Ok(())
    }

    /// Only the countries whose facts changed, and those sharing a holder
    /// whose total became known or unknown. The countries present, their
    /// holders and the multipliers are the observed hour's.
    fn observe_countries(
        &mut self,
        hour: i32,
        facts: &FrameScores,
        tags: &[String],
    ) -> io::Result<()> {
        let mut holders = self.holders.take().expect("holders of the observed hour");
        let result = self.observe_holders(hour, facts, tags, &mut holders);
        self.holders = Some(holders);
        result?;
        self.observed = Some(hour);
        Ok(())
    }

    fn observe_holders(
        &mut self,
        hour: i32,
        facts: &FrameScores,
        tags: &[String],
        holders: &mut Holders,
    ) -> io::Result<()> {
        let country = |tag: &str| {
            facts
                .countries
                .get(tag)
                .ok_or_else(|| dashboard_core::invalid("Changed score country is not observed"))
        };
        // Whether each holder whose known totals changed was live before.
        let mut before = BTreeMap::<String, bool>::new();
        for tag in tags {
            let known = country(tag)?.0.total.raw.is_some();
            let (entity, was) = holders
                .tags
                .get_mut(tag)
                .ok_or_else(|| dashboard_core::invalid("Changed score country has no holder"))?;
            if *was == known {
                continue;
            }
            *was = known;
            if let Some(entity) = entity {
                let count = holders.live.entry(entity.clone()).or_default();
                before.entry(entity.clone()).or_insert(*count > 0);
                if known {
                    *count += 1;
                } else {
                    *count -= 1;
                }
            }
        }
        let flipped: BTreeSet<String> = before
            .into_iter()
            .filter(|(entity, was)| *was != holders.is_live(entity))
            .map(|(entity, _)| entity)
            .collect();
        let mut dirty: BTreeSet<&str> = tags.iter().map(String::as_str).collect();
        for entity in &flipped {
            dirty.extend(
                holders
                    .countries
                    .get(entity)
                    .into_iter()
                    .flatten()
                    .map(String::as_str),
            );
        }
        // A holder's span runs while any of its countries' totals is known,
        // and ended at the previous observed hour when none is any longer.
        for entity in &flipped {
            if !holders.is_live(entity)
                && let (Some(span), Some(observed)) = (self.entities.get_mut(entity), self.observed)
            {
                span.last_hour = observed;
            }
        }
        for tag in &dirty {
            if let Some((Some(entity), _)) = holders.tags.get(*tag)
                && holders.is_live(entity)
            {
                let span = self.entities.entry(entity.clone()).or_insert(HourSpan {
                    first_hour: hour,
                    last_hour: hour,
                });
                if flipped.contains(entity) {
                    span.first_hour = hour;
                }
                span.last_hour = hour;
            }
        }
        for tag in dirty {
            #[cfg(test)]
            {
                self.work.row_visits += 1;
            }
            let (values, identity) = country(tag)?;
            let base = *self
                .keys
                .get(tag)
                .ok_or_else(|| dashboard_core::invalid("Changed score country has no key"))?;
            let same = self.active.contains(&base)
                && self
                    .last
                    .get(&base)
                    .is_some_and(|(_, before)| same_holder(before.identity.as_ref(), identity));
            let entity = holders
                .tags
                .get(tag)
                .and_then(|(entity, _)| entity.as_deref());
            self.observe_country(base, hour, values, identity, same, entity)?;
        }
        Ok(())
    }

    /// One country's stretches and values. `same` continues its identity from
    /// the previous observed hour.
    fn observe_country(
        &mut self,
        base: u64,
        hour: i32,
        values: &GameScoreValues,
        identity: &GameScoreIdentity,
        same: bool,
        entity: Option<&str>,
    ) -> io::Result<()> {
        let current = self.stretches.entry(base).or_insert((hour, None, hour));
        if !same {
            current.0 = hour;
            current.1 = None;
        }
        if values.total.raw.is_some() {
            current.1.get_or_insert(hour);
        } else {
            current.1 = None;
        }
        current.2 = hour;
        let stretch = match entity {
            Some(key) => serde_json::to_string(&(
                key,
                self.entities
                    .get(key)
                    .filter(|span| span.last_hour == hour)
                    .map(|span| span.first_hour),
            ))?,
            None => serde_json::to_string(&(current.0, current.1))?,
        };
        self.values(
            base,
            hour,
            values,
            Some(identity.clone()),
            Some(stretch),
            !same,
        )
    }

    /// Date every retained hour marker at the observed hour, as observing
    /// every country then would: active countries, the multipliers and live
    /// holders. Between whole observations only changed countries' markers move.
    fn normalize(&mut self) {
        let (Some(observed), Some(holders)) = (self.observed, &self.holders) else {
            return;
        };
        for base in self.active.iter().copied().chain([0]) {
            if let Some((last, _)) = self.last.get_mut(&base) {
                *last = observed;
            }
        }
        for base in &self.active {
            if let Some(stretch) = self.stretches.get_mut(base) {
                stretch.2 = observed;
            }
        }
        for (entity, count) in &holders.live {
            if *count > 0
                && let Some(span) = self.entities.get_mut(entity)
            {
                span.last_hour = observed;
            }
        }
    }

    /// Count this hour's warnings: each counts once per observed hour.
    fn count_warnings(&mut self, facts: &FrameScores, change: &Change) {
        match change {
            Change::Same if self.counting.is_some() => {}
            Change::Countries { warnings, .. } if self.counting.is_some() => {
                let counting = self.counting.as_mut().expect("counted warnings");
                for key in warnings {
                    if let Some((count, since)) = counting.counts.remove(key) {
                        add_observations(
                            &mut self.metadata.warnings,
                            key,
                            count * (counting.hours - since),
                        );
                    }
                    if let Some(&count) = facts.warnings.get(key) {
                        counting.counts.insert(key.clone(), (count, counting.hours));
                    }
                }
            }
            _ => {
                self.add_counted();
                let hours = self.counting.as_ref().map_or(0, |counting| counting.hours);
                self.counting = Some(Counting {
                    hours,
                    counts: facts
                        .warnings
                        .iter()
                        .map(|(key, count)| (key.clone(), (*count, hours)))
                        .collect(),
                });
            }
        }
        if let Some(counting) = &mut self.counting {
            counting.hours += 1;
        }
    }

    /// Add every held warning's hours since it was last added.
    fn add_counted(&mut self) {
        let Some(counting) = &mut self.counting else {
            return;
        };
        for (key, (count, since)) in &mut counting.counts {
            if counting.hours > *since {
                add_observations(
                    &mut self.metadata.warnings,
                    key,
                    *count * (counting.hours - *since),
                );
                *since = counting.hours;
            }
        }
    }

    fn add_warnings(&mut self, warnings: &BTreeMap<(GameScoreMissingKind, String), u64>) {
        for (key, count) in warnings {
            add_observations(&mut self.metadata.warnings, key, *count);
        }
    }
    fn values(
        &mut self,
        base: u64,
        hour: i32,
        values: &GameScoreValues,
        identity: Option<GameScoreIdentity>,
        stretch: Option<String>,
        boundary: bool,
    ) -> io::Result<()> {
        let amounts = values.amounts();
        let facts = Facts {
            identity,
            missing: amounts.map(|amount| amount.missing.clone()),
            read_each: amounts.map(|amount| amount.read_each_hour),
            stretch,
        };
        if boundary
            || self
                .last
                .get(&base)
                .is_none_or(|(_, before)| before != &facts)
        {
            self.series.event(base, hour, &facts)?;
        }
        self.series.observe(base, hour, Some(1), boundary);
        for (index, amount) in amounts.iter().enumerate() {
            self.series.observe(
                base + index as u64 + 1,
                hour,
                amount.raw.map(i128::from),
                boundary,
            );
        }
        self.last.insert(base, (hour, facts));
        #[cfg(test)]
        {
            self.work.history_visits += 1;
            self.work.emitted += 7;
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn work(&self) -> dashboard_base::prepared::observer_tests::Work {
        self.work
    }
    pub fn buffered_bytes(&self) -> usize {
        self.series.bytes()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.add_counted();
        self.normalize();
        self.series
            .flush(writer, &format!("game-score-{branch}"), 5, cancelled)?;
        series::write_catalogue(writer, "game-score", branch, &self.keys, &self.metadata)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Family {
    kind: String,
    measure: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    hour: i32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    family: Family,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    reads: Vec<Read>,
    tag: Option<String>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    cursor: Option<String>,
    page_size: Option<u32>,
    key: Option<series::MeasureKey>,
    first_hour: Option<f64>,
    last_hour: Option<f64>,
}
fn field_index(measure: Option<&str>) -> io::Result<usize> {
    PARTS
        .iter()
        .position(|part| Some(*part) == measure)
        .ok_or_else(|| dashboard_core::invalid("Unknown score measure"))
}

pub fn state(
    source: &Source,
    params: Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let parsed: Params = serde_json::from_value(params.clone())?;
    if parsed.family.kind != "game-score" {
        return Err(dashboard_core::invalid("Unknown score family"));
    }
    let [read] = parsed.reads.as_slice() else {
        return Err(dashboard_core::invalid("Score state requires one hour"));
    };
    for tag in &parsed.tags {
        validate_tag(tag).map_err(dashboard_core::invalid)?;
    }
    let branch = parsed
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid score branch"))?;
    let accepted = match dashboard_base::prepared::coverage::coverage(
        source,
        context.content.as_ref(),
        branch,
        cancelled,
    ) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
        other => other?,
    };
    let mut data = if accepted
        .as_ref()
        .is_some_and(|coverage| coverage.complete || read.hour <= coverage.last_hour)
    {
        let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
        query("series-state", params, &store, &key, cancelled)?
    } else {
        let naming = Naming::load(source, context);
        serde_json::to_value(tail_state(
            source,
            &parsed,
            accepted.as_ref(),
            &naming,
            context,
            cancelled,
        )?)?
    };
    if let Some(reply) = data.as_object_mut() {
        reply.insert("coverage".into(), serde_json::to_value(accepted)?);
    }
    Ok(data)
}

fn tail_state(
    source: &Source,
    params: &Params,
    coverage: Option<&dashboard_base::prepared::coverage::Coverage>,
    naming: &Naming,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<GameScoreStateReply> {
    use std::cell::{OnceCell, RefCell};
    let hour = params.reads[0].hour;
    let replay = source.replay_ref();
    let old_multipliers = OnceCell::new();
    // Retain only the requested frame, never an intervening block or history.
    let target = RefCell::new(None);
    let sample = |frame: &DecodedFrame, _requested: Option<&[(String, String)]>| {
        metadata::check_cancel(cancelled)?;
        metadata::validate_campaign(frame, replay)?;
        let samples = super::sample(frame, replay.format, &mut || {
            old_multipliers
                .get_or_init(|| read_content_multipliers(source, context))
                .clone()
        });
        let values = samples.values();
        if frame.hour == hour {
            *target.borrow_mut() = Some((frame.clone(), samples));
        }
        Ok(values)
    };
    let (values, _, _) = dashboard_base::prepared::query::progressive::state_tail(
        source,
        params
            .branch_id
            .parse()
            .map_err(|_| dashboard_core::invalid("Invalid score branch"))?,
        coverage,
        SCORE_TABLES,
        &[],
        &[hour],
        &sample,
        cancelled,
    )?;
    let mut warnings = BTreeMap::new();
    let hints = score_hints(naming.localisation.as_deref().ok(), &mut warnings);
    let mut reply = GameScoreReply {
        branch_id: params.branch_id.clone(),
        scale: Q,
        reads: Vec::new(),
        countries: Vec::new(),
        multipliers: GameScoreSeries::default(),
        reasons: Vec::new(),
        labels: score_labels(),
        hints,
        warnings: naming.warnings.clone(),
        score_warnings: Vec::new(),
    };
    if let Some((frame, samples)) = target.into_inner() {
        let mut observer = naming.observer(replay);
        let facts = observer.present(&frame, samples)?;
        let mut reasons = ReasonTable::default();
        let mut budget = 0;
        reply.reads.push(hour);
        // The core's retained values own this hour; presentation supplies exact missing reasons.
        let at = |tag: &str,
                  multiplier: bool,
                  scores: &GameScoreValues|
         -> io::Result<GameScoreValues> {
            let mut scores = scores.clone();
            for (field, amount) in PARTS.iter().zip([
                &mut scores.army,
                &mut scores.navy,
                &mut scores.air,
                &mut scores.industry,
                &mut scores.land,
                &mut scores.total,
            ]) {
                let field = if multiplier {
                    format!("multiplier:{field}")
                } else {
                    (*field).into()
                };
                amount.raw = values
                    .get(&(tag.into(), field))
                    .and_then(|runs| {
                        runs.iter()
                            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                    })
                    .map(|run| i64::try_from(run.value))
                    .transpose()
                    .map_err(|_| dashboard_core::invalid("Score value exceeds native width"))?;
            }
            Ok(scores)
        };
        reply.multipliers.push(
            hour,
            &at("", true, &facts.multipliers)?,
            true,
            &mut reasons,
            &mut budget,
        )?;
        for (tag, (scores, identity)) in &facts.countries {
            metadata::check_cancel(cancelled)?;
            if !params.tags.is_empty() && !params.tags.contains(tag) {
                continue;
            }
            let mut series = GameScoreSeries::default();
            series.push(
                hour,
                &at(tag, false, scores)?,
                true,
                &mut reasons,
                &mut budget,
            )?;
            reply.countries.push(GameScoreCountry {
                tag: tag.clone(),
                scores: series,
                identity: vec![GameScoreRun {
                    first_hour: hour,
                    last_hour: hour,
                    value: identity.clone(),
                }],
            });
        }
        for (key, count) in &facts.warnings {
            *warnings.entry(key.clone()).or_default() += count;
        }
        reply.reasons = reasons.entries;
    }
    reply.score_warnings = warnings
        .into_iter()
        .map(|((kind, reason), observations)| GameScoreWarning {
            kind,
            reason,
            observations,
        })
        .collect();
    if dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(oversized());
    }
    Ok(GameScoreStateReply {
        reply,
        stretches: BTreeMap::new(),
        approximate: true,
    })
}
pub fn query(
    product: &str,
    params: Value,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: Params = serde_json::from_value(params)?;
    if params.family.kind != "game-score" {
        return Err(dashboard_core::invalid("Unknown score family"));
    }
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid score branch"))?;
    let Some((keys, metadata)) =
        series::read_catalogue::<Metadata>(store, key, "game-score", branch)?
    else {
        return Ok(Value::Null);
    };
    let selected: Vec<_> = if params.tags.is_empty() {
        keys.iter().collect()
    } else {
        for tag in &params.tags {
            validate_tag(tag).map_err(dashboard_core::invalid)?;
        }
        keys.iter()
            .filter(|(tag, _)| params.tags.contains(tag))
            .collect()
    };
    let mut reader = Reader::new(store, key, format!("game-score-{branch}"), cancelled)?;
    Ok(match product {
        "series-state" => {
            let [read] = params.reads.as_slice() else {
                return Err(dashboard_core::invalid("Score state requires one hour"));
            };
            let hour = read.hour;
            let mut reasons = ReasonTable::default();
            let mut budget = 0;
            let mut countries = Vec::new();
            let mut stretches = BTreeMap::new();
            let recorded = reader.value_at(0, hour)?.is_some();
            let multipliers = if recorded {
                values_at(&mut reader, 0, hour, &mut reasons, &mut budget)?.1
            } else {
                GameScoreSeries::default()
            };
            for (tag, &base) in selected {
                metadata::check_cancel(cancelled)?;
                if reader.value_at(base, hour)?.is_none() {
                    continue;
                }
                let (facts, scores) =
                    values_at(&mut reader, base, hour, &mut reasons, &mut budget)?;
                if let Some(stretch) = facts.stretch {
                    stretches.insert(tag.clone(), stretch);
                }
                let identity = facts
                    .identity
                    .ok_or_else(|| dashboard_core::invalid("Score country identity is missing"))?;
                countries.push(GameScoreCountry {
                    tag: tag.clone(),
                    scores,
                    identity: vec![GameScoreRun {
                        first_hour: hour,
                        last_hour: hour,
                        value: identity,
                    }],
                });
            }
            serde_json::to_value(GameScoreStateReply {
                reply: GameScoreReply {
                    branch_id: params.branch_id,
                    scale: Q,
                    reads: if recorded { vec![hour] } else { Vec::new() },
                    countries,
                    multipliers,
                    reasons: reasons.entries,
                    labels: score_labels(),
                    hints: metadata.hints,
                    warnings: metadata.content_warnings,
                    score_warnings: metadata.warnings,
                },
                stretches,
                approximate: false,
            })?
        }
        "series-summary" => {
            let index = field_index(params.family.measure.as_deref())?;
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            series::summary_budget(from, to, width, selected.len())?;
            let mut summaries = Vec::new();
            for (tag, &base) in selected {
                summaries.push(json!({ "tag": tag, "summary": reader.summary(base + index as u64 + 1, (i32::MIN, i32::MAX), (from, to, width), 5)? }));
            }
            json!({ "branchId": params.branch_id, "decimals": 5, "series": summaries, "warnings": metadata.content_warnings })
        }
        "events-page" => {
            let (from, to, cursor, size) = series::event_window(
                params.from,
                params.to,
                params.cursor.as_deref(),
                params.page_size,
            )?;
            let page = reader.events(
                &selected.iter().map(|(_, key)| **key).collect::<Vec<_>>(),
                from,
                to,
                cursor,
                size,
            )?;
            json!({ "firstOrdinal": page.first.to_string(), "total": page.total.to_string(),
                "cursor": page.cursor.map(|cursor| cursor.to_string()), "rows": page.rows.into_iter().map(|row|
                    json!({"key": row.key.to_string(), "hour": row.hour, "ordinal": row.ordinal.to_string(), "row": row.row})).collect::<Vec<_>>() })
        }
        "events-clusters" => {
            let base = params
                .tag
                .as_ref()
                .and_then(|tag| keys.get(tag))
                .copied()
                .ok_or_else(|| dashboard_core::invalid("Unknown score country"))?;
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            serde_json::to_value(reader.event_clusters(base, from, to, width)?)?
        }
        "series-next-change" => {
            let index = field_index(params.family.measure.as_deref())?;
            let navigation = series::Navigation {
                from: params.from,
                first_hour: params
                    .first_hour
                    .ok_or_else(|| dashboard_core::invalid("Missing first score hour"))?,
                last_hour: params
                    .last_hour
                    .ok_or_else(|| dashboard_core::invalid("Missing last score hour"))?,
                key: params
                    .key
                    .ok_or_else(|| dashboard_core::invalid("Missing score navigation"))?,
            };
            let keys = selected
                .iter()
                .map(|(_, base)| (**base + index as u64 + 1, None))
                .collect();
            json!({ "hour": series::navigate(&mut [(&mut reader, keys)], &navigation)? })
        }
        _ => return Err(dashboard_core::invalid("Unknown prepared score query")),
    })
}
fn values_at(
    reader: &mut Reader<'_>,
    base: u64,
    hour: i32,
    reasons: &mut ReasonTable,
    budget: &mut usize,
) -> io::Result<(Facts, GameScoreSeries)> {
    let (_, row) = reader
        .latest_event(base, hour)?
        .ok_or_else(|| dashboard_core::invalid("Missing score facts"))?;
    let facts: Facts = serde_json::from_value(row)?;
    let mut amounts = Vec::new();
    for index in 0..6 {
        let raw = reader
            .value_at(base + index as u64 + 1, hour)?
            .map(|run| i64::try_from(run.value))
            .transpose()
            .map_err(|_| dashboard_core::invalid("Score value exceeds native width"))?;
        amounts.push(GameScoreAmount {
            raw,
            missing: facts.missing[index].clone(),
            read_each_hour: facts.read_each[index],
        });
    }
    let [army, navy, air, industry, land, total]: [GameScoreAmount; 6] =
        amounts.try_into().expect("six score parts");
    let mut scores = GameScoreSeries::default();
    scores.push(
        hour,
        &GameScoreValues {
            army,
            navy,
            air,
            industry,
            land,
            total,
        },
        true,
        reasons,
        budget,
    )?;
    Ok((facts, scores))
}
