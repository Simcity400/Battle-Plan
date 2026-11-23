pub mod windows;
pub mod identities;
pub mod groups;
pub mod styles;
pub mod country_flags;
pub mod land_owners;
pub mod subjects;
pub mod territory;
pub mod place_names;
pub mod sea_regions;
pub mod scalars;
pub mod army_windows;
pub mod holder_tags;
pub mod region_names;
pub mod equipment_names;

dashboard_base::reply_warnings!(
    country_flags::CountryFlagReply,
);
