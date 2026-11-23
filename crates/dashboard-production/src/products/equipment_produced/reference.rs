
use super::{value::TypeCells, *};
use replay_data::rows::tables;

fn type_key(id: u32, token: u32) -> u128 {
    u128::from(id) << 32 | u128::from(token)
}

struct Select {
    choice: Choice,
    measure: ProducedMeasure,
    types: Option<BTreeSet<String>>,
}

struct Asked {
    tokens: Vec<u32>,
    untyped: bool,
}

impl Select {
    fn asked(&self, types: Option<&BTreeMap<u32, Typed>>) -> Option<Asked> {
        let keys = self.types.as_ref()?;
        let tokens = types.into_iter().flatten().filter(|(_, typed)| keys.contains(&typed.kind.key)).map(|(&token, _)| token).collect();
        Some(Asked { tokens, untyped: !keys.contains(UNRECORDED_TYPE) })
    }

    fn counts(&self, types: Option<&BTreeMap<u32, Typed>>, token: u32, spelled: Option<&Typed>) -> Counted {
        let Some(keys) = &self.types else { return Counted::Yes };
        if token == ProductionTypeTotalRow::NO_ARCHETYPE {
            return if keys.contains(UNRECORDED_TYPE) { Counted::Yes } else { Counted::NotKnown };
        }
        let typed = types.and_then(|types| types.get(&token)).or(spelled);
        match typed {
            Some(typed) if keys.contains(&typed.kind.key) => Counted::Yes,
            Some(_) => Counted::No,
            None => Counted::NotKnown,
        }
    }
}

fn rocket_archetypes(query: &Query<'_>, run: u32, hour: i32, variants: &[u128]) -> io::Result<HashMap<u64, u32>> {
    if variants.is_empty() {
        return Ok(HashMap::new());
    }
    let frame = cost::storage(|| replay_lanes::at::frame(query.replay.recording(), run, hour, &[replay_lanes::at::Read { table: TableId::VariantDefinitions, columns: None, keys: Keys::These(variants) }]))?;
    Ok(frame.complete::<tables::VariantDefinitions>().unwrap_or(&[]).iter().map(|row| (row.id, row.archetype_token)).collect())
}

fn rocket_typed(query: &Query<'_>, span: &Span, tokens: &HashMap<u64, u32>, content: &Content, notes: &mut BTreeSet<String>) -> io::Result<HashMap<u32, Typed>> {
    let unique: BTreeSet<u128> = tokens.values().filter(|&&token| token != 0).map(|&token| u128::from(token)).collect();
    let asked: Vec<u128> = unique.into_iter().collect();
    let spelled = spellings(query, span.run, span.last, &asked)?;
    Ok(asked.into_iter().map(|token| (token as u32, typed(content, spelled.get(&(token as u32)).map(String::as_str), notes))).collect())
}

fn type_keys_of(id: u32, asked: &Asked) -> Vec<u128> {
    let mut keys: Vec<u128> = asked.tokens.iter().map(|&token| type_key(id, token)).collect();
    if asked.untyped {
        keys.push(type_key(id, ProductionTypeTotalRow::NO_ARCHETYPE));
    }
    keys
}

struct RunRows {
    country: Read,
    coverage: Read,
    types: Option<Read>,
    world: Read,
    rocket: Read,
}

