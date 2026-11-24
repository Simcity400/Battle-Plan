use dashboard_base::Source;
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;

pub fn panel_of(source: &Source, load: &LoadContext, asked: &Value) -> Option<Result<Box<dyn Panel>, String>> {
    let kind = asked.get("panel")?.as_str()?;
    let entry = crate::products::AT_HOUR.iter().find(|entry| entry.registered.key == kind)?;
    Some((entry.panel)(source, load, asked))
}

#[cfg(test)]
mod registry_tests;

pub fn dated_panel_of(source: &Source, load: &LoadContext, asked: &Value) -> Option<Result<Box<dyn Panel>, String>> {
    Some(match asked["panel"].as_str()? {
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
        "occupied-states" => dashboard_wars::registrations::EXACT_OCCUPIEDSTATES(source, load, asked),
        "politics-state" => dashboard_politics::registrations::EXACT_POLITICSSTATE(source, load, asked),
        "air-force-at" => dashboard_land::registrations::EXACT_AIRFORCEAT(source, load, asked),
        "navy-at" => dashboard_land::registrations::EXACT_NAVYAT(source, load, asked),
        "army-at" => dashboard_land::registrations::EXACT_ARMYAT(source, load, asked),
        "factory-mix" => dashboard_world::registrations::EXACT_FACTORYMIX(source, load, asked),
        "faction-totals-at" => dashboard_wars::registrations::EXACT_FACTIONTOTALSAT(source, load, asked),
        "log-tail" => dashboard_politics::products::logs::dated_panel(source, load, asked),
        "market-ledger-tail" => dashboard_economy::products::economy::market::ledger::dated_panel(source, load, asked),
        "raid-log-tail" => dashboard_intelligence::products::raids::tail::dated_panel(source, load, asked),
        "event-feed-tail" => dashboard_politics::products::feed::event_feed_dated_panel(source, load, asked),
        "event-choices-tail" => dashboard_politics::products::feed::event_choices_dated_panel(source, load, asked),
        "decisions-tail" => dashboard_politics::products::feed::decisions_dated_panel(source, load, asked),
        _ => return None,
    })
}
