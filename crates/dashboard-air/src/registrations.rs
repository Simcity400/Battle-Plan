dashboard_base::register_products! {
    AIRCRAFT_CATALOGUE_AIRCRAFTCATALOGUE => crate::products::aircraft_catalogue::AircraftCatalogue,
    AIR_LEDGER_MILITARYAIRLEDGER => crate::products::air_ledger::MilitaryAirLedger,
    AIR_LEDGER_AT_MILITARYAIRAT => crate::products::air_ledger::at::MilitaryAirAt,
    REGIONAL_WEATHER_REGIONALWEATHER => crate::products::regional_weather::RegionalWeather,
    AIR_SUPERIORITY_AIRSUPERIORITYCHOICES => crate::products::air_superiority::AirSuperiorityChoices,
    AIR_STATISTICS_AIRSTATISTICSREGIONS => crate::products::air_statistics::AirStatisticsRegions,
    AIR_SUPERIORITY_TIMELINE_AIRSUPERIORITYTIMELINE => crate::products::air_superiority::timeline::AirSuperiorityTimeline,
    AIR_SUPERIORITY_TIMELINE_AIRSUPERIORITYRANKING => crate::products::air_superiority::timeline::AirSuperiorityRanking,
}

dashboard_base::register_hourly_products! {
    AIR_STATISTICS_AIRSTATISTICSAT => crate::products::air_statistics::AirStatisticsAt,
    AIR_ACES_AIRACESAT => crate::products::air_aces::AirAcesAt,
    REGIONAL_WEATHER_WEATHERPROVINCES => crate::products::regional_weather::WeatherProvinces,
    AIR_MISSIONS_AIRMISSIONSAT => crate::products::air_missions::AirMissionsAt,
    AIR_SUPERIORITY_AIRSUPERIORITYAT => crate::products::air_superiority::AirSuperiorityAt,
}
