
use super::*;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Whole {
    pub family: MilitaryAirFamily,
    pub first_hour: i32,
    pub last_hour: i32,
    pub available: bool,
    pub observed: Vec<HourSpan>,
    pub incomplete: Vec<HourSpan>,
    pub countries: Vec<MilitaryAirCountry>,
    pub parts: Vec<MilitaryAirPart>,
    pub regions: Vec<MilitaryAirName>,
    pub buildings: Vec<MilitaryAirName>,
    pub warnings: Vec<String>,
}

pub(super) fn whole(source: &Source, query: &Query<'_>, params: &MilitaryAirParams, first: i32, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Whole> {
    let fold = scan(source, query, params.family, (first, query.timeline.last), cancelled)?;
    let countries = countries(query, &fold.observed);
    let mut warnings = Vec::new();
    let (regions, buildings) = names(source, context, &fold, &mut warnings);
    let mut reply = whole_reply(&fold, (first, query.timeline.last), &countries, params, warnings, cancelled)?;
    reply.regions = regions;
    reply.buildings = buildings;
    Ok(reply)
}

pub(super) fn whole_reply(fold: &Fold, (first, last): (i32, i32), countries: &BTreeMap<Named, Vec<HourSpan>>, params: &MilitaryAirParams, warnings: Vec<String>, cancelled: &AtomicBool) -> io::Result<Whole> {
    let leaves = selected(fold, params)?;
    let from = params.from_hour.unwrap_or_else(|| fold.observed.first().map_or(first, |span| span.first_hour));
    let membership = params.top.as_ref().map(|top| ranked(countries, leaf_deltas(&leaves, from, top)?, from, top, cancelled)).transpose()?;
    let mut all = fold.warnings.iter().map(|(reason, count)| format!("{count} events: {reason}")).collect::<Vec<_>>();
    all.extend(warnings);
    Ok(Whole {
        family: params.family,
        first_hour: first,
        last_hour: last,
        available: !fold.observed.is_empty(),
        observed: fold.observed.clone(),
        incomplete: fold.incomplete.clone(),
        countries: ranked_countries(countries, membership.as_ref(), cancelled)?,
        parts: parts(&leaves, params.family)?,
        regions: Vec::new(),
        buildings: Vec::new(),
        warnings: all,
    })
}

fn push_joined(spans: &mut Vec<HourSpan>, span: &HourSpan) {
    match spans.last_mut() {
        Some(last) if span.first_hour <= last.last_hour.saturating_add(1) => {
            last.first_hour = last.first_hour.min(span.first_hour);
            last.last_hour = last.last_hour.max(span.last_hour);
        }
        _ => spans.push(span.clone()),
    }
}

pub(super) fn join(windows: &[MilitaryAirReply]) -> io::Result<Whole> {
    let (Some(head), Some(tail)) = (windows.first(), windows.last()) else { return Err(dashboard_core::invalid("no window to join")) };
    if !head.openings.is_empty() {
        return Err(dashboard_core::invalid("the first window joined carries totals from before it"));
    }
    let (mut observed, mut incomplete) = (Vec::new(), Vec::new());
    let mut present: BTreeMap<Named, (Vec<HourSpan>, Option<Vec<HourSpan>>)> = BTreeMap::new();
    let mut parts: BTreeMap<MilitaryAirPart, (Vec<(i32, i32, String)>, i128)> = BTreeMap::new();
    let (mut regions, mut buildings) = (BTreeMap::new(), BTreeMap::new());
    let mut notes: BTreeMap<String, usize> = BTreeMap::new();
    let mut warnings: Vec<String> = Vec::new();
    for window in windows {
        window.observed.iter().for_each(|span| push_joined(&mut observed, span));
        window.incomplete.iter().for_each(|span| push_joined(&mut incomplete, span));
        for country in &window.countries {
            let (spans, membership) = present.entry((country.tag.clone(), country.identity.clone())).or_default();
            country.present.iter().for_each(|span| push_joined(spans, span));
            if let Some(ranked) = &country.membership {
                let held = membership.get_or_insert_with(Vec::new);
                ranked.iter().for_each(|span| push_joined(held, span));
            }
        }
        for part in &window.parts {
            let (points, carried) = parts.entry(MilitaryAirPart { points: Vec::new(), ..part.clone() }).or_default();
            let base = *carried;
            for (hour, read_hour, total) in &part.points {
                let total = base + total.parse::<i128>().map_err(|_| dashboard_core::invalid("military air total is not a number"))?;
                points.push((*hour, *read_hour, total.to_string()));
                *carried = total;
            }
        }
        for name in &window.regions {
            regions.entry(name.id.parse::<u32>().map_err(|_| dashboard_core::invalid("region id is not a number"))?).or_insert_with(|| name.clone());
        }
        for name in &window.buildings {
            buildings.entry(name.id.clone()).or_insert_with(|| name.clone());
        }
        for note in &window.notes {
            *notes.entry(note.reason.clone()).or_default() += note.events;
        }
        for warning in &window.warnings {
            if !warnings.contains(warning) {
                warnings.push(warning.clone());
            }
        }
    }
    let ranking = head.countries.iter().any(|country| country.membership.is_some());
    let countries = present
        .into_iter()
        .map(|((tag, identity), (present, membership))| MilitaryAirCountry { tag, identity, present, membership: membership.or_else(|| ranking.then(Vec::new)) })
        .collect();
    let mut all: Vec<String> = notes.into_iter().map(|(reason, events)| format!("{events} events: {reason}")).collect();
    all.extend(warnings);
    Ok(Whole {
        family: head.family,
        first_hour: head.first_hour,
        last_hour: tail.last_hour,
        available: !observed.is_empty(),
        countries,
        observed,
        incomplete,
        parts: parts.into_iter().map(|(part, (points, _))| MilitaryAirPart { points, ..part }).collect(),
        regions: regions.into_values().collect(),
        buildings: buildings.into_values().collect(),
        warnings: all,
    })
}

pub(super) fn carried(windows: &[MilitaryAirReply]) -> io::Result<Openings> {
    let mut out = Openings::new();
    for window in windows {
        for part in &window.parts {
            let Some((_, _, total)) = part.points.last() else { continue };
            let cause = part.cause.ok_or_else(|| dashboard_core::invalid("a part without a cause"))?;
            let total: i128 = total.parse().map_err(|_| dashboard_core::invalid("military air total is not a number"))?;
            *out.entry((part.identity.clone(), part.dealt, cause, part.opponent_known)).or_default() += total;
        }
    }
    out.retain(|_, total| *total != 0);
    Ok(out)
}
