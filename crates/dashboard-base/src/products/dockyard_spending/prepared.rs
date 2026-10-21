//! Dockyard spending reads production's prepared endpoints and spent series and
//! the prepared repair rate, through the hours both families have prepared.
use super::*;
use crate::Source;
use crate::prepared_production::ProductionNaming;
use crate::products::dockyard_repairs::{RepairPart, RepairReads};
use crate::products::equipment::Domain;
use crate::products::equipment::production::{Purpose, variant_types_for};
use crate::products::production_spending::{
    Purposes,
    prepared::{Answered, Params as SpendingParams, query_with},
};
use crate::products::purpose_split::{keep_domain, purpose_months};
use serde_json::{Value, json};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

/// The purposes dockyard spending shows.
const SHOWN: [Purpose; 2] = [Purpose::New, Purpose::Refit];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    tag: String,
    view: DockyardView,
    from: Option<i32>,
    hour: Option<i32>,
    end: Option<i32>,
}

fn cancelled(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Dockyard spending request cancelled",
        ))
    } else {
        Ok(())
    }
}

pub fn query(
    source: &Source,
    params: Value,
    content: Option<&super::super::ContentOptions>,
    cancel: &AtomicBool,
) -> io::Result<Option<Value>> {
    let input: Params = serde_json::from_value(params)?;
    let spending = SpendingParams {
        branch_id: input.branch_id.clone(),
        tag: input.tag.clone(),
        refits: true,
        from: input.from,
        hour: input.hour,
        end: input.end,
    };
    let (mut data, answered) = query_with(
        source,
        spending,
        content,
        DockyardSpending::KEY,
        Purposes::Split,
        cancel,
    )?;
    let (store, key) = crate::prepared::files(source, content)?;
    let mut repairs = RepairReads::new(&store, &key, answered.branch, cancel)?;
    let mismatches = repairs.allocation_mismatches();
    if mismatches > 0 {
        data["warnings"]
            .as_array_mut()
            .ok_or_else(|| crate::invalid("Dockyard spending warnings are not a list"))?
            .push(json!(format!(
                "{mismatches} repair reads had naval base assignments that did not sum to their country's dockyards in use; repairs there are not split by design."
            )));
    }
    match input.view {
        DockyardView::Circle => {
            let other = keep_domain(&mut data, Domain::Naval, &SHOWN)?;
            if other > 0 {
                warn(&mut data, other, "designs")?;
            }
            // A whole campaign's repairs start where the country's output is
            // first known, named when that is after the recording's start.
            let origin = repairs.origin(&input.tag)?;
            let start = answered.from.or(origin);
            let since =
                origin.filter(|&origin| answered.from.is_none() && origin > answered.bounds.0);
            let ends = [answered.hour, answered.end];
            let [hour, end] =
                window_integrals(start, ends, |spans| repairs.integrals(&input.tag, spans))?;
            let parts = repair_parts(source, content, &mut repairs, &input.tag, start, ends)?;
            data["repairs"] = serde_json::to_value(DockyardRepairs {
                denominator: REPAIR_DENOMINATOR,
                hour: DockyardAmount::of(hour),
                end: DockyardAmount::of(end),
                since,
                parts,
            })?;
            data["months"] = json!([]);
        }
        DockyardView::Months => {
            let (months, other) =
                month_columns(&store, &key, &input.tag, &answered, &mut repairs, cancel)?;
            if other > 0 {
                warn(&mut data, other, "months")?;
            }
            data["leaves"] = json!([]);
            data["peaks"] = json!({});
            data["repairs"] = serde_json::to_value(DockyardRepairs {
                denominator: REPAIR_DENOMINATOR,
                ..Default::default()
            })?;
            data["months"] = serde_json::to_value(months)?;
        }
    }
    Ok(Some(data))
}

pub(super) type Integral = Result<i128, ProductionSpendingUnknown>;

/// Repair integrals from `start` through each of `ends`, from `integrals` of
/// the nonempty spans. A window ending before the rate's origin, or without
/// one, is not known: no output was recorded there, which is not zero.
pub(super) fn window_integrals(
    start: Option<i32>,
    ends: [i32; 2],
    integrals: impl FnOnce(&[(i32, i32)]) -> io::Result<Vec<Integral>>,
) -> io::Result<[Integral; 2]> {
    let Some(start) = start else {
        return Ok([Err(ProductionSpendingUnknown::NotRecorded); 2]);
    };
    let spans: Vec<(i32, i32)> = ends
        .iter()
        .filter(|&&end| end >= start)
        .map(|&end| (start, end))
        .collect();
    let mut integrals = integrals(&spans)?.into_iter();
    Ok(ends.map(|end| {
        if end < start {
            Err(ProductionSpendingUnknown::NotRecorded)
        } else {
            integrals
                .next()
                .unwrap_or(Err(ProductionSpendingUnknown::NotRecorded))
        }
    }))
}

