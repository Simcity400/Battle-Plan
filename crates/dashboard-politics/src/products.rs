pub mod politics;
pub mod feed;
pub mod logs;
pub mod ideas;
pub mod party_history;
pub mod power_ledger;
pub mod country_people;
pub mod stability;
pub mod stability_markers;
pub mod focus;
pub mod national_focus;
pub mod focus_artwork;
pub mod focus_card;
pub mod technology_artwork;
pub mod focus_dto;

dashboard_base::reply_warnings!(
    national_focus::FocusReply,
    technology_artwork::TechnologyArtworkReply,
);
