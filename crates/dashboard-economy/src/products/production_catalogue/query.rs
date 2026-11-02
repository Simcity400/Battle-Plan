//! The catalogue's replies, read from the family's series publication ([`super::emit`]): a
//! view's rows by their keys' dimensions, each row's runs within the hours a chosen country is
//! party to it, the holders' tags, the organisations' and variants' naming reads, the texts and
//! the refusals' hours.

use super::{
    emit::{self, ACTOR, kind},
    *,
};
use crate::products::keyed::{clip_text, holder_tags, selected_spans, tag_spans};
use dashboard_base::prepared::engine;
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot,
    codec::Binary,
    family::{Directory, Scalar},
    query::EventView,
};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

/// Bytes one event page answers at most.
const BUDGET: usize = 1 << 20;

fn malformed() -> io::Error {
    dashboard_base::invalid("A production catalogue publication is malformed")
}

/// The reply before the family publishes the branch: `through` the hours it answers.
pub fn unpublished(
    params: &ProductionCatalogueParams,
    through: Option<i32>,
    warnings: Vec<String>,
) -> ProductionCatalogueReply {
    let mut reply = ProductionCatalogueReply::empty(params, through);
    reply.warnings = warnings;
    reply
}

/// `key`'s runs through `through`, as exact decimal text.
fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    through: i32,
) -> io::Result<Vec<EconomyRun>> {
    let Some(key) = key else {
        return Ok(Vec::new());
    };
    let accepted = snapshot.publication().accepted;
    Ok(
        engine::runs(snapshot, key, accepted.first, accepted.last.min(through))?
            .into_iter()
            .map(|run| EconomyRun(run.span.first, run.span.last, run.value.to_string()))
            .collect(),
    )
}

/// Each `[Unsigned id, Text]` event of the family's `measure`, in native order.
fn events<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    measure: u64,
    expected: u64,
    mut visit: impl FnMut(i32, u64, String) -> io::Result<()>,
) -> io::Result<()> {
    let Some(key) = directory.key(ACTOR, measure, None) else {
        return Ok(());
    };
    let selection = [Selection {
        key,
        membership: None,
    }];
    let native = snapshot.publication().accepted;
    let mut cursor = None;
    loop {
        let mut found = Vec::new();
        let page = snapshot.event_page(
            EventQuery {
                selection: &selection,
                native,
                as_of: None,
                order: EventOrder::Native,
            },
            cursor,
            BUDGET,
            |event| {
                found.push(event);
                Ok(())
            },
        )?;
        for event in found {
            let EventView::Family(event) = event else {
                return Err(malformed());
            };
            if event.kind != expected || event.fields.len != 2 * Cell::SIZE as u64 {
                return Err(malformed());
            }
            let id = match snapshot.field(event.fields, 0)? {
                Cell::Unsigned(id) => id,
                _ => return Err(malformed()),
            };
            let text = match snapshot.field(event.fields, 1)? {
                Cell::Text(blob) => {
                    let mut bytes = vec![0; usize::try_from(blob.len).map_err(|_| malformed())?];
                    snapshot.cell_bytes(blob, 0, &mut bytes)?;
                    String::from_utf8(bytes).map_err(|_| malformed())?
                }
                _ => return Err(malformed()),
            };
            visit(event.native_hour, id, text)?;
        }
        cursor = page.next;
        if cursor.is_none() {
            return Ok(());
        }
    }
}

/// Each holder's tag from each hour a row it is party to was first read with it.
fn tags<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
) -> io::Result<BTreeMap<u32, Vec<(i32, String)>>> {
    let mut tags = BTreeMap::<u32, Vec<(i32, String)>>::new();
    events(snapshot, directory, emit::TAGS, kind::TAG, |hour, slot, tag| {
        let slot = u32::try_from(slot).map_err(|_| malformed())?;
        tags.entry(slot).or_default().push((hour, tag));
        Ok(())
    })?;
    Ok(tags)
}

/// Each organisation's naming operands at each read that changed them.
fn organisation_names<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
) -> io::Result<BTreeMap<u64, Vec<RawOrganisation>>> {
    let mut names = BTreeMap::<u64, Vec<RawOrganisation>>::new();
    events(
        snapshot,
        directory,
        emit::ORGANISATIONS,
        kind::ORGANISATION,
        |_, id, text| {
            let raw: RawOrganisation = serde_json::from_str(&text).map_err(|_| malformed())?;
            names.entry(id).or_default().push(raw);
            Ok(())
        },
    )?;
    Ok(names)
}