impl RunRows {
    fn read(query: &Query<'_>, group: &RunGroup, asked: Option<&Asked>) -> io::Result<Self> {
        let span = Span { run: group.run, first: group.first, last: group.last, id: 0, breaks: false };
        let countries: Vec<u128> = group.ids.iter().map(|&id| u128::from(id)).collect();
        let mut coverage: Vec<u128> = group.ids.iter().map(|&id| coverage_keys(id)[0]).collect();
        coverage.extend(&coverage_keys(0)[1..]);
        coverage.sort_unstable();
        coverage.dedup();
        let mut types: Vec<u128> = asked.map(|asked| group.ids.iter().flat_map(|&id| type_keys_of(id, asked)).collect()).unwrap_or_default();
        types.sort_unstable();
        let rockets: Vec<(u128, u128)> = group.ids.iter().map(|&id| rocket_designs(id)).collect();
        let ((country, coverage), ((types, world), rocket)) = rayon::join(
            || rayon::join(
                || Read::of(query, &span, TableId::ProductionCountryTotals, &COUNTRY_COLUMNS, Keys::These(&countries)),
                || Read::of(query, &span, TableId::ProductionCoverage, &COVERAGE_COLUMNS, Keys::These(&coverage)),
            ),
            || rayon::join(
                || rayon::join(
                    || asked.map(|_| Read::of(query, &span, TableId::ProductionTypeTotals, &COUNTRY_COLUMNS, Keys::These(&types))).transpose(),
                    || Read::of(query, &span, TableId::RocketProductionOutput, &["spent_raw"], Keys::These(&[0])),
                ),
                || Read::of(query, &span, TableId::RocketProductionOutput, &ROCKET_COLUMNS, Keys::Ranges(&rockets)),
            ),
        );
        Ok(Self { country: country?, coverage: coverage?, types: types?, world: world?, rocket: rocket? })
    }

