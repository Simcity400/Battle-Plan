//! Stateless part extraction; only supplying-read character names need history.
use super::*;
use dashboard_base::prepared::query::progressive::Samples;
use replay::frame::DecodedFrame;

const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryStabilityInputs,
    TableId::CountryStabilityLines,
    TableId::CountryCommandPower,
    TableId::ModifierDefinitions,
    TableId::SliceReads,
    TableId::Characters,
    TableId::LexicalTokens,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::CountryCivilWars,
    TableId::CountryWars,
    TableId::WarRelations,
    TableId::CountryCounterInputs,
];

pub(super) fn needs_earlier_names(value: &Value, read: i32, hour: i32) -> bool {
    read != hour
        && value
            .line
            .as_ref()
            .is_some_and(|line| matches!(line.kind.as_str(), "advisor" | "countryLeader"))
}

/// Requested countries expand to the same `(JSON([tag, part]), measure)`
/// operands preparation stores. Preparation supplies its retained read/naming
/// state; direct reads supply an empty scan and never substitute earlier names.
#[allow(clippy::type_complexity)]
pub(super) fn sample(
    scan: &mut Scan,
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
    changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    mut names: impl FnMut(
        &str,
        (u32, u32),
        i32,
        bool,
        usize,
        Option<&[Value]>,
        &NamingCountry,
        &Samples,
    ) -> io::Result<()>,
) -> io::Result<Samples> {
    if requested.is_some_and(|keys| {
        keys.iter()
            .any(|(_, field)| !["stability", "warSupport"].contains(&field.as_str()))
    }) {
        return Err(dashboard_core::invalid(
            "Unknown stability parts sample measure",
        ));
    }
    let selected = requested.map(|keys| {
        keys.iter()
            .map(|(tag, _)| tag.as_str())
            .collect::<BTreeSet<_>>()
    });
    let wanted = requested.map(|keys| {
        keys.iter()
            .map(|(tag, field)| (tag.as_str(), field.as_str()))
            .collect::<BTreeSet<_>>()
    });
    let mut result = Samples::new();
    let mut seen = BTreeSet::new();
    // Use the production step once: preparation retains read/naming state,
    // while direct queries supply an empty scan and never substitute old names.
    scan.step(
        frame,
        selected.as_ref(),
        changes,
        |tag, identity, read, joined, measures, naming| {
            for (index, values) in measures.iter().enumerate() {
                let field = ["stability", "warSupport"][index];
                if wanted
                    .as_ref()
                    .is_some_and(|keys| !keys.contains(&(tag, field)))
                {
                    continue;
                }
                seen.insert((tag.to_owned(), field.to_owned()));
                result.insert(
                    (serde_json::to_string(&(tag, ""))?, field.into()),
                    (Some(identity), values.as_ref().map(|_| i128::from(read))),
                );
                for value in values
                    .iter()
                    .flatten()
                    .filter(|value| !needs_earlier_names(value, read, frame.hour))
                {
                    result.insert(
                        (serde_json::to_string(&(tag, &value.id))?, field.into()),
                        (Some(identity), Some(i128::from(value.raw))),
                    );
                }
                names(
                    tag,
                    identity,
                    read,
                    joined,
                    index,
                    values.as_deref(),
                    naming,
                    &result,
                )?;
            }
            Ok(())
        },
    )?;
    if changes.is_none()
        && let Ok(countries) = frame.complete::<tables::Countries>()
    {
        for country in countries {
            let Ok(tag) = std::str::from_utf8(&country.tag) else {
                continue;
            };
            let identity = frame
                .complete::<tables::CountryLinks>()
                .ok()
                .and_then(|links| mapped_slot(country, links).ok())
                .map(|slot| (slot, country.id));
            for field in ["stability", "warSupport"] {
                if wanted
                    .as_ref()
                    .is_some_and(|keys| !keys.contains(&(tag, field)))
                {
                    continue;
                }
                let key = (tag.to_owned(), field.to_owned());
                if seen.insert(key) {
                    result.insert(
                        (serde_json::to_string(&(tag, ""))?, field.into()),
                        (identity, None),
                    );
                }
            }
        }
    }
    if let Some(keys) = requested {
        for key in keys {
            if !seen.contains(key) {
                result.insert(key.clone(), (None, None));
            }
        }
    }
    Ok(result)
}