/// The naming reads of each design, never two at one hour.
fn design_names<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    designs: &BTreeSet<u64>,
) -> io::Result<BTreeMap<u64, Vec<VariantVersion>>> {
    let mut names = BTreeMap::<u64, Vec<VariantVersion>>::new();
    events(snapshot, directory, emit::VARIANTS, kind::VARIANT, |_, id, text| {
        if !designs.contains(&id) {
            return Ok(());
        }
        let version: VariantVersion = serde_json::from_str(&text).map_err(|_| malformed())?;
        let versions = names.entry(id).or_default();
        match versions.last_mut() {
            Some(last) if last.first_hour >= version.first_hour => *last = version,
            _ => versions.push(version),
        }
        Ok(())
    })?;
    Ok(names)
}

/// The text of each number, in order.
fn texts(directory: &Directory) -> io::Result<Vec<String>> {
    let mut texts = Vec::new();
    while let Some(text) = directory.scalar(emit::TEXTS + texts.len() as u64) {
        let text = text.text().ok_or_else(malformed)?;
        texts.push(String::from_utf8(text.to_vec()).map_err(|_| malformed())?);
    }
    Ok(texts)
}

/// Each refusal's hours over every hour the publication holds, as the catalogue counted them,
/// each counted as often as it held then.
fn refusals<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
) -> io::Result<BTreeMap<String, u64>> {
    let mut refusals = BTreeMap::new();
    for &(id, key) in directory.dimensions(ACTOR, emit::REFUSALS) {
        let reason = directory
            .scalar(emit::REASONS + id)
            .and_then(Scalar::text)
            .and_then(|text| std::str::from_utf8(text).ok())
            .ok_or_else(malformed)?
            .to_owned();
        let count: u64 = runs(snapshot, Some(key), i32::MAX)?
            .iter()
            .map(|EconomyRun(first, last, value)| {
                let hours = (i64::from(*last) - i64::from(*first) + 1) as u64;
                value.parse::<u64>().unwrap_or(0) * hours
            })
            .sum();
        if count != 0 {
            *refusals.entry(reason).or_default() += count;
        }
    }
    Ok(refusals)
}

/// The dimensions of `measures`' keys: a view's rows.
fn rows(directory: &Directory, measures: &[u64]) -> BTreeSet<u64> {
    measures
        .iter()
        .flat_map(|&measure| directory.dimensions(ACTOR, measure))
        .map(|&(dimension, _)| dimension)
        .collect()
}

/// A row's key of `measure`.
fn key(directory: &Directory, measure: u64, row: u64) -> Option<u64> {
    directory.key(ACTOR, measure, Some(row))
}