    fn rocket_variants(&self, id: u32) -> Vec<u64> {
        self.rocket.history.keys.iter().filter(|(key, _)| (key >> 64) as u32 == id).map(|(key, _)| *key as u64).collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RunGroup {
    run: u32,
    first: i32,
    last: i32,
    ids: BTreeSet<u32>,
    spans: Vec<(usize, usize)>,
}

fn run_groups(spans: &[Vec<Span>]) -> Vec<RunGroup> {
    let mut groups: BTreeMap<u32, RunGroup> = BTreeMap::new();
    for (line, held) in spans.iter().enumerate() {
        for (at, span) in held.iter().enumerate() {
            let group = groups.entry(span.run).or_insert_with(|| RunGroup { run: span.run, first: span.first, last: span.last, ids: BTreeSet::new(), spans: Vec::new() });
            group.first = group.first.min(span.first);
            group.last = group.last.max(span.last);
            group.ids.insert(span.id);
            group.spans.push((line, at));
        }
    }
    groups.into_values().collect()
}


struct SpanPlan {
    type_keys: Vec<u128>,
    variants: Vec<u64>,
    counted: Vec<Counted>,
}

fn span_plan(query: &Query<'_>, rows: &RunRows, span: &Span, select: &Select, types: Option<&BTreeMap<u32, Typed>>, content: &Content, notes: &mut BTreeSet<String>) -> io::Result<SpanPlan> {
    let asked = select.asked(types);
    let type_keys = asked.as_ref().map(|asked| type_keys_of(span.id, asked)).unwrap_or_default();
    let variants = rows.rocket_variants(span.id);
    let keys: Vec<u128> = variants.iter().map(|&variant| u128::from(variant)).collect();
    let archetypes = if select.types.is_some() { rocket_archetypes(query, span.run, span.last, &keys)? } else { HashMap::new() };
    let extra = if select.types.is_some() { rocket_typed(query, span, &archetypes, content, notes)? } else { HashMap::new() };
    let counted: Vec<Counted> = variants
        .iter()
        .map(|variant| {
            let token = archetypes.get(variant).copied().unwrap_or(0);
            select.counts(types, token, extra.get(&token))
        })
        .collect();
    Ok(SpanPlan { type_keys, variants, counted })
}

fn span_values(rows: &RunRows, span: &Span, select: &Select, asked: Option<&Asked>, plan: &SpanPlan) -> Vec<Piece<measure::Counted>> {
    let mut tracks = vec![rows.country.steps(u128::from(span.id))];
    tracks.extend(coverage_keys(span.id).map(|key| rows.coverage.steps(key)));
    tracks.push(rows.world.steps(0));
    let typed_at = tracks.len();
    if let Some(read) = &rows.types {
        tracks.extend(plan.type_keys.iter().map(|&key| read.steps(key)));
    }
    let rocket_at = tracks.len();
    tracks.extend(plan.variants.iter().map(|&variant| rows.rocket.steps(u128::from(span.id) << 64 | u128::from(variant))));
    let asked_count = asked.map_or(0, |asked| asked.tokens.len());

    let mut steps: Vec<(i32, measure::Counted)> = Vec::new();
    let mut held_types: Vec<Option<Totals>> = Vec::with_capacity(asked_count);
    let mut held_rocket: Vec<(Counted, Option<RocketRow>)> = Vec::with_capacity(plan.variants.len());
    sweep(&tracks, span.first, span.last, |hour, cells| {
        let country = cells[0].map(|at| totals(number(&rows.country, Some(at))));
        let [own, lines, rocket_run] = [1, 2, 3].map(|at| cells[at].map(|held| coverage(number(&rows.coverage, Some(held)))));
        held_types.clear();
        let mut untyped = None;
        if let (Some(asked), Some(read)) = (asked, &rows.types) {
            for at in 0..asked.tokens.len() {
                held_types.push(cells[typed_at + at].map(|held| totals(number(read, Some(held)))));
            }
            if asked.untyped {
                untyped = Some(cells[typed_at + asked.tokens.len()].map(|held| totals(number(read, Some(held)))));
            }
        }
        held_rocket.clear();
        for (at, counted) in plan.counted.iter().enumerate() {
            held_rocket.push((*counted, cells[rocket_at + at].map(|held| rocket_row(number(&rows.rocket, Some(held))))));
        }
        let held = value::Cells {
            country,
            lines,
            rocket_run,
            rocket_own: own,
            types: asked.map(|_| TypeCells { asked: &held_types, untyped }),
            rocket_world: cells[4].is_some(),
            rocket: &held_rocket,
        };
        let amount = value::value(&held, hour, select.choice);
        steps.push((hour, (amount, (lines.and_then(|row| row.observation), rocket_run.and_then(|row| row.observation)))));
    });
    let absent = (Amount::Unknown(ProducedUnknown::NotRecorded), (None, None));
    pieces(steps, span.first, span.last, absent)
}

fn lines(query: &Query<'_>, tags: &[Country], select: &Select, types: &BTreeMap<u32, BTreeMap<u32, Typed>>, content: &Content, notes: &mut BTreeSet<String>, dated: &mut Dated, cancelled: &AtomicBool) -> io::Result<Vec<ProducedSeries>> {
    use rayon::prelude::*;
    let spans: Vec<Vec<Span>> = tags.iter().map(|tag| spending::spans(query, tag)).collect();
    let mut totals: Vec<Vec<Vec<Piece<measure::Counted>>>> = spans.iter().map(|held| vec![Vec::new(); held.len()]).collect();
    for group in run_groups(&spans) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let run_types = types.get(&group.run);
        let asked = select.asked(run_types);
        let rows = RunRows::read(query, &group, asked.as_ref())?;
        let plans = group.spans.iter().map(|&(line, at)| span_plan(query, &rows, &spans[line][at], select, run_types, content, notes)).collect::<io::Result<Vec<_>>>()?;
        let swept: Vec<Vec<Piece<measure::Counted>>> = group.spans.par_iter().zip(&plans).map(|(&(line, at), plan)| span_values(&rows, &spans[line][at], select, asked.as_ref(), plan)).collect();
        for (&(line, at), values) in group.spans.iter().zip(swept) {
            totals[line][at] = values;
        }
    }
    let mut out = Vec::with_capacity(tags.len());
    for (held, values) in spans.iter().zip(totals) {
        let mut series = ProducedSeries::default();
        for (span, totals) in held.iter().zip(values) {
            let reads = dated.reads(query, span)?;
            dated.observed(&totals);
            series.append(&measure::measured(&totals, &reads, select.measure), span.breaks);
        }
        out.push(series);
    }
    Ok(out)
}

fn values_at(query: &Query<'_>, run: u32, hour: i32, ids: &[u32], select: &Select, types: Option<&BTreeMap<u32, Typed>>, content: &Content, notes: &mut BTreeSet<String>) -> io::Result<HashMap<u32, measure::Counted>> {
    let asked = select.asked(types);
    let country_keys: Vec<u128> = ids.iter().map(|&id| u128::from(id)).collect();
    let country = snapshot(query, run, TableId::ProductionCountryTotals, hour, &COUNTRY_COLUMNS, Keys::These(&country_keys))?;
    let mut cover: Vec<u128> = ids.iter().map(|&id| coverage_keys(id)[0]).collect();
    cover.extend(&coverage_keys(0)[1..]);
    let coverage_rows = snapshot(query, run, TableId::ProductionCoverage, hour, &COVERAGE_COLUMNS, Keys::These(&cover))?.unwrap_or_default();
    let typed_rows = match &asked {
        None => None,
        Some(asked) => {
            let mut keys: Vec<u128> = Vec::new();
            for &id in ids {
                keys.extend(asked.tokens.iter().map(|&token| type_key(id, token)));
                if asked.untyped {
                    keys.push(type_key(id, ProductionTypeTotalRow::NO_ARCHETYPE));
                }
            }
            Some(snapshot(query, run, TableId::ProductionTypeTotals, hour, &COUNTRY_COLUMNS, Keys::These(&keys))?.unwrap_or_default())
        }
    };
    let rocket_rows = snapshot(query, run, TableId::RocketProductionOutput, hour, &ROCKET_COLUMNS, Keys::All)?;
    let world = rocket_rows.as_ref().is_some_and(|rows| rows.contains_key(&0));
    let rocket_rows = rocket_rows.unwrap_or_default();
    let designs: Vec<u128> = rocket_rows.keys().filter(|&&key| key as u64 != 0 && key >> 64 != 0).map(|&key| u128::from(key as u64)).collect();
    let archetypes = if select.types.is_some() { rocket_archetypes(query, run, hour, &designs)? } else { HashMap::new() };
    let span = Span { run, first: hour, last: hour, id: 0, breaks: false };
    let extra = if select.types.is_some() { rocket_typed(query, &span, &archetypes, content, notes)? } else { HashMap::new() };
    let mut rocket_of: HashMap<u32, Vec<(Counted, Option<RocketRow>)>> = HashMap::new();
    for (&key, cells) in &rocket_rows {
        let (owner, variant) = ((key >> 64) as u32, key as u64);
        if owner == 0 || variant == 0 {
            continue;
        }
        let token = archetypes.get(&variant).copied().unwrap_or(0);
        rocket_of.entry(owner).or_default().push((select.counts(types, token, extra.get(&token)), Some(rocket_row(|at| cells[at]))));
    }
    let cover_of = |key: u128| coverage_rows.get(&key).map(|cells| coverage(|at| cells[at]));
    let [_, lines_key, rocket_key] = coverage_keys(0);
    let (lines, rocket_run) = (cover_of(lines_key), cover_of(rocket_key));
    let mut out = HashMap::with_capacity(ids.len());
    for &id in ids {
        let held = |key: u128| typed_rows.as_ref().and_then(|rows| rows.get(&key)).map(|cells| totals(|at| cells[at]));
        let asked_types: Vec<Option<Totals>> = asked.as_ref().map_or_else(Vec::new, |asked| asked.tokens.iter().map(|&token| held(type_key(id, token))).collect());
        let untyped = asked.as_ref().filter(|asked| asked.untyped).map(|_| held(type_key(id, ProductionTypeTotalRow::NO_ARCHETYPE)));
        let rocket = rocket_of.remove(&id).unwrap_or_default();
        let cells = value::Cells {
            country: country.as_ref().and_then(|rows| rows.get(&u128::from(id))).map(|cells| totals(|at| cells[at])),
            lines,
            rocket_run,
            rocket_own: cover_of(coverage_keys(id)[0]),
            types: asked.as_ref().map(|_| TypeCells { asked: &asked_types, untyped }),
            rocket_world: world,
            rocket: &rocket,
        };
        out.insert(id, (value::value(&cells, hour, select.choice), (lines.and_then(|row| row.observation), rocket_run.and_then(|row| row.observation))));
    }
    Ok(out)
}

fn ranked(query: &Query<'_>, hour: i32, n: usize, select: &Select, types: &BTreeMap<u32, BTreeMap<u32, Typed>>, content: &Content, notes: &mut BTreeSet<String>) -> io::Result<Vec<String>> {
    let Some(stretch) = query.timeline.at(hour).copied() else {
        return Ok(Vec::new());
    };
    let mut held: BTreeMap<u32, String> = BTreeMap::new();
    for (tag, runs) in &query.holders.tags {
        if let Some(run) = runs.iter().find(|run| run.first <= hour && hour <= run.last)
            && let Holder::Found { id, .. } = run.holder
        {
            held.entry(id).or_insert_with(|| tag.clone());
        }
    }
    let ids: Vec<u32> = held.keys().copied().collect();
    let run_types = types.get(&stretch.run);
    let reads = reads_in(query, stretch.run, stretch.first, hour)?;
    let Some(read) = measure::read_at(&reads, hour) else {
        return Ok(Vec::new());
    };
    let at = values_at(query, stretch.run, read, &ids, select, run_types, content, notes)?;
    let before = match measure::back(select.measure, read) {
        Some(start) if start >= stretch.first && reads.binary_search(&start).is_ok() => Some((start, values_at(query, stretch.run, start, &ids, select, run_types, content, notes)?)),
        _ => None,
    };
    let mut scored: Vec<(i128, &String)> = Vec::new();
    for (id, tag) in &held {
        let Some(&now) = at.get(id) else { continue };
        let amount = match select.measure {
            ProducedMeasure::Total => now.0,
            _ => {
                let same = before.as_ref().filter(|(start, _)| {
                    query.holders.at(tag, *start).is_some_and(|run| run.last >= hour && matches!(run.holder, Holder::Found { id: held, .. } if held == *id))
                });
                measure::difference(now, same.and_then(|(_, values)| values.get(id).copied()))
            }
        };
        if let Some(value) = amount.known().filter(|&value| value > 0) {
            scored.push((value, tag));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    Ok(scored.into_iter().take(n).map(|(_, tag)| tag.clone()).collect())
}

fn select(count: ProducedCount, refits: bool, measure: ProducedMeasure, types: &[String]) -> Select {
    Select { choice: Choice { count, refits }, measure, types: (!types.is_empty()).then(|| types.iter().cloned().collect()) }
}

pub(super) fn whole(source: &Source, query: &Query<'_>, params: &EquipmentProducedParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<EquipmentProducedReply> {
    let mut warnings = Vec::new();
    let content = Content::read(source, context, &mut warnings);
    let mut notes = BTreeSet::new();
    let types = branch_types(source, query, &content, &mut notes)?;
    let select = select(params.count, params.refits, params.measure, &params.types);
    let tags = match (params.top, params.rank_hour) {
        (Some(n), Some(hour)) => ranked(query, hour, n as usize, &select, &types, &content, &mut notes)?.into_iter().map(Country::from).collect(),
        _ => params.countries.clone(),
    };
    let mut dated = Dated::default();
    let series = self::lines(query, &tags, &select, &types, &content, &mut notes, &mut dated, cancelled)?;
    let lines: Vec<ProducedLine> = tags.into_iter().zip(series).map(|(country, series)| ProducedLine { identities: identities(query, &country), tag: query.holders.name(&country, query.timeline.last), country, series }).collect();
    for stretch in &query.timeline.stretches {
        dated.reads(query, &Span { run: stretch.run, first: stretch.first, last: stretch.last, id: 0, breaks: false })?;
    }
    warnings.extend(notes);
    let reply = EquipmentProducedReply {
        branch_id: params.branch_id.clone(),
        count: params.count,
        measure: params.measure,
        decimals: decimals(params.count),
        types: params.types.clone(),
        rank_hour: params.top.and(params.rank_hour),
        window: params.window,
        first_hour: query.timeline.first,
        last_hour: query.timeline.last,
        lines,
        reads: dated.reads.into_iter().collect(),
        observations: dated.observations.into_iter().collect(),
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}
