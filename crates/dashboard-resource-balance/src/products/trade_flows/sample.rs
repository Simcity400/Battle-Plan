//! Native per-frame flows; cumulative accounting remains in the prepared observer.
use super::{Identity, Inputs, Terms, flow_rows, flow_token};
use dashboard_base::prepared::query::progressive::Samples;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub(crate) const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::ResourceExchanges,
];
pub(crate) const FIELDS: [&str; 5] = [
    "delivered",
    "factories",
    "convoysAssigned",
    "convoysRequired",
    "deals",
];
pub(super) fn amounts(terms: Terms) -> [i128; 5] {
    [
        i128::from(terms.delivered),
        i128::from(terms.factories),
        i128::from(terms.assigned),
        i128::from(terms.required),
        i128::from(terms.deals),
    ]
}
pub(super) fn terms(amounts: [i128; 5]) -> io::Result<Terms> {
    let invalid = || dashboard_core::invalid("Invalid sampled native trade terms");
    Ok(Terms {
        delivered: amounts[0].try_into().map_err(|_| invalid())?,
        factories: amounts[1].try_into().map_err(|_| invalid())?,
        assigned: amounts[2].try_into().map_err(|_| invalid())?,
        required: amounts[3].try_into().map_err(|_| invalid())?,
        deals: amounts[4].try_into().map_err(|_| invalid())?,
    })
}
pub(crate) fn pair(token: &str) -> io::Result<(Identity, Identity, u32)> {
    let (seller, buyer, resource): (String, String, u32) = serde_json::from_str(token)?;
    Ok((
        serde_json::from_str(&seller)?,
        serde_json::from_str(&buyer)?,
        resource,
    ))
}

/// A reply's native input projections, refreshed from each visited frame's
/// actual table identities. Country balance changes retain the holder join;
/// unchanged exchanges and dictionaries retain their reduced operands.
#[derive(Default)]
pub(crate) struct Query {
    inputs: std::cell::RefCell<Inputs>,
    changes: std::cell::RefCell<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
}
impl Query {
    pub(crate) fn sample(
        &self,
        frame: &DecodedFrame,
        requested: Option<&[(String, String)]>,
        format: replay_data::format::FormatVersion,
    ) -> io::Result<Samples> {
        let cpu = replay_frame::cpu::scope_phase("trade:query_inputs");
        let changes = self.changes.borrow_mut().observe(frame, format);
        let mut inputs = self.inputs.borrow_mut();
        inputs.refresh(frame, &changes);
        drop(cpu);
        let _cpu = replay_frame::cpu::scope_phase("trade:query_flows");
        sample_inputs(frame, requested, &inputs)
    }
}

/// Country selectors have an empty field; native flow tokens select particular fields.
/// No absent holder or unavailable exchange table supplies zero.
#[cfg(test)]
pub(crate) fn sample(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> io::Result<Samples> {
    let mut inputs = Inputs::default();
    inputs.refresh(
        frame,
        &dashboard_base::prepared::frame_changes::FrameChanges {
            rows_changed: [true; replay_data::rows::TABLE_COUNT],
            table_read: [true; replay_data::rows::TABLE_COUNT],
            slot_read: [true; replay_data::rows::TABLE_COUNT],
            updates: None,
        },
    );
    sample_inputs(frame, requested, &inputs)
}
pub(super) fn sample_inputs(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
    inputs: &Inputs,
) -> io::Result<Samples> {
    let mut result = BTreeMap::new();
    let mut selectors = BTreeSet::new();
    let mut wanted = BTreeSet::new();
    let mut actors = BTreeSet::new();
    if let Some(requested) = requested {
        for (tag, field) in requested {
            if field.is_empty() {
                selectors.insert(tag.as_str());
            } else if field == "actor" {
                actors.insert(serde_json::from_str::<Identity>(tag)?);
            } else if field != "holder"
                && field != "rejected"
                && !(tag.is_empty() && ["known", "partial"].contains(&field.as_str()))
            {
                pair(tag)?;
                if !FIELDS.contains(&field.as_str()) {
                    return Err(dashboard_core::invalid("Unknown trade sample field"));
                }
                wanted.insert((tag.as_str(), field.as_str()));
            }
        }
    }
    let all = requested.is_none() || requested.is_some_and(|keys| keys.is_empty());
    let holders = inputs.identities();
    for (&slot, &(tag, id)) in &holders {
        result.insert((tag.to_owned(), "holder".into()), (Some((slot, id)), None));
        if selectors.contains(tag) {
            actors.insert((slot, id));
        }
    }
    let mut rejected = Vec::new();
    let mut wanted_holders = actors.clone();
    for &(token, _) in &wanted {
        let (seller, buyer, _) = pair(token)?;
        wanted_holders.extend([seller, buyer]);
    }
    let rows = if frame.complete::<tables::CountryLinks>().is_ok() {
        flow_rows(
            inputs,
            &holders,
            (!all).then_some(&wanted_holders),
            &mut rejected,
        )?
    } else {
        None
    };
    let known = rows.is_some();
    result.insert((String::new(), "known".into()), (None, known.then_some(1)));
    let partial: i128 = rejected
        .iter()
        .filter(|(pair, _)| {
            all || pair
                .is_none_or(|(seller, buyer)| actors.contains(&seller) || actors.contains(&buyer))
        })
        .map(|(_, count)| i128::from(*count))
        .sum();
    result.insert(
        (String::new(), "partial".into()),
        (None, known.then_some(partial)),
    );
    for (pair, count) in rejected {
        if all
            || pair
                .is_none_or(|(seller, buyer)| actors.contains(&seller) || actors.contains(&buyer))
        {
            let key = (serde_json::to_string(&pair)?, "rejected".into());
            let row = result.entry(key).or_insert((None, Some(0)));
            row.1 = Some(row.1.unwrap_or(0) + i128::from(count));
        }
    }
    if let Some(rows) = rows {
        for ((seller, buyer, resource), (seller_tag, buyer_tag, terms)) in rows {
            let token = flow_token(seller, buyer, resource);
            let selected = all
                || selectors.contains(seller_tag)
                || selectors.contains(buyer_tag)
                || actors.contains(&seller)
                || actors.contains(&buyer);
            for (field, value) in FIELDS.iter().zip(amounts(terms)) {
                if selected || wanted.contains(&(token.as_str(), *field)) {
                    result.insert(
                        (token.clone(), (*field).to_owned()),
                        (Some(seller), Some(value)),
                    );
                }
            }
        }
        for &(token, field) in &wanted {
            let (seller, buyer, resource) = pair(token)?;
            if holders
                .get(&seller.0)
                .is_some_and(|(_, id)| *id == seller.1)
                && holders.get(&buyer.0).is_some_and(|(_, id)| *id == buyer.1)
                && inputs
                    .names
                    .as_ref()
                    .is_some_and(|names| names.contains(resource))
            {
                result
                    .entry((token.to_owned(), field.to_owned()))
                    .or_insert((Some(seller), Some(0)));
            }
        }
    }
    if let Some(requested) = requested {
        for key in requested.iter().filter(|(_, field)| !field.is_empty()) {
            result.entry(key.clone()).or_insert((None, None));
        }
    }
    Ok(result)
}