/// Each repaired design's share of the window's repairs, named and typed
/// through the recorded content stack, and the unattributed rest.
fn repair_parts(
    source: &Source,
    content: Option<&super::super::ContentOptions>,
    repairs: &mut RepairReads<'_>,
    tag: &str,
    start: Option<i32>,
    ends: [i32; 2],
) -> io::Result<Vec<DockyardRepairPart>> {
    let parts = repairs.parts(tag)?;
    if parts.is_empty() {
        return Ok(Vec::new());
    }
    let naming = ProductionNaming::load(
        source,
        &super::super::LoadContext {
            content: content.cloned(),
        },
    );
    let named = naming.named();
    let designs = parts.iter().filter_map(|part| match part {
        RepairPart::Design(variant) => Some(*variant),
        RepairPart::Unattributed => None,
    });
    let (types, _, _) = variant_types_for(repairs.variants(), designs, named);
    let mut result = Vec::with_capacity(parts.len());
    for part in parts {
        let described = match part {
            RepairPart::Design(variant) => {
                let kind = types.get(&variant).expect("every repaired design is typed");
                DockyardRepairPart {
                    variant: Some(variant.to_string()),
                    type_key: kind.key.clone(),
                    name: named.type_name(&kind.key),
                    domain: kind.domain,
                    names: repairs.variants().names(variant, named.texts),
                    ..Default::default()
                }
            }
            RepairPart::Unattributed => DockyardRepairPart::default(),
        };
        let [hour, end] = window_integrals(start, ends, |spans| {
            repairs.part_integrals(tag, part, spans)
        })?;
        result.push(DockyardRepairPart {
            hour: DockyardAmount::of(hour),
            end: DockyardAmount::of(end),
            ..described
        });
    }
    Ok(result)
}

/// Naval input of another purpose than new ships and refits is counted in
/// the whole as its own part and in the reply's warnings.
fn warn(data: &mut Value, count: usize, unit: &str) -> io::Result<()> {
    data["warnings"]
        .as_array_mut()
        .ok_or_else(|| crate::invalid("Dockyard spending warnings are not a list"))?
        .push(json!(format!(
            "{count} naval {unit} had converted input, which is neither new ships nor refits; it is shown apart."
        )));
    Ok(())
}

/// The calendar months from the window's first hour through its hour, each
/// with exact new, refit, repair, unrecorded-purpose and other-purpose
/// amounts or the reason one is unknown, and how many had naval input of
/// another purpose.
fn month_columns(
    store: &store::Store,
    key: &store::SourceKey,
    tag: &str,
    answered: &Answered,
    repairs: &mut RepairReads<'_>,
    cancel: &AtomicBool,
) -> io::Result<(Vec<DockyardMonth>, usize)> {
    let months = purpose_months(store, key, tag, answered, Domain::Naval, cancel)?;
    let spans: Vec<(i32, i32)> = months
        .iter()
        .map(|month| (month.first_hour, month.last_hour))
        .collect();
    let repaired = repairs.integrals(tag, &spans)?;
    let mut result = Vec::with_capacity(months.len());
    let mut other = 0;
    for (month, repair) in months.into_iter().zip(repaired) {
        cancelled(cancel)?;
        let remainder = month.other(&SHOWN);
        other += usize::from(remainder.is_ok_and(|amount| amount != 0));
        result.push(DockyardMonth {
            first_hour: month.first_hour,
            last_hour: month.last_hour,
            new_ships: DockyardAmount::of(month.part(Purpose::New)),
            refits: DockyardAmount::of(month.part(Purpose::Refit)),
            repairs: DockyardAmount::of(repair),
            unrecorded: DockyardAmount::of(month.part(Purpose::Unknown)),
            other: DockyardAmount::of(remainder),
            purpose_unproven: month.purpose_unproven,
            unresolved: month.unresolved,
        });
    }
    Ok((result, other))
}
