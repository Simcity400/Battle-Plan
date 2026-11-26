use dashboard_base::products::at_hour::{at_hour, before};

dashboard_base::register_products! {
    ARTWORK_SPRITEARTWORK => dashboard_base::products::artwork::sprites::SpriteArtwork,
    COUNTRY_FLAGS_COUNTRYFLAGS => crate::products::country_flags::CountryFlags,
    IDENTITIES_COUNTRYIDENTITIES => crate::products::identities::CountryIdentities,
    STYLES_COUNTRYFACTS => crate::products::styles::CountryFacts,
    STYLES_COUNTRYSTYLES => crate::products::styles::CountryStyles,
    GROUPS_COUNTRYGROUPS => crate::products::groups::CountryGroups,
    GROUP_COUNTS_GROUPCOUNTS => crate::products::group_counts::GroupCounts; interval(at_hour, |r| before(r.next_change)),
    HISTORY_AT_COUNTRYGROUPSAT => crate::products::history_at::CountryGroupsAt; interval(at_hour, |r| before(r.next_change)),
    HISTORY_AT_SUBJECTSAT => crate::products::history_at::SubjectsAt; interval(at_hour, |r| before(r.next_change)),
    LAND_OWNERS_LANDOWNERSPRODUCT => crate::products::land_owners::LandOwnersProduct,
    SCALARS_WORLDTENSIONSTACK => crate::products::scalars::WorldTensionStack,
    SCALARS_FACTORYGROUPS => crate::products::scalars::FactoryGroups,
    PLACE_NAME_RUNS_PLACENAMERUNS => crate::products::place_name_runs::PlaceNameRuns,
    SEA_REGIONS_STRATEGICREGIONS => crate::products::sea_regions::StrategicRegions,

    @hourly
    TERRITORY_TERRITORYSHARE => exact<crate::products::territory::TerritoryShare>,
    TERRITORY_TERRITORYBREAKDOWN => whole_frame<crate::products::territory::TerritoryBreakdown>,
    SCALARS_GAMESCORE => exact<crate::products::scalars::GameScore>,
    SCALARS_NUCLEARARSENAL => exact<crate::products::scalars::NuclearArsenal>,
    SCALARS_WORLDTENSION => whole_frame<crate::products::scalars::WorldTension>,
    SCALARS_FACTORYPARTS => whole_frame<crate::products::scalars::FactoryParts>,
    SCALARS_FACTORYMIX => exact<crate::products::scalars::FactoryMix>,
    AUTONOMY_LEVELS_AUTONOMYLEVELS => whole_frame<crate::products::autonomy_levels::AutonomyLevels>,
}
