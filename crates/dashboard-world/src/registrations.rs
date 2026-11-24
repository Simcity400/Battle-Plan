dashboard_base::register_products! {
    ARTWORK_SPRITEARTWORK => dashboard_base::products::artwork::sprites::SpriteArtwork,
    COUNTRY_FLAGS_COUNTRYFLAGS => crate::products::country_flags::CountryFlags,
    IDENTITIES_COUNTRYIDENTITIES => crate::products::identities::CountryIdentities,
    STYLES_COUNTRYFACTS => crate::products::styles::CountryFacts,
    STYLES_COUNTRYSTYLES => crate::products::styles::CountryStyles,
    GROUPS_COUNTRYGROUPS => crate::products::groups::CountryGroups,
    LAND_OWNERS_LANDOWNERSPRODUCT => crate::products::land_owners::LandOwnersProduct,
    SCALARS_THREATEVENTS => crate::products::scalars::ThreatEvents,
    SCALARS_FACTORYGROUPS => crate::products::scalars::FactoryGroups,
    SUBJECTS_SUBJECTS => crate::products::subjects::Subjects,
    PLACE_NAMES_PLACENAMES => crate::products::place_names::PlaceNames,
    SEA_REGIONS_STRATEGICREGIONS => crate::products::sea_regions::StrategicRegions,
}

dashboard_base::register_hourly_products! {
    TERRITORY_TERRITORYSHARE => crate::products::territory::TerritoryShare,
    TERRITORY_TERRITORYBREAKDOWN => crate::products::territory::TerritoryBreakdown,
    SCALARS_GAMESCORE => crate::products::scalars::GameScore,
    SCALARS_NUCLEARARSENAL => crate::products::scalars::NuclearArsenal,
    SCALARS_WORLDTENSION => crate::products::scalars::WorldTension,
    SCALARS_FACTORYPARTS => crate::products::scalars::FactoryParts,
}

use dashboard_base::{Source, products::at_hour::exact_panel};
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;

type ExactPanel = fn(&Source, &LoadContext, &Value) -> Result<Box<dyn Panel>, String>;

pub const EXACT_NUCLEARARSENAL: ExactPanel = exact_panel::<crate::products::scalars::NuclearArsenal>;
pub const EXACT_GAMESCORE: ExactPanel = exact_panel::<crate::products::scalars::GameScore>;
pub const EXACT_TERRITORYSHARE: ExactPanel = exact_panel::<crate::products::territory::TerritoryShare>;
