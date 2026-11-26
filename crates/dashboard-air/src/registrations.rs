use dashboard_base::products::at_hour::at_hour;

dashboard_base::register_products! {
    AIRCRAFT_CATALOGUE_AIRCRAFTCATALOGUE => crate::products::aircraft_catalogue::AircraftCatalogue,
    AIR_LEDGER_MILITARYAIRLEDGER => crate::products::air_ledger::MilitaryAirLedger,
    AIR_LEDGER_AT_MILITARYAIRAT => crate::products::air_ledger::at::MilitaryAirAt,
    AIR_LEDGER_BREAKDOWN_BOMBINGDAMAGEBREAKDOWN => crate::products::air_ledger::breakdown::BombingDamageBreakdown; interval(at_hour, |r| r.through),
    REGIONAL_WEATHER_REGIONALWEATHER => crate::products::regional_weather::RegionalWeather; interval(|request, hour| request["lastHour"] = hour.into(), |r| r.valid_through),
    AIR_SUPERIORITY_AIRSUPERIORITYCHOICES => crate::products::air_superiority::AirSuperiorityChoices,
    AIR_STATISTICS_AIRSTATISTICSREGIONS => crate::products::air_statistics::AirStatisticsRegions,
    AIR_SUPERIORITY_TIMELINE_AIRSUPERIORITYTIMELINE => crate::products::air_superiority::timeline::AirSuperiorityTimeline; day(DayThroughHour, "firstHour", "lastHour", |r| r.last_hour),

    @hourly
    AIR_STATISTICS_AIRSTATISTICSAT => whole_frame<crate::products::air_statistics::AirStatisticsAt>,
    AIR_ACES_AIRACESAT => whole_frame<crate::products::air_aces::AirAcesAt>,
    AIR_MISSIONS_AIRMISSIONSAT => whole_frame<crate::products::air_missions::AirMissionsAt>,
    AIR_SUPERIORITY_AIRSUPERIORITYAT => whole_frame<crate::products::air_superiority::AirSuperiorityAt>,
}