/// The reply the publication `snapshot` pins answers, through `through`.
pub fn query<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &ProductionCatalogueParams,
    through: i32,
    naming: &Naming<'_>,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<ProductionCatalogueReply> {
    let mut reply = ProductionCatalogueReply::empty(params, Some(through));
    let tags = tags(snapshot, directory)?;
    let chosen: Option<BTreeMap<u32, Vec<(i32, i32)>>> = (!params.tags.is_empty()).then(|| {
        tags.iter()
            .filter_map(|(&slot, runs)| {
                let spans = tag_spans(runs, &params.tags);
                (!spans.is_empty()).then_some((slot, spans))
            })
            .collect()
    });
    let domain = params.view.domain();
    reply.known = runs(
        snapshot,
        directory.key(ACTOR, emit::KNOWN, Some(u64::from(domain))),
        through,
    )?;
    let mut holders = BTreeSet::new();
    let mut licensed = BTreeSet::new();
    let mut refused_names = 0_usize;
    let designs = rows(directory, &[emit::DESIGN_CREATOR]);
    let variants = match params.view {
        CatalogueView::Organisations => BTreeMap::new(),
        _ => design_names(snapshot, directory, &designs)?,
    };
    let check = || {
        if cancelled.load(Ordering::Acquire) {
            Err(dashboard_base::invalid("Economy history request cancelled"))
        } else {
            Ok(())
        }
    };
    match params.view {
        CatalogueView::Licences => {
            let measures = [
                emit::LICENCE_GIVER,
                emit::LICENCE_UNIDENTIFIED,
                emit::LICENCE_LENT,
                emit::LICENCE_REQUIRED,
                emit::LICENCE_START,
            ];
            for row in rows(directory, &measures) {
                check()?;
                let (licensee, at) = ((row >> 32) as u32, row as u32);
                let party = runs(snapshot, key(directory, emit::LICENCE_GIVER, row), through)?;
                let unidentified = match &chosen {
                    Some(_) => runs(snapshot, key(directory, emit::LICENCE_UNIDENTIFIED, row), through)?,
                    None => Vec::new(),
                };
                let spans = selected_spans(chosen.as_ref(), &[licensee], &party, &unidentified);
                if spans.is_empty() {
                    continue;
                }
                let licensor = clip_text(&party, &spans);
                let parties = std::iter::once(licensee)
                    .chain(licensor.iter().filter_map(|run| run.2.parse().ok()))
                    .collect::<Vec<u32>>();
                let mut text = |measure: u64| -> io::Result<Vec<EconomyRun>> {
                    let runs = runs(snapshot, key(directory, measure, row), through)?;
                    Ok(clip_text(&runs, &spans))
                };
                let mut licence = LicenceHistory {
                    licensee: licensee.to_string(),
                    ordinal: at,
                    licensor,
                    lent: text(emit::LICENCE_LENT)?,
                    required: text(emit::LICENCE_REQUIRED)?,
                    start: text(emit::LICENCE_START)?,
                    variants: Vec::new(),
                };
                let mut lists = Vec::new();
                while key(directory, emit::LICENCE_VARIANTS + lists.len() as u64, row).is_some() {
                    lists.push(text(emit::LICENCE_VARIANTS + lists.len() as u64)?);
                }
                licence.variants = licensed_variants(lists.into_iter(), &mut licensed);
                holders.extend(parties);
                reply.licences.push(licence);
            }
            name_licensed(&mut reply, &variants, licensed, naming, &mut refused_names);
        }
        CatalogueView::Organisations => {
            let names = organisation_names(snapshot, directory)?;
            let measures = [
                emit::MIO_HOLDER,
                emit::MIO_SIZE,
                emit::MIO_FUNDS,
                emit::MIO_POINTS,
                emit::MIO_POLICY,
                emit::MIO_POLICY_NAME,
                emit::MIO_COOLDOWN,
                emit::MIO_UNLOCKED,
                emit::MIO_POLICY_UNRESOLVED,
            ];
            for id in rows(directory, &measures) {
                check()?;
                let party = runs(snapshot, key(directory, emit::MIO_HOLDER, id), through)?;
                let spans = selected_spans(chosen.as_ref(), &[], &party, &[]);
                if spans.is_empty() {
                    continue;
                }
                let mut text = |measure: u64| -> io::Result<Vec<EconomyRun>> {
                    let runs = runs(snapshot, key(directory, measure, id), through)?;
                    Ok(clip_text(&runs, &spans))
                };
                let mut mio = organisation(id, Vec::new());
                mio.size = text(emit::MIO_SIZE)?;
                mio.funds = text(emit::MIO_FUNDS)?;
                mio.points = text(emit::MIO_POINTS)?;
                mio.unlocked = text(emit::MIO_UNLOCKED)?;
                mio.policy = text(emit::MIO_POLICY)?;
                mio.policy_name = text(emit::MIO_POLICY_NAME)?;
                mio.policy_unresolved = text(emit::MIO_POLICY_UNRESOLVED)?;
                mio.cooldown = text(emit::MIO_COOLDOWN)?;
                mio.holder = clip_text(&party, &spans);
                holders.extend(mio.holder.iter().filter_map(|run| run.2.parse::<u32>().ok()));
                name_organisation(
                    &mut mio,
                    names.get(&id).map_or(&[][..], Vec::as_slice),
                    naming,
                    &mut reply.icons,
                    &mut warnings,
                    &mut refused_names,
                );
                reply.organisations.push(mio);
            }
        }
        CatalogueView::Designs => {
            let catalogue = VariantCatalogue::seeded(variants);
            for id in rows(directory, &[emit::DESIGN_CREATOR, emit::DESIGN_OBSOLETE]) {
                check()?;
                let party = runs(snapshot, key(directory, emit::DESIGN_CREATOR, id), through)?;
                let spans = selected_spans(chosen.as_ref(), &[], &party, &[]);
                if spans.is_empty() || party.is_empty() {
                    continue;
                }
                let obsolete = runs(snapshot, key(directory, emit::DESIGN_OBSOLETE, id), through)?;
                let mut design = design(id, clip_text(&party, &spans));
                design.obsolete = clip_text(&obsolete, &spans);
                holders.extend(
                    design
                        .creator
                        .iter()
                        .filter_map(|run| run.2.parse::<u32>().ok()),
                );
                name_design(
                    &mut design,
                    id,
                    Some(&catalogue),
                    naming,
                    &mut warnings,
                    &mut refused_names,
                );
                reply.designs.push(design);
            }
        }
    }
    if refused_names > 0 {
        warnings.push(format!(
            "{refused_names} names unavailable through the recorded content"
        ));
    }
    warnings.sort();
    warnings.dedup();
    reply.designs_from = directory
        .scalar(emit::DESIGNS_FROM)
        .and_then(Scalar::integer)
        .and_then(|hour| i32::try_from(hour).ok());
    reply.texts = texts(directory)?;
    reply.holders = holders
        .into_iter()
        .map(|slot| {
            let runs = tags.get(&slot).map(Vec::as_slice).unwrap_or_default();
            (slot.to_string(), holder_tags(runs))
        })
        .collect();
    warnings.extend(
        refusals(snapshot, directory)?
            .into_iter()
            .map(|(reason, hours)| format!("Production catalogue: {reason} ({hours} observed hours)")),
    );
    reply.warnings = warnings;
    Ok(reply)
}
