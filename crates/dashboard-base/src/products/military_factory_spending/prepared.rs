//! Military factory spending reads production's prepared endpoints and spent
//! series, through the hours production has prepared.
use super::*;
use crate::Source;
use crate::products::equipment::production::Purpose;
use crate::products::production_spending::{
    Purposes,
    prepared::{Params as SpendingParams, query_with},
};
use crate::products::purpose_split::{keep_domain, purpose_months};
use serde_json::{Value, json};
use std::{io, sync::atomic::AtomicBool};

/// The purposes military factory spending shows.
const SHOWN: [Purpose; 2] = [Purpose::New, Purpose::Converted];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    tag: String,
    view: DockyardView,
    equipment: FactoryEquipment,
    from: Option<i32>,
    hour: Option<i32>,
    end: Option<i32>,
}

pub fn query(
    source: &Source,
    params: Value,
    content: Option<&super::super::ContentOptions>,
    cancel: &AtomicBool,
) -> io::Result<Option<Value>> {
    let input: Params = serde_json::from_value(params)?;
    let domain = input.equipment.domain();
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
        MilitaryFactorySpending::KEY,
        Purposes::Split,
        cancel,
    )?;
    let other = match input.view {
        DockyardView::Circle => {
            data["months"] = json!([]);
            keep_domain(&mut data, domain, &SHOWN)?
        }
        DockyardView::Months => {
            let (store, key) = crate::prepared::files(source, content)?;
            let months = purpose_months(
                &store,
                &key,
                &input.tag,
                &answered,
                domain,
                cancel,
            )?;
            let mut other = 0;
            let mut columns = Vec::with_capacity(months.len());
            for month in months {
                let remainder = month.other(&SHOWN);
                other += usize::from(remainder.is_ok_and(|amount| amount != 0));
                columns.push(FactoryMonth {
                    first_hour: month.first_hour,
                    last_hour: month.last_hour,
                    new: DockyardAmount::of(month.part(Purpose::New)),
                    converted: DockyardAmount::of(month.part(Purpose::Converted)),
                    unrecorded: DockyardAmount::of(month.part(Purpose::Unknown)),
                    other: DockyardAmount::of(remainder),
                    purpose_unproven: month.purpose_unproven,
                    unresolved: month.unresolved,
                });
            }
            data["leaves"] = json!([]);
            data["peaks"] = json!({});
            data["months"] = serde_json::to_value(columns)?;
            other
        }
    };
    if other > 0 {
        let unit = match input.view {
            DockyardView::Circle => "designs",
            DockyardView::Months => "months",
        };
        let equipment = match input.equipment {
            FactoryEquipment::Air => "air",
            FactoryEquipment::Land => "land",
        };
        data["warnings"]
            .as_array_mut()
            .ok_or_else(|| crate::invalid("Military factory spending warnings are not a list"))?
            .push(json!(format!(
                "{other} {equipment} {unit} had ship refit input, which is neither new nor converted; it is shown apart."
            )));
    }
    Ok(Some(data))
}
