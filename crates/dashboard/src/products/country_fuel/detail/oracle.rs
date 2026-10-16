//! Detail parity replies reuse one real raw-read scan per oracle stage.
use super::*;
use serde_json::Value;

pub(in crate::products::country_fuel) fn replies(
    source: &Source,
    first: i32,
    windows: &[i32],
) -> Vec<Vec<Value>> {
    let flag = AtomicBool::new(false);
    let raw = if source.path().is_relative() {
        let summary = scanned(source, 1, &flag).unwrap();
        let mut hours = BTreeSet::from([first]);
        for &window in windows {
            let (first, last) =
                super::super::super::production_lines::detail_window(window).unwrap();
            for country in summary.countries.values() {
                hours.extend(
                    country
                        .reads
                        .iter()
                        .filter(|read| read.last_hour >= first && read.first_hour <= last)
                        .map(|read| read.first_hour),
                );
            }
        }
        // Keep the scanner as the oracle; share its actual decoded reads across
        // the basis and windows instead of decoding the same blocks per query.
        Some(Arc::new(scan_reads(source, 1, &hours, &flag).unwrap()))
    } else {
        None
    };
    windows
        .iter()
        .map(|&window| {
            if let Some(raw) = &raw {
                let state = source.state::<DetailCache>();
                let mut cache = state.lock().unwrap();
                cache.window = Some(((1, window), Arc::clone(raw)));
                cache.basis = Some(((1, first), Arc::clone(raw)));
            }
            detail_replies(source, first, window)
        })
        .collect()
}

fn detail_replies(source: &Source, first: i32, window: i32) -> Vec<Value> {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let basis = FuelAwardsParams {
        branch_id: "1".into(),
        tag: "X".into(),
        hour: first,
        country: "11".into(),
        country_instance: 9,
    };
    let awards = CountryFuelAwards::load(source, &basis, &context, &flag).unwrap();
    let demand = demand::CountryFuelDemand::load(source, &basis, &context, &flag).unwrap();
    let key = demand.parts[0].key.clone();
    let award_history = detail::CountryFuelAwardHistory::load(
        source,
        &detail::FuelAwardHistoryParams {
            branch_id: basis.branch_id.clone(),
            tag: basis.tag.clone(),
            hour: basis.hour,
            country: basis.country.clone(),
            country_instance: basis.country_instance,
            window,
            recipient_key: awards.awards.recipients[0].key.clone(),
        },
        &context,
        &flag,
    )
    .unwrap();
    let demand_history = demand::CountryFuelDemandHistory::load(
        source,
        &demand::FuelDemandHistoryParams {
            branch_id: basis.branch_id.clone(),
            tag: basis.tag.clone(),
            hour: basis.hour,
            country: basis.country.clone(),
            country_instance: basis.country_instance,
            window,
            key: key.clone(),
        },
        &context,
        &flag,
    )
    .unwrap();
    let demand_stack = demand::CountryFuelDemandStack::load(
        source,
        &demand::FuelDemandStackParams {
            branch_id: basis.branch_id,
            tag: basis.tag,
            hour: basis.hour,
            country: basis.country,
            country_instance: basis.country_instance,
            window,
            keys: vec![key],
        },
        &context,
        &flag,
    )
    .unwrap();
    vec![
        serde_json::to_value(demand).unwrap(),
        serde_json::to_value(award_history).unwrap(),
        serde_json::to_value(demand_history).unwrap(),
        serde_json::to_value(demand_stack).unwrap(),
    ]
}
