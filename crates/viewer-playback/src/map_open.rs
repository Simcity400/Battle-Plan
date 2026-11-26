use viewer_gpu::{camera::MapSize, textures::IntelCells};
use viewer_layer_core::{engagements, vision};
use viewer_layers_counters::{air_mission_counters, base_counters, counters, gun_counters, land_markers, naval_markers, relations};
use viewer_layers_forces::{aircraft, convoys, intel_icons, orders, ships, units as ground_units};
use viewer_layers_infrastructure::{buildings, intel, radar, rails, snow, sounds, strikes, trains};
use viewer_layers_political::{labels, map_text, mode_fills, names, naval, occupation_marks, political, resource_icons};
use viewer_layer_core::{layers::{Layer, SightInputs, View}, playhead_host::{At, PlaybackProducts, Side, combat::Asking, keep_registered, open, open_private, settings, units, views}};
use crate::{document::Content, overlay::{Ask, Made, Overlay}};
use replay_data::Hour;
use std::{io, sync::Arc};

pub type MapSide = viewer_layer_core::playhead_host::MapSide<Products>;
pub type Opening = viewer_layer_core::playhead_host::Opening<Products>;

pub struct Products;

impl PlaybackProducts for Products {
    type Sight = Content;
    type Combat = Combat;
    type OverlayAsk = Ask;
    type OverlayMade = Made;

    fn sight(content: &Content) -> SightInputs<'_> {
        SightInputs { scene: content, state_provinces: &content.icons.raids.trigger_content.state_provinces }
    }

    fn overlay_viewer(made: &Made) -> u32 {
        made.tables.ours()
    }
}

#[derive(Clone)]
pub struct Combat(pub viewer_combat::Asking);

impl Asking for Combat {
    type Key = viewer_combat::ReadKey;
    type Made = viewer_combat::Product;

    fn key(&self) -> Self::Key {
        self.0.key()
    }

    fn read(&self, branch: replay_data::BranchId, hour: Hour, seek: u64, rows: Arc<replay_frame::frame::DecodedFrame>, engaged: Option<Arc<viewer_engagements::engagement_history::History>>) -> Self::Made {
        self.0.read(branch, hour, seek, rows, engaged)
    }
}

pub fn for_document_at(document: &crate::document::Document, at: Option<At>) -> io::Result<MapSide> {
    ForMap::of(document, at).made(false)
}

pub fn for_document_private(document: &crate::document::Document, at: Option<At>) -> io::Result<MapSide> {
    ForMap::of(document, at).made(true)
}

pub struct ForMap {
    content: Arc<crate::document::Content>,
    replay: Arc<replay::campaign::Replay>,
    branch: replay_data::BranchId,
    hour: Hour,
    at: Option<At>,
    weather: Box<dyn Layer>,
}

impl ForMap {
    pub fn of(document: &crate::document::Document, at: Option<At>) -> Self {
        Self {
            content: Arc::clone(&document.content),
            replay: document.playback.replay_handle(),
            branch: at.map_or_else(|| document.playback.branch(), |at| at.branch),
            hour: at.map_or_else(|| document.playback.hour(), |at| at.hour),
            at,
            weather: crate::layer_sources::weather(document),
        }
    }