/// The closure retains only latest naming operands from frames already sampled
/// by the shared reader. It never scans again or writes transient data.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn read(
    source: &Source,
    coverage: Option<&dashboard_base::prepared::coverage::Coverage>,
    product: &str,
    params: &serde_json::Value,
    countries: &mut BTreeMap<String, Scanned>,
    cancelled: &AtomicBool,
) -> io::Result<(
    BTreeMap<(String, String), Vec<dashboard_base::prepared::series::Run>>,
    Vec<HourSpan>,
    BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>,
    BTreeSet<(String, i32)>,
)> {
    use dashboard_base::prepared::{query::progressive, series};
    let branch = params["branchId"]
        .as_str()
        .and_then(|branch| branch.parse().ok())
        .ok_or_else(|| dashboard_core::invalid("Invalid parts branch"))?;
    let mut pending = BTreeSet::new();
    let mut sampled = (BTreeMap::new(), Vec::new(), BTreeMap::new());
    if !coverage.is_some_and(|coverage| coverage.complete)
        && matches!(
            product,
            "series-state" | "series-summary" | "series-next-change"
        )
    {
        let requests = params[if product == "series-state" {
            "reads"
        } else {
            "tags"
        }]
        .as_array()
        .ok_or_else(|| dashboard_core::invalid("Missing parts selection"))?;
        let fields = params["measures"]
            .as_array()
            .filter(|fields| !fields.is_empty())
            .map_or_else(|| vec![params["family"]["measure"].clone()], Clone::clone);
        let mut requested = Vec::new();
        let mut hours = Vec::new();
        for request in requests {
            let tag = request
                .as_str()
                .or_else(|| request["tag"].as_str())
                .ok_or_else(|| dashboard_core::invalid("Invalid parts tag"))?;
            dashboard_core::protocol::validate_tag(tag).map_err(dashboard_core::invalid)?;
            for field in &fields {
                requested.push((
                    tag.to_owned(),
                    field
                        .as_str()
                        .ok_or_else(|| dashboard_core::invalid("Invalid parts measure"))?
                        .to_owned(),
                ));
            }
            if product == "series-state" {
                hours.push(
                    request["hour"]
                        .as_i64()
                        .and_then(|hour| i32::try_from(hour).ok())
                        .ok_or_else(|| dashboard_core::invalid("Invalid parts hour"))?,
                );
            }
        }
        requested.sort();
        requested.dedup();
        let transient = std::cell::RefCell::new(BTreeMap::<String, Scanned>::new());
        let unknown = std::cell::RefCell::new(BTreeSet::new());
        let extract = |frame: &DecodedFrame, keys: Option<&[(String, String)]>| {
            sample(
                &mut Scan::default(),
                frame,
                keys,
                None,
                |tag, _, read, _, index, values, naming, _| {
                    dashboard_text::metadata::check_cancel(cancelled)?;
                    let mut transient = transient.borrow_mut();
                    let measure = &mut transient.entry(tag.into()).or_default().measures[index];
                    for value in values.into_iter().flatten() {
                        if needs_earlier_names(value, read, frame.hour) {
                            unknown.borrow_mut().insert((tag.to_owned(), frame.hour));
                        } else {
                            measure
                                .parts
                                .entry(value.id.clone())
                                .or_default()
                                .update(value, naming);
                        }
                    }
                    Ok(())
                },
            )
        };
        sampled = if product == "series-state" {
            progressive::state_tail(
                source, branch, coverage, TABLES, &requested, &hours, &extract, cancelled,
            )?
        } else {
            let window = if product == "series-summary" {
                series::range(params["from"].as_f64(), params["to"].as_f64())?
            } else {
                series::range(params["firstHour"].as_f64(), params["lastHour"].as_f64())?
            };
            progressive::tail(
                source,
                branch,
                coverage,
                TABLES,
                &requested,
                (window.0.floor() as i32, window.1.ceil() as i32),
                &extract,
                cancelled,
            )?
        };
        for (tag, scanned) in transient.into_inner() {
            let current = countries.entry(tag).or_default();
            for (measure, incoming) in current.measures.iter_mut().zip(scanned.measures) {
                measure.parts.extend(incoming.parts);
            }
        }
        pending = unknown.into_inner();
    }
    Ok((sampled.0, sampled.1, sampled.2, pending))
}

#[cfg(test)]
mod tests;
