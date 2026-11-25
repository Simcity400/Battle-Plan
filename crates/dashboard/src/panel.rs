use dashboard_base::Source;
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;
use std::sync::Arc;
use dashboard_base::products::at_hour::{interval_panel, product_interval_panel};

pub fn panel_of(source: &Source, load: &LoadContext, asked: &Value) -> Option<Result<Box<dyn Panel>, String>> {
    let kind = asked.get("panel")?.as_str()?;
    let entry = crate::products::AT_HOUR.iter().find(|entry| entry.registered.key == kind)?;
    Some((entry.panel)(source, load, asked))
}

#[cfg(test)]
mod registry_tests;

fn at_hour(params: &mut Value, hour: i32) {
    params["hour"] = hour.into();
}

pub fn dated_panel_of(source: &Arc<Source>, load: &LoadContext, asked: &Value) -> Option<Result<Box<dyn Panel>, String>> {
    Some(match asked["panel"].as_str()? {
        "diplomatic-relations" => dashboard_wars::registrations::EXACT_DIPLOMATICRELATIONS(source, load, asked),
        "intel-network-coverage" => dashboard_intelligence::registrations::EXACT_INTELNETWORKCOVERAGE(source, load, asked),
        "power-spending-breakdown" => product_interval_panel::<dashboard_politics::products::power_ledger::breakdown::PowerSpendingBreakdown>(source, load, asked, at_hour, |r| r.through),
        "production-spending" => product_interval_panel::<dashboard_production::products::production_spending::ProductionSpending>(source, load, asked, at_hour, |r| r.through),
        "construction-breakdown" => product_interval_panel::<dashboard_production::products::construction::breakdown::ConstructionBreakdown>(source, load, asked, at_hour, |r| r.through),
        "equipment-losses" => product_interval_panel::<dashboard_production::products::equipment_losses::EquipmentLosses>(source, load, asked, at_hour, |r| r.through),
        "casualty-breakdown" => product_interval_panel::<dashboard_wars::products::casualties::CasualtyBreakdown>(source, load, asked, at_hour, |r| r.through),
        "bombing-damage-breakdown" => product_interval_panel::<dashboard_air::products::air_ledger::breakdown::BombingDamageBreakdown>(source, load, asked, at_hour, |r| r.through),
        "focus-card" => product_interval_panel::<dashboard_politics::products::focus_card::FocusCard>(source, load, asked, at_hour, |r| r.valid_through),
        "regional-weather" => {
            let first = dashboard_query::branch(asked).and_then(|branch| dashboard_base::queries::query(source, branch)).map(|q| q.timeline.first);
            first.map_err(|e| e.to_string()).and_then(|first| product_interval_panel::<dashboard_air::products::regional_weather::RegionalWeather>(
                source, load, asked, move |request, hour| {
                    request["firstHour"] = first.max(hour.saturating_sub(7 * 24 - 1)).into();
                    request["lastHour"] = hour.into();
                }, |r| r.last_hour))
        }
        "series-rank" => rank_panel(source, load, asked),
        "special-projects" => dashboard_research::products::special_projects::dated_panel(source, load, asked),
        "special-project-details" => dashboard_research::registrations::EXACT_SPECIALPROJECTDETAILSAT(source, load, asked),
        "doctrine-mastery" => dashboard_land::registrations::EXACT_DOCTRINEMASTERY(source, load, asked),
        "research-slots" => dashboard_research::registrations::EXACT_RESEARCHSLOTS(source, load, asked),
        "nuclear-arsenal" => dashboard_world::registrations::EXACT_NUCLEARARSENAL(source, load, asked),
        "game-score" => dashboard_world::registrations::EXACT_GAMESCORE(source, load, asked),
        "production-lines-at" => dashboard_production::registrations::EXACT_PRODUCTIONLINESAT(source, load, asked),
        "agency-comparison" => dashboard_intelligence::registrations::EXACT_AGENCYCOMPARISON(source, load, asked),
        "territory-share" => dashboard_world::registrations::EXACT_TERRITORYSHARE(source, load, asked),
        "occupation-reserve-manpower" => dashboard_wars::registrations::EXACT_OCCUPATIONRESERVE(source, load, asked),
        "lend-lease-balance" => dashboard_economy::registrations::EXACT_LENDLEASEBALANCE(source, load, asked),
        "fuel-branches" => dashboard_economy::registrations::EXACT_FUELBRANCHES(source, load, asked),
        "occupied-states" => dashboard_wars::registrations::EXACT_OCCUPIEDSTATES(source, load, asked),
        "politics-state" => dashboard_politics::registrations::EXACT_POLITICSSTATE(source, load, asked),
        "air-force-at" => dashboard_land::registrations::EXACT_AIRFORCEAT(source, load, asked),
        "navy-at" => dashboard_land::registrations::EXACT_NAVYAT(source, load, asked),
        "army-at" => dashboard_land::registrations::EXACT_ARMYAT(source, load, asked),
        "factory-mix" => dashboard_world::registrations::EXACT_FACTORYMIX(source, load, asked),
        "faction-totals-at" => dashboard_wars::registrations::EXACT_FACTIONTOTALSAT(source, load, asked),
        "faction-pools" => dashboard_wars::registrations::EXACT_FACTIONPOOLS(source, load, asked),
        "log-tail" => dashboard_politics::products::logs::dated_panel(source, load, asked),
        "market-ledger-tail" => dashboard_economy::products::economy::market::ledger::dated_panel(source, load, asked),
        "raid-log-tail" => dashboard_intelligence::products::raids::tail::dated_panel(source, load, asked),
        "division-service-record-tail" => dashboard_land::products::divisions::careers::service_record::dated_panel(source, load, asked),
        "division-log-tail" => dashboard_land::products::division_log::dated_panel(source, load, asked),
        "espionage-log-tail" => dashboard_intelligence::products::intelligence::espionage_tail::dated_panel(source, load, asked),
        "construction-log-tail" => dashboard_production::products::construction::tail::dated_panel(source, load, asked),
        "event-feed-tail" => dashboard_politics::products::feed::event_feed_dated_panel(source, load, asked),
        "event-choices-tail" => dashboard_politics::products::feed::event_choices_dated_panel(source, load, asked),
        "decisions-tail" => dashboard_politics::products::feed::decisions_dated_panel(source, load, asked),
        _ => return None,
    })
}

fn rank_panel(source: &Arc<Source>, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    let source = Arc::clone(source);
    let load = LoadContext { content: load.content.clone() };
    let mut params = asked.clone();
    params.as_object_mut().ok_or("Expected rank parameters")?.remove("panel");
    Ok(interval_panel(source.tables(), move |hour, cancelled| {
        let mut request = params.clone();
        at_hour(&mut request, hour);
        let reply = crate::answer::answer(&source, dashboard_query::series::ranked::PRODUCT, &request, &load, cancelled)?;
        let through = reply["until"].as_i64().and_then(|n| i32::try_from(n).ok()).unwrap_or(hour);
        let rows = reply["rows"].as_array().map_or(0, |rows| rows.len() as u64);
        Ok((reply, through, rows))
    }))
}