    fn made(self, private: bool) -> io::Result<MapSide> {
        let Self { content: document, replay, branch: recording, hour, at, weather } = self;
        let units = units::of(&replay, recording)?;
        let (engaged, branch) = (crate::engaged_index::lanes::shared(&replay, &units, recording, &viewer_foundation::perspective::Country::Recorded)?, recording);
        let entry = if private {
            open_private(units, settings(), Side::Map)?
        } else {
            open(units, settings())?
        };
        entry.set_capture(replay.capture_mode);
        let content = Arc::new(political::Content::new(Arc::clone(&document.map), Arc::clone(&document.border_meshes)));
        let place_in = Arc::clone(&document);
        let map = &document.map;
        let largest = map.definitions.last_key_value().map_or(0, |(&id, _)| id).max(map.province_ids.iter().copied().max().unwrap_or(0));
        let intel = IntelCells::new(map).map(|cells| Arc::new(intel::Content::new(cells, largest))).map_err(|error| error.to_string());
        let snow = document
            .products
            .snow_content()
            .map(|content| Arc::new(snow::Content::new(content.cells.clone(), content.winter.clone(), MapSize::of(&document.map), document.map_defines)))
            .map_err(|error| error.to_string());
        let counters = counters::Content::new(Arc::clone(&document.counters), document.movement.clone(), (MapSize::of(&document.map), document.map_defines), document.tables);
        let marker_index = || engagements::Index { history: Arc::clone(&engaged), branch };
        let (land_index, naval_index) = (marker_index(), marker_index());
        let guns = gun_counters::Content::new(Arc::clone(&document.counters), Arc::clone(&document.map), document.movement.clone());
        let mut layers: Vec<Box<dyn Layer>> = vec![
            Box::new(political::Political::new(content)),
            Box::new(mode_fills::new(Arc::clone(&document) as Arc<dyn mode_fills::Paint>, false)?.in_window(Arc::clone(&entry), views::Recording { branch: recording })?),
            Box::new(vision::Vision::new(Arc::new(vision::Content::new(document.products.fog.as_ref().clone())))),
            Box::new(intel::Intel::new(intel)),
            Box::new(radar::Radar::new()),
            weather,
            Box::new(snow::Snow::new(snow)),
            Box::new(sounds::SoundsFed::new(sounds::Sounds::new(document.audio.as_ref().map(|source| Arc::clone(&source.content))))),
            Box::new(strikes::Strikes::new(Arc::new(move |province| place_in.products.impact_place(province)))),
            Box::new(map_text::MapText::new(Arc::clone(&document) as Arc<dyn map_text::Source>, recording).with_rows(Arc::clone(&entry))),
            Box::new(convoys::Convoys::new(Arc::clone(&document) as Arc<dyn convoys::Routes>).in_window()),
            Box::new({
                let ships = ships::Ships::new(Arc::clone(&document) as Arc<dyn ships::Naval>)?.in_window();
                ships.set_source(Arc::clone(&entry), views::Recording { branch: recording })?;
                ships
            }),
            Box::new(aircraft::new(Arc::clone(&document) as Arc<dyn aircraft::Source>, false)?.in_window(Arc::clone(&entry), views::Recording { branch: recording })?),
            Box::new(
                ground_units::Units::new(Arc::clone(&document) as Arc<dyn ground_units::Ground>)
                    .in_window(Arc::clone(&document) as Arc<dyn ground_units::models::Select>),
            ),
            Box::new({
                let orders = orders::Orders::new(Arc::clone(&document) as Arc<dyn orders::Plans>).in_window();
                orders
            }),
            Box::new(buildings::BuildingsFed::new(buildings::Buildings::reading_vision(Arc::clone(&document) as Arc<dyn buildings::Catalogue>))),
            Box::new(naval::Naval::new(Arc::new(document.naval_dominance.clone()))),
            Box::new(engagements::Engagements::new(Some(engagements::Index { history: engaged, branch }), None).reading_vision()),
            Box::new(intel_icons::Icons::new(Arc::clone(&document) as Arc<dyn intel_icons::Evaluate>, views::Pace::Ready)),
            Box::new(resource_icons::ResourceIcons::new(Arc::clone(&document) as Arc<dyn resource_icons::Source>)),
            Box::new(occupation_marks::OccupationMarks::new(Arc::clone(&document) as Arc<dyn occupation_marks::Source>)),
            Box::new(relations::Relations::new()),
            Box::new(counters::fed::Fed::new(counters::Counters::new(Arc::new(counters)).reading_vision())),
            Box::new(land_markers::LandMarkers::new(Arc::clone(&document) as Arc<dyn land_markers::Front>, Some(land_index))),
            Box::new(naval_markers::NavalMarkers::new(Arc::clone(&document) as Arc<dyn naval_markers::Shore>, Some(naval_index))),
            Box::new(base_counters::BaseCountersFed::new(base_counters::BaseCounters::new())),
            Box::new(gun_counters::GunCounters::new(Arc::new(guns))),
            Box::new(air_mission_counters::AirMissionCounters::new(Arc::new(air_mission_counters::Content::new(&document.counters)))),
            Box::new(rails::Rails::new(Arc::clone(&document) as Arc<dyn rails::Geography>)),
            Box::new(trains::Trains::new(Arc::clone(&document) as Arc<dyn trains::Yard>, None).reading_vision()),
            Box::new(names::Names::new(Arc::clone(&document) as Arc<dyn names::Atlas>).in_window(Arc::clone(&entry), views::Recording { branch: recording })?),
            Box::new(labels::Labels::new(Arc::clone(&document.names.localisation), None).reading_vision()),
            Box::new(crate::layer_sources::scripts(&document)),
        ];
        keep_registered(&mut layers);
        let anchored = entry.anchored();
        let mut side = MapSide::with_products(Arc::clone(&entry), layers, View::default())?;
        side.set_branch(recording);
        side.set_sight(Arc::clone(&document));
        side.set_overlay(Box::new(Overlay::new(Arc::clone(&document))));
        if !anchored {
            let (hour, playing, speed) = at.map_or((hour, false, 0.0), |at| (at.hour, at.playing, at.speed));
            entry.anchor(hour, playing, speed, true);
        }
        Ok(side)
    }
}

pub fn open_document(document: &crate::document::Document, at: Option<At>) -> io::Result<Opening> {
    let made = ForMap::of(document, at);
    Opening::spawn(move || made.made(false))
}

pub fn open_document_private(document: &crate::document::Document, at: Option<At>) -> io::Result<Opening> {
    let made = ForMap::of(document, at);
    Opening::spawn(move || made.made(true))
}
