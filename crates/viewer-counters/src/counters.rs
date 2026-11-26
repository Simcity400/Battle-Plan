use content_reader_mesh::mesh::MeshProducts as _;
use viewer_counters_naval::naval_members::HourJoins;
use viewer_native_gui::{native_gui::Assets, native_windows::NativeWindows};
pub mod carry;
pub mod faction_pings;
mod geography;
pub mod initial_flags;
pub mod layered;
pub mod preparation;
#[cfg(test)]
mod tests;

use crate::counters::faction_pings::HourPins;
use crate::counters::faction_pings::PreparedPins;
use crate::counters::preparation::Preparation;
use content_reader_layers::ResolvedLayers;
use content_reader_key_names::buildings::Slot;
use content_reader_defines::defines::Defines;
use content_reader_gui::flags;
use content_reader_gui::flags::FlagSet;
use content_reader_gui::gui::native;
use content_reader_map::map::MapData;
use content_reader_map::map::ProvinceKind;
use content_reader_map::positions;
use content_reader_gfx::gfx::SpriteSet;
use content_reader_image::dds;
use content_reader_regions::regions;
use dashboard_text::table_access::TokenKeys;
use dashboard_text::table_access::rows as table_access_rows;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;
use viewer_counter_kit::ActionContent;
use viewer_counter_kit::AtSeaPlate;
use viewer_counter_kit::ColumnPlacement;
use viewer_counter_kit::CounterArt;
use viewer_counter_kit::CounterDefines;
use viewer_counter_kit::CounterExit;
use viewer_counter_kit::CounterInputs;
use viewer_counter_kit::CounterMap;
use viewer_counter_kit::CounterPointer;
use viewer_counter_kit::CounterReport;
use viewer_counter_kit::Countries;
use viewer_counter_kit::GUN_WINDOW;
use viewer_counter_kit::Geography;
use viewer_counter_kit::Kinds;
use viewer_counter_kit::LAND_WINDOW;
use viewer_counter_kit::Layouts;
use viewer_counter_kit::Lookups;
use viewer_counter_kit::MEDIUM_FLAG_OVERLAY;
use viewer_counter_kit::Movements;
use viewer_counter_kit::UiColors;
use viewer_counter_kit::assign_union;
use viewer_counter_kit::blend_flag_overlay;
use viewer_counter_kit::combat_markers::Gaps;
use viewer_counter_kit::combat_markers::LandCombatDefines;
use viewer_counter_kit::combat_markers::LandMarkers;
use viewer_counter_kit::combat_markers::land_combats;
use viewer_counter_kit::counter_portraits;
use viewer_counter_kit::gui_windows;
use viewer_counter_kit::invalid;
use viewer_counter_kit::land_windows;
use viewer_counter_kit::region_icon_position;
use viewer_counter_kit::type_tint;
use viewer_counter_kit::units_shown;
use viewer_counter_kit::visible;
use viewer_counters_land::gun_members::GunFields;
use viewer_counters_land::gun_members::GunGround;
use viewer_counters_land::gun_members::TodayGunJoins;
use viewer_counters_land::gun_members::TodayGuns;
use viewer_counters_land::gun_members::gun_member;
use viewer_counters_land::land;
use viewer_counters_land::land::AbilityInputs;
use viewer_counters_land::land::CounterTables;
use viewer_counters_land::land::DivisionCounters;
use viewer_counters_land::land::DivisionStatuses;
use viewer_counters_land::land::GunInputs;
use viewer_counters_land::land::LandUnit;
use viewer_counters_land::land::PreparedLand;
use viewer_counters_land::land::TemplateIconMemo;
use viewer_counters_land::land::TemplateIcons;
use viewer_counters_land::land::division_counters;
use viewer_counters_land::land::hq_portraits;
use viewer_counters_land::land::land_counters;
use viewer_counters_land::members::Member;
use viewer_counters_naval::air_grid;
use viewer_counters_naval::air_missions;
use viewer_counters_naval::air_missions::AirPasses;
use viewer_counters_naval::air_missions::TodayAir;
use viewer_counters_naval::air_volunteers;
use viewer_counters_naval::base_windows;
use viewer_counters_naval::base_windows::View as BaseView;
use viewer_counters_naval::bases;
use viewer_counters_naval::bases::BaseContext;
use viewer_counters_naval::bases::BaseIcons;
use viewer_counters_naval::bases::BaseTables;
use viewer_counters_naval::bases::BaseValues;
use viewer_counters_naval::bases::BuildingIconContent;
use viewer_counters_naval::naval;
use viewer_counters_naval::naval::NavalContext;
use viewer_counters_naval::naval::NavalSources;
use viewer_counters_naval::naval::NavalTables;
use viewer_counters_naval::naval::PortTables;
use viewer_counters_naval::naval::port_counters;
use viewer_counters_naval::naval_markers;
use viewer_counters_naval::naval_markers::MarkerContext;
use viewer_counters_naval::naval_markers::NavalMarkerTables;
use viewer_counters_naval::naval_markers::NavalMarkerValues;
use viewer_counters_naval::naval_markers::Viewer;
use viewer_counters_naval::naval_members::FleetMarker;
use viewer_counters_naval::naval_members::ForceFields;
use viewer_counters_naval::naval_members::SeaGround;
use viewer_counters_naval::naval_members::TodayNaval;
use viewer_counters_naval::naval_members::naval_member;
use viewer_counters_naval::naval_mission_inputs;
use viewer_counters_naval::naval_missions;
use viewer_counters_naval::naval_missions::MissionIcons;
use viewer_counters_naval::naval_regional;
use viewer_counters_naval::naval_windows;
use viewer_engagements::engagement_history::CombatFlags;
use viewer_engagements::engagement_history::History;
use viewer_engagements::engagement_history::LAND_ICON;
use viewer_engagements::engagement_history::NAVAL_ICON;
use viewer_engagements::engagement_history::kept_frames;
use viewer_foundation::allegiance;
use viewer_foundation::allegiance::Allegiance;
use viewer_foundation::building_facts;
use viewer_foundation::building_facts::SiteSlots;
use viewer_foundation::building_facts::SlotAdmission;
use viewer_foundation::country_index::CountryIndex;
use viewer_foundation::mode_visibility::PriorityTable;
use viewer_foundation::mode_visibility::UnitKinds;
use viewer_foundation::mode_visibility::Visibility;
use viewer_foundation::movement_timing::Etas;
use viewer_foundation::movement_timing::MovementContent;
use viewer_foundation::perspective::Vision;
use viewer_foundation::tables::core;
use viewer_foundation::tables::rows;
use viewer_foundation::templates;
use viewer_foundation::unit_rules::STANDSTILL;
use viewer_foundation::visibility::FogContent;
use viewer_foundation::visibility::Relations;
use viewer_gpu::camera::MapSize;
use viewer_gpu::province_admission::province_boxes;
use viewer_gpu::province_map::HeightField;
use viewer_gpu::province_map::ProvinceMap;
use viewer_gpu::stack_anchors::StackAnchors;
use viewer_gpu::terrain_content::TerrainContent;
use viewer_native_gui::icon_windows::PoolView;
use viewer_native_gui::lazy_icons::LazyTextures;
use viewer_native_gui::lazy_icons::flag_set;
use viewer_native_gui::native_resources::GuiResources;
use viewer_native_gui::native_windows::MAPICONS_GUI;
use viewer_native_gui::native_windows::NativeWindows as SharedNativeWindows;
use viewer_native_gui::native_windows::WindowCache;
use viewer_native_gui::sprites::Batch;
use viewer_native_gui::sprites::SpriteSheet;
use viewer_native_gui::sprites::decode_sheet;
use viewer_native_gui::sprites::load_sheet;
use viewer_runtime::memo::Recent;
use viewer_runtime::unit_handle::Unit;
use viewer_unit_reads::counter_action;
use viewer_unit_reads::counter_anchors::stack_anchor;
use viewer_unit_reads::counter_inputs;
use viewer_unit_reads::counter_inputs::NativeInputs;
use viewer_unit_reads::counter_move;
use viewer_unit_reads::counter_order;
use viewer_unit_reads::task_force_visual::TaskForceMotions;

pub fn layers_moved(was: [usize; 6], now: [usize; 6]) -> Vec<&'static str> {
    layered::Layered::moved(was, now)
}

pub type MissionMemo = Recent<(replay_data::BranchId, replay_data::Hour, u32, bool), MissionIcons>;

pub struct CounterAssets(CounterArt);

impl std::ops::Deref for CounterAssets {
    type Target = CounterArt;

    fn deref(&self) -> &CounterArt {
        &self.0
    }
}

impl std::ops::DerefMut for CounterAssets {
    fn deref_mut(&mut self) -> &mut CounterArt {
        &mut self.0
    }
}

impl CounterAssets {
    pub fn load(
        shared: &Arc<ResolvedLayers>,
        (layouts, windows, window_cache): (Layouts, SharedNativeWindows, Arc<WindowCache>),
        defines: &Defines,
        (sprites, registry): (&SpriteSet, &native::resources::Registry),
        (language, map_size, decode): (&str, [u32; 2], content_reader_image::native::Decode),
        images: &mut Vec<dds::Texture>,
        lazy: &Arc<LazyTextures>,
    ) -> io::Result<Self> {
        let layers: &ResolvedLayers = shared;
        let sheet_names: Vec<&str> = layouts.sprite_names();
        let decoded: Vec<io::Result<(dds::Texture, u32, [u32; 2])>> = std::thread::scope(|scope| {
            sheet_names
                .iter()
                .map(|name| {
                    job_priority::spawn_in(scope, || decode_sheet(layers, sprites, name, decode))
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().expect("sheet decoder panicked"))
                .collect()
        });
        let mut sheets = BTreeMap::new();
        let mut warnings: Vec<String> = layouts
            .missing
            .iter()
            .map(|child| format!("{MAPICONS_GUI} declares no {child}; it is not drawn."))
            .chain(
                layouts
                    .unsupported
                    .iter()
                    .map(|reason| format!("{MAPICONS_GUI} {reason}; it is not drawn.")),
            )
            .collect();
        for (name, sheet) in sheet_names.iter().zip(decoded) {
            let (texture, frames, size) = match sheet {
                Ok(sheet) => sheet,
                Err(error) => {
                    warnings.push(format!("{name} is not drawn on map icons: {error}"));
                    continue;
                }
            };
            let id = images.len();
            images.push(texture);
            sheets.insert(
                (*name).to_owned(),
                SpriteSheet {
                    name: (*name).to_owned(),
                    texture: id,
                    frames,
                    size,
                },
            );
        }
        let small_cell = FlagSet::Small.cell(defines)?;
        let medium_cell = FlagSet::Medium.cell(defines)?;
        let overlay_of = |stack: &ResolvedLayers| {
            stack
                .read(MEDIUM_FLAG_OVERLAY)
                .ok()
                .and_then(|bytes| flags::decode(&bytes, medium_cell))
        };
        let overlay = overlay_of(layers).or_else(|| {
            let overlay = overlay_of(&layers.without_overlays()?)?;
            layers.note_overlay_refusal(format!(
                "{MEDIUM_FLAG_OVERLAY} (not a {} x {} Targa)",
                medium_cell[0], medium_cell[1]
            ));
            Some(overlay)
        });
        let overlay_missing = overlay.is_none().then(|| {
            format!(
                "{MEDIUM_FLAG_OVERLAY} is missing or not a {} x {} Targa; medium counter flags are drawn without it.",
                medium_cell[0], medium_cell[1]
            )
        });
        let flags = flag_set(shared, FlagSet::Small, small_cell, |_| {}, lazy)?;
        let medium_flags = flag_set(
            shared,
            FlagSet::Medium,
            medium_cell,
            move |image| {
                if let Some(overlay) = &overlay {
                    blend_flag_overlay(&mut image.pixels, &overlay.pixels);
                }
            },
            lazy,
        )?;
        let large_flags = flag_set(
            shared,
            FlagSet::Normal,
            FlagSet::Normal.cell(defines)?,
            |_| {},
            lazy,
        )?;
        let (types, type_warnings) = templates::TypeAtlas::load(layers, sprites, decode, images)?;
        warnings.extend(type_warnings);
        warnings.extend(overlay_missing);
        let (portraits, portrait_warnings) =
            counter_portraits::load(shared, sprites, decode, lazy)?;
        warnings.extend(portrait_warnings);
        let mut recorded_sheets = BTreeMap::new();
        let format24 = layouts
            .land
            .iter()
            .flat_map(|land| [&land.group_marker, &land.leader_enroute])
            .chain(layouts.gun.iter().map(|gun| &gun.group_marker))
            .map(|child| child.sprite.as_str())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
        for name in [
            counter_order::UNKNOWN_LEADER_PORTRAIT,
            counter_order::ROCKET_SITE_ICON,
        ]
        .into_iter()
        .chain(format24)
        {
            if sheets.contains_key(name) || recorded_sheets.contains_key(name) {
                continue;
            }
            let loaded = if name == counter_order::UNKNOWN_LEADER_PORTRAIT {
                counter_portraits::load_sprite(layers, sprites, name, decode, images)
            } else {
                load_sheet(layers, sprites, name, decode, images)
            };
            match loaded {
                Ok(sheet) => {
                    recorded_sheets.insert(name.to_owned(), sheet);
                }
                Err(error) => {
                    warnings.push(format!("{name} is not drawn on recorded counters: {error}"))
                }
            }
        }
        let ability_sprites = if layouts.abilities.is_some() {
            content_reader_abilities::abilities::read(layers).unwrap_or_else(|error| {
                warnings.push(format!(
                    "common/abilities cannot be read ({error}); the counter ability grid is not drawn"
                ));
                Default::default()
            })
        } else {
            Default::default()
        };
        warnings.extend(
            ability_sprites
                .warnings
                .iter()
                .map(|(_, message)| message.clone()),
        );
        for name in ability_sprites.by_token.values().collect::<BTreeSet<_>>() {
            if sheets.contains_key(name) || recorded_sheets.contains_key(name) {
                continue;
            }
            match load_sheet(layers, sprites, name, decode, images) {
                Ok(sheet) => {
                    recorded_sheets.insert(name.to_owned(), sheet);
                }
                Err(error) => warnings.push(format!(
                    "ability sprite {name} is not drawn on counters: {error}"
                )),
            }
        }
        let gui = GuiResources::new(registry.clone(), language.as_bytes(), map_size, decode);
        let land_action = layouts.land.as_ref().ok().map(|layout| {
            ActionContent::load(
                &layout.action,
                LAND_WINDOW,
                sheets.get(&layout.action.sprite),
            )
        });
        let gun_action = layouts.gun.as_ref().ok().map(|layout| {
            ActionContent::load(
                &layout.action,
                GUN_WINDOW,
                sheets.get(&layout.action.sprite),
            )
        });
        Ok(Self(CounterArt {
            layouts,
            air_sortie_defines: viewer_unit_reads::air_sortie_defines::load_defines(defines)
                .map_err(|e| e.to_string()),
            windows,
            sheets,
            land_action,
            gun_action,
            gui,
            gui_assets: Default::default(),
            flags,
            medium_flags,
            large_flags,
            window_cache,
            types,
            portraits,
            recorded_sheets,
            ability_sprites: Arc::new(ability_sprites),
            warnings,
        }))
    }
}

pub struct CounterContent {
    pub common: CounterMap,
    mission_content: Result<naval_mission_inputs::Content, String>,
    pub building_icons: BuildingIconContent,
    geography: geography::Memo,
    template_icons: TemplateIconMemo,
    volunteer_air_defines: Result<air_volunteers::Defines, String>,
}

impl std::ops::Deref for CounterContent {
    type Target = CounterMap;

    fn deref(&self) -> &CounterMap {
        &self.common
    }
}

impl std::ops::DerefMut for CounterContent {
    fn deref_mut(&mut self) -> &mut CounterMap {
        &mut self.common
    }
}

impl CounterContent {
    pub fn load(
        layers: &ResolvedLayers,
        artwork: &ResolvedLayers,
        map: &MapData,
        defines: &content_reader_defines::defines::Defines,
        regions: &BTreeMap<u32, regions::StrategicRegion>,
    ) -> io::Result<Self> {
        Self::load_with_products(None, layers, artwork, map, defines, regions)
    }

    pub fn load_with_products(
        products: Option<&map_content::loader::Loader>,
        layers: &ResolvedLayers,
        artwork: &ResolvedLayers,
        map: &MapData,
        defines: &content_reader_defines::defines::Defines,
        regions: &BTreeMap<u32, regions::StrategicRegion>,
    ) -> io::Result<Self> {
        let stacks = match products {
            Some(products) => products.anchors(layers)?,
            None => StackAnchors::read(layers)?,
        };
        let anchors = stacks.positions;
        let stack_scales = stacks.scales;
        let mut ports = BTreeMap::new();
        let definitions = content_reader_key_names::buildings::read(layers)?;
        let port_slots = port_slot_names(&definitions);
        let provinces = ProvinceMap {
            width: map.width,
            height: map.height,
            ids: &map.province_ids,
        };
        let building_slots = positions::read_buildings(layers)?;
        let (states, _) = content_reader_states::states::read(layers, defines, &map.definitions)?;
        let roster = artwork.descriptor_roster()?;
        let admit = |spawn: &content_reader_key_names::buildings::SpawnPoint,
                     slot: &content_reader_map::positions::BuildingPosition,
                     province: Option<u32>| {
            viewer_foundation::building_facts::admit_slot(
                spawn,
                slot,
                province,
                map,
                states.definitions(),
                &roster,
            )
        };
        let sites = SiteSlots::build(
            &building_slots,
            &definitions,
            |x, z| provinces.at(x, z),
            |spawn, slot, province| admit(spawn, slot, province) == SlotAdmission::Admitted,
        );
        let site_icons = Arc::new(
            [
                building_facts::AIR_BASE,
                building_facts::ROCKET_SITE,
                building_facts::GUN_EMPLACEMENT,
                building_facts::RADAR,
            ]
            .map(|kind| sites.site_icons(kind).clone()),
        );
        let air_base_first_provinces = Arc::new(sites.first_provinces().clone());
        let building_icons = BuildingIconContent::load(
            &definitions,
            &building_slots,
            &provinces,
            admit,
            BuildingIconContent::offsets(defines)?,
        );
        for building in building_slots {
            if let Some(Slot::Bucket(slot)) = definitions.slot(&building.kind)
                && port_slots.contains(slot.name.as_str())
                && let Some(province) = provinces.at(building.x, building.z)
            {
                ports
                    .entry(province)
                    .or_insert([building.x, building.y, building.z]);
            }
        }
        let mut region_of = BTreeMap::new();
        let mut region_provinces = BTreeMap::new();
        for (&id, region) in regions {
            let mut members: Vec<u32> = Vec::with_capacity(region.provinces.len());
            for &province in &region.provinces {
                region_of.insert(province, id);
                if map.definitions.contains_key(&province) && !members.contains(&province) {
                    members.push(province);
                }
            }
            region_provinces.insert(id, members);
        }
        let (terrain, default_map) = content_reader_key_names::terrain::read(layers)?;
        let wing_types = content_reader_key_names::unit_leader::sub_units(layers)
            .map(Arc::new)
            .map_err(|error| error.to_string());
        let attrition = viewer_unit_reads::counter_attrition::Content::read(
            defines,
            &terrain,
            wing_types.clone(),
        )
        .map(Arc::new)
        .map_err(|error| error.to_string());
        let mission_content =
            naval_mission_inputs::Content::read(terrain.clone(), layers, regions, defines)
                .map_err(|error| error.to_string());
        let naval_mission_allegiance_group_cutoff = mission_content
            .as_ref()
            .map(|content| content.allegiance_group_cutoff)
            .map_err(Clone::clone);
        Ok(CounterContent {
            mission_content,
            building_icons,
            geography: geography::Memo::default(),
            template_icons: TemplateIconMemo::default(),
            volunteer_air_defines: air_volunteers::Defines::read(defines)
                .map_err(|e| e.to_string()),
            common: CounterMap {
                naval_mission_allegiance_group_cutoff,
                wing_types,
                recorded_labels: None,
                port_mission_defines: viewer_unit_reads::port_mission_defines::Defines::read(
                    defines,
                ),
                terrain: Arc::new(TerrainContent {
                    combat_terrain: combat_terrains(
                        &layers.read(&default_map.file("definitions")?)?,
                    )?,
                    terrain_widths: terrain
                        .categories
                        .into_iter()
                        .map(|category| {
                            (
                                category.name,
                                [category.combat_width, category.combat_support_width],
                            )
                        })
                        .collect(),
                }),
                anchors: Arc::new(anchors),
                stack_scales,
                default_anchors: Arc::new(BTreeMap::new()),
                site_icons,
                air_base_first_provinces,
                ports,
                kinds: Arc::new(
                    map.definitions
                        .iter()
                        .map(|(&id, definition)| (id, definition.kind))
                        .collect(),
                ),
                bounds: Arc::new(province_boxes(&provinces)),
                routes: Default::default(),
                region_of: Arc::new(region_of),
                region_provinces,
                region_icons: BTreeMap::new(),
                region_names: regions
                    .iter()
                    .map(|(&id, region)| (id, region.name_key.clone()))
                    .collect(),
                ground_crew_defines: (|| {
                    Ok((
                        defines.fixed5("NAir", "AIR_MORE_GROUND_CREWS_COST")?,
                        defines.fixed5("NAir", "AIR_MORE_GROUND_CREWS_BOOST")?,
                    ))
                })()
                .map_err(|error: io::Error| error.to_string()),
                air_log_timeout: defines
                    .int32("NGame", "AIR_LOG_TIMEOUT_HOURS")
                    .map_err(|error| error.to_string()),
                missing_standstill: Vec::new(),
                map: MapSize::of(map),
                aggression: Arc::new(defines.fixed5_array("NNavy", "AGGRESSION_SETTINGS_VALUES")?),
                defines: CounterDefines::read(defines)?,
                supply_grace: defines
                    .int32("NMilitary", "SUPPLY_GRACE")
                    .map_err(|error| error.to_string()),
                comms_max_distance: defines
                    .int32("NMilitary", "COMMS_MAX_DISTANCE")
                    .map_err(|error| error.to_string()),
                dig_in_factor: defines
                    .fixed5("NMilitary", "DIG_IN_FACTOR")
                    .map_err(|error| error.to_string()),
                attrition,
                ability_comms_scaling: defines
                    .fixed5_array("NMilitary", "ABILITY_COMMS_SCALING")
                    .map(Arc::new)
                    .map_err(|error| error.to_string()),
                ability_no_hq_scaling: defines
                    .fixed5("NMilitary", "ABILITY_NO_HQ_SCALING")
                    .map_err(|error| error.to_string()),
                fog: match products {
                    Some(products) => products.fog(layers, map, defines),
                    None => Arc::new(
                        FogContent::load(layers, map, defines).map_err(|error| error.to_string()),
                    ),
                },
                subunit_tokens: dashboard_text::subunit_files::subunit_keys(layers)
                    .map(|keys| TokenKeys::new(&keys))
                    .map_err(|error| error.to_string()),
                ideologies: content_reader_key_names::ideologies::Ideologies::default(),
            },
        })
    }

    pub fn place_region_icons(
        &mut self,
        centroid: impl Fn(u32) -> Option<[f32; 2]>,
        width: u32,
        heights: &HeightField<'_>,
    ) -> Vec<u32> {
        let mut refused = Vec::new();
        for (&region, provinces) in &self.common.region_provinces {
            let centroids: Option<Vec<[i32; 2]>> = provinces
                .iter()
                .map(|&province| centroid(province).map(|[x, z]| [x as i32, z as i32]))
                .collect();
            let position = centroids.and_then(|centroids| {
                region_icon_position(&centroids, width as i32, |x, z| heights.native_height(x, z))
            });
            match position {
                Some(position) => {
                    self.common.region_icons.insert(region, position);
                }
                None => refused.push(region),
            }
        }
        refused
    }
}

fn combat_terrains(bytes: &[u8]) -> io::Result<BTreeMap<u32, String>> {
    content_reader_map::map::province_terrains(bytes)?
        .into_iter()
        .map(|(id, terrain)| {
            String::from_utf8(terrain)
                .map(|terrain| (id, terrain))
                .map_err(|_| invalid(format!("province {id}'s terrain name is not UTF-8")))
        })
        .collect()
}

fn port_slot_names(definitions: &content_reader_key_names::buildings::Buildings) -> BTreeSet<&str> {
    definitions
        .types
        .values()
        .filter(|building| building.is_port)
        .flat_map(|building| [building.name.as_str(), building.slot_name()])
        .collect()
}

#[derive(Clone)]
pub struct HourTables(viewer_counter_kit::HourTables<HourBase, NavalMarkerTables>);

impl HourTables {
    pub fn country_index(&self) -> &Arc<CountryIndex> {
        &self.0.base.country_index
    }
}

pub struct HourBase {
    frame: Arc<DecodedFrame>,
    engagement_content: EngagementContent,
    relations: Result<Relations, &'static str>,
    country_index: Arc<CountryIndex>,
    #[cfg(test)]
    links: BTreeMap<u32, u32>,
    vision: Arc<Vision>,
    countries: Arc<Countries>,
    geography: Arc<geography::Geometry>,
    native_relations: Arc<allegiance::NativeRelations>,
    peace_conference: bool,
    order: Option<counter_order::RecordedOrder>,
    native: NativeInputs,
    orders: counter_inputs::OrdersIndex,
    icons: Arc<TemplateIcons>,
    status: DivisionStatuses,
    movement: Movements,
    actions: counter_action::Actions,
    pings: HourPins,
    guns: GunInputs,
    abilities: AbilityInputs,
    hq_providers: BTreeSet<u64>,
    portraits: BTreeMap<u64, counter_order::Portrait>,
    icon_inputs: land::IconInputs,
    task_forces: TaskForceMotions,
    divisions: DivisionCounters,
    base_icons: BaseIcons,
    bases: BaseTables,
    naval: NavalTables,
    ports: PortTables,
    ideologies: BTreeMap<u32, Result<Option<i32>, String>>,
    air_missions: AirMissions,
    missions: Arc<MissionIcons>,
    type_tints: Arc<type_tint::Index>,
    left_out: Kinds,
    shown: Shown,
    lookups: Lookups,
}

impl std::ops::Deref for HourTables {
    type Target = HourBase;

    fn deref(&self) -> &HourBase {
        &self.0.base
    }
}

#[cfg(test)]
impl std::ops::DerefMut for HourTables {
    fn deref_mut(&mut self) -> &mut HourBase {
        Arc::get_mut(&mut self.0.base).expect("synthetic base must be uniquely held")
    }
}

impl HourBase {
    fn links(&self) -> &BTreeMap<u32, u32> {
        #[cfg(test)]
        if !self.links.is_empty() {
            return &self.links;
        }
        self.country_index.links()
    }

    fn country_ids(&self) -> &BTreeMap<u32, u32> {
        self.country_index.ids()
    }

    fn land(&self) -> CounterTables<'_> {
        CounterTables {
            type_tints: &self.type_tints,
            countries: &self.countries,
            icons: &self.icons,
            status: &self.status,
            movement: &self.movement,
            actions: &self.actions,
            vision: &self.vision,
            order: self.order.as_ref(),
            native: &self.native,
            orders: &self.orders,
            guns: &self.guns,
            abilities: &self.abilities,
            hq_providers: &self.hq_providers,
            portraits: &self.portraits,
            icon_inputs: &self.icon_inputs,
            task_forces: &self.task_forces,
        }
    }
}

struct EngagementContent {
    land: LandCombatDefines,
    kinds: Arc<BTreeMap<u32, ProvinceKind>>,
    anchors: Arc<BTreeMap<(u32, u32), [f32; 3]>>,
    defaults: Arc<BTreeMap<u32, [f32; 3]>>,
}

impl EngagementContent {
    fn new(content: &CounterContent) -> Self {
        Self {
            land: content.defines.land_combat,
            kinds: Arc::clone(&content.kinds),
            anchors: Arc::clone(&content.anchors),
            defaults: Arc::clone(&content.default_anchors),
        }
    }
}

fn engagement_tables(
    frame: &DecodedFrame,
    vision: &Arc<Vision>,
    links: &BTreeMap<u32, u32>,
    relations: Result<&Relations, &'static str>,
    movement: &Movements,
    content: &EngagementContent,
    index: Option<&History>,
    left_out: Kinds,
    shown: Shown,
) -> io::Result<(Arc<LandMarkers>, Arc<NavalMarkerTables>, Arc<CombatFlags>)> {
    let engaged = CombatFlags::read(frame, vision.ours(), links, relations, index)?;
    let kept =
        |kind, flags| kept_frames(frame, kind, vision.ours(), flags, index).map(|(kept, _)| kept);
    let land_combats = if left_out.has(Kinds::LAND_MARKERS) || !shown.land_combats {
        Arc::default()
    } else {
        let land_kept = kept(LAND_ICON, &engaged.land)?;
        Arc::new(LandMarkers::read(
            frame,
            vision,
            links,
            &(engaged.land.clone(), engaged.land_provenance),
            &land_kept,
            movement,
            &content.land,
        )?)
    };
    let naval_markers = if left_out.has(Kinds::NAVAL_MARKERS) {
        Arc::new(NavalMarkerTables::empty())
    } else {
        let naval_kept = kept(NAVAL_ICON, &engaged.naval)?;
        Arc::new(NavalMarkerTables::read(
            frame,
            Viewer {
                vision,
                owner: &|raw| links.get(&raw).copied(),
            },
            &engaged,
            &naval_kept,
            &|province| content.kinds.get(&province).copied(),
            &|province| stack_anchor(&content.anchors, &content.defaults, province, STANDSTILL),
        )?)
    };
    Ok((land_combats, naval_markers, Arc::new(engaged)))
}

impl HourTables {
    pub fn warm_flags(&self, assets: &CounterArt) {
        self.countries.warm_flags(assets);
    }

    fn land(&self) -> CounterTables<'_> {
        self.0.base.land()
    }

    pub fn is_for_frame(&self, frame: &DecodedFrame) -> bool {
        self.frame.branch == frame.branch
            && self.frame.hour == frame.hour
            && self.frame.metrics == frame.metrics
            && self.frame.tables.iter().zip(&frame.tables).all(|(a, b)| {
                replay_frame::frame::SharedView::ptr_eq(a, b) && a.sampled_at() == b.sampled_at()
            })
    }

    pub fn refine_engagement(&self, index: Option<&History>) -> io::Result<Self> {
        let (land_combats, naval_markers, engaged) = engagement_tables(
            &self.frame,
            &self.vision,
            self.links(),
            self.relations.as_ref().map_err(|reason| *reason),
            &self.movement,
            &self.engagement_content,
            index,
            self.left_out,
            self.shown,
        )?;
        Ok(Self(viewer_counter_kit::HourTables {
            epoch: Arc::new(()),
            base: Arc::clone(&self.0.base),
            land_combats,
            naval_markers,
            engaged,
        }))
    }

    pub fn naval_dominance_sources(
        &self,
    ) -> Arc<viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap> {
        self.missions
            .naval_dominance_sources
            .clone()
            .unwrap_or_else(|| {
                Arc::new(Err("naval dominance source preparation unavailable".into()))
            })
    }
    pub fn vision(&self) -> &Arc<Vision> {
        &self.vision
    }

    pub fn left_out(&self) -> Kinds {
        self.left_out
    }

    pub fn lookups(&self) -> (u64, u64) {
        self.0.base.lookups.counts()
    }

    pub fn routes_by_key(&self) -> bool {
        self.0.base.movement.by_key()
    }

    pub fn fill_routes(&self, content: &CounterContent, movement: Option<&MovementContent>) {
        self.0.base.movement.fill(content, movement);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn land_combat_gaps(&self) -> (&Gaps, &Gaps) {
        (&self.0.land_combats.gaps, &self.0.land_combats.limits)
    }
}

fn peace_conference_hides_units(countries: &[replay_data::rows::CountryRow], ours: u32) -> bool {
    countries
        .iter()
        .any(|country| country.canonical_slot == ours && country.peace_conference_participant != 0)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the hourly inputs come from different owners"
)]
pub fn hour_tables(
    frame: &DecodedFrame,
    map: &MapData,
    vision: Arc<Vision>,
    content: &CounterContent,
    assets: &CounterArt,
    equipment: Result<&content_reader_key_names::equipment::Database, &str>,
    localisation: &content_reader_localisation::localisation::Localisation,
    etas: &Etas,
    movement_content: Option<&MovementContent>,
    engaged: Option<&History>,
    air_passes: Option<&dyn AirPasses>,
    type_tints: &type_tint::Memo,
    mission_memo: &MissionMemo,
    reads: viewer_unit_reads::movement_reads::ReadOperands<'_>,
    mission_topology: Result<
        (
            &content_reader_map::province_graph::ProvinceGraph,
            &content_reader_map::province_geometry::ProvinceGeometry,
        ),
        String,
    >,
) -> io::Result<HourTables> {
    hour_tables_profiled(
        frame,
        map,
        vision,
        content,
        assets,
        equipment,
        localisation,
        etas,
        movement_content,
        engaged,
        air_passes,
        type_tints,
        mission_memo,
        reads,
        mission_topology,
        Kinds::default(),
        Need::WHOLE,
        &Carry::default(),
        None,
        &mut viewer_runtime::profiling::BuilderSpans::new(false),
    )
}

pub struct Carry {
    country_index: viewer_runtime::memo::Settled<CountryIndex>,
    countries: carry::Carried<CountryInputs, u32>,
    pins: carry::Carried<HourPins, u32>,
    abilities: carry::Carried<AbilityInputs, ()>,
    native_relations: carry::Carried<allegiance::NativeRelations, ()>,
}

impl Default for Carry {
    fn default() -> Self {
        Self {
            country_index: CountryIndex::memo(),
            countries: Default::default(),
            pins: Default::default(),
            abilities: Default::default(),
            native_relations: Default::default(),
        }
    }
}

struct CountryInputs {
    index: Arc<CountryIndex>,
    relations: Result<Relations, &'static str>,
    countries: Arc<Countries>,
    peace_conference: bool,
}

fn country_inputs(
    frame: &DecodedFrame,
    map: &MapData,
    ours: u32,
    index_memo: Option<&viewer_runtime::memo::Settled<CountryIndex>>,
) -> io::Result<CountryInputs> {
    let index = match index_memo {
        Some(memo) => CountryIndex::settled(memo, frame)?,
        None => Arc::new(CountryIndex::read(frame)?),
    };
    let country_source = core::<tables::Countries>(frame)?;
    let country_rows: BTreeMap<_, _> = country_source
        .iter()
        .map(|country| (country.canonical_slot, country))
        .collect();
    let relations = Relations::read(frame, index.links());
    let countries = Countries {
        index: Some(Arc::clone(&index)),
        sides: match &relations {
            Ok(relations) => allegiance::counter_table(relations, index.links(), ours),
            Err(_) => BTreeMap::from([(ours, Allegiance::Ours)]),
        },
        groups: ideology_groups(frame, &country_rows)?,
        ui_colors: UiColors::read(map, frame, index.links())?,
        ..Default::default()
    };
    let peace_conference = peace_conference_hides_units(country_source, ours);
    Ok(CountryInputs {
        index,
        relations,
        countries: Arc::new(countries),
        peace_conference,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Need {
    pub air_missions: bool,
    pub by_key: bool,
    pub shown: Shown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub units: bool,
    pub bases: bool,
    pub land_combats: bool,
    pub naval_missions: bool,
}

impl Shown {
    pub const EVERY: Self = Self {
        units: true,
        bases: true,
        land_combats: true,
        naval_missions: true,
    };

    pub fn at(
        height: Option<f32>,
        target: Option<f32>,
        visibility: &Visibility,
        defines: &CounterDefines,
    ) -> Self {
        let Some(height) = height else {
            return Self::EVERY;
        };
        let shows = |cutoff: f32| {
            visible(height, cutoff) || target.is_some_and(|target| visible(target, cutoff))
        };
        Self {
            units: visibility.unit_counters && shows(defines.icons_cutoff),
            bases: visibility.province_icons
                && (shows(defines.air_base_cutoff)
                    || shows(defines.naval_base_cutoff)
                    || shows(defines.radar_cutoff)
                    || shows(defines.facility_cutoff)),
            land_combats: visibility.province_icons && shows(defines.land_combat_cutoff),
            naval_missions: shows(defines.naval_mission_cutoff),
        }
    }
}

#[derive(Clone)]
enum AirMissions {
    Made(Arc<air_missions::Prepared>),
    Deferred(Arc<DeferredAir>),
}

struct DeferredAir {
    frame: Arc<DecodedFrame>,
    vision: Arc<Vision>,
    passes: Option<Arc<dyn AirPasses + Send + Sync>>,
    made: std::sync::OnceLock<Result<Arc<air_missions::Prepared>, String>>,
}

impl AirMissions {
    fn get(&self, inputs: &CounterInputs<'_>) -> io::Result<Arc<air_missions::Prepared>> {
        let deferred = match self {
            Self::Made(made) => return Ok(Arc::clone(made)),
            Self::Deferred(deferred) => deferred,
        };
        if let Some(made) = deferred.made.get() {
            return made.clone().map_err(io::Error::other);
        }
        let hour = air_missions::HourInputs {
            content: inputs.content,
            assets: inputs.assets,
            localisation: inputs.localisation,
            passes: deferred
                .passes
                .as_deref()
                .map(|passes| passes as &dyn AirPasses),
        };
        let made = air_missions::Prepared::read(&hour, &deferred.frame, &deferred.vision)
            .map(Arc::new)
            .map_err(|error| error.to_string());
        let _ = deferred.made.set(made.clone());
        made.map_err(io::Error::other)
    }

    fn made(&self) -> Option<Arc<air_missions::Prepared>> {
        match self {
            Self::Made(made) => Some(Arc::clone(made)),
            Self::Deferred(deferred) => deferred.made.get().and_then(|made| made.clone().ok()),
        }
    }
}

impl Need {
    pub const WHOLE: Self = Self {
        air_missions: true,
        by_key: false,
        shown: Shown::EVERY,
    };

    pub fn routes_by_key(&self, left_out: Kinds) -> bool {
        self.by_key && left_out.has(Kinds::LAND) && left_out.has(Kinds::LAND_MARKERS)
    }
}

pub fn naval_dominance_sources(
    frame: &DecodedFrame,
    map: &MapData,
    content: &CounterContent,
    ours: u32,
    region: u32,
    reads: viewer_unit_reads::movement_reads::ReadOperands<'_>,
    mission_topology: Result<
        (
            &content_reader_map::province_graph::ProvinceGraph,
            &content_reader_map::province_geometry::ProvinceGeometry,
        ),
        String,
    >,
) -> viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap {
    if frame
        .complete::<tables::NavalMissionLedgers>()
        .is_ok_and(|rows| rows.is_empty())
    {
        return Ok(BTreeMap::new());
    }
    let CountryInputs { index, .. } =
        country_inputs(frame, map, ours, None).map_err(|error| error.to_string())?;
    let native = NativeInputs::read(frame).map_err(|error| error.to_string())?;
    let forces = table_access_rows::<tables::TaskForces>(frame)
        .map_err(|error| error.to_string())?
        .rows()
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let world = dashboard_text::triggers::World::new(frame, reads.trigger_content.facts())
        .map_err(|error| error.to_string());
    let cells = viewer_unit_reads::movement_reads::Cells::default();
    let moves = world
        .as_ref()
        .map(|world| viewer_unit_reads::movement_reads::Moves::new(world, reads, &cells))
        .map_err(Clone::clone);
    naval_mission_inputs::Hour::read(
        frame,
        content,
        &content.mission_content,
        &native,
        (index.links(), index.ids()),
        forces,
        (mission_topology, moves),
        ours,
    )?
    .dominance_sources_in(Some(region))
}

pub fn hour_tables_profiled(
    frame: &DecodedFrame,
    map: &MapData,
    vision: Arc<Vision>,
    content: &CounterContent,
    assets: &CounterArt,
    equipment: Result<&content_reader_key_names::equipment::Database, &str>,
    localisation: &content_reader_localisation::localisation::Localisation,
    etas: &Etas,
    movement_content: Option<&MovementContent>,
    engaged: Option<&History>,
    air_passes: Option<&dyn AirPasses>,
    type_tints: &type_tint::Memo,
    mission_memo: &MissionMemo,
    reads: viewer_unit_reads::movement_reads::ReadOperands<'_>,
    mission_topology: Result<
        (
            &content_reader_map::province_graph::ProvinceGraph,
            &content_reader_map::province_geometry::ProvinceGeometry,
        ),
        String,
    >,
    left_out: Kinds,
    need: Need,
    carry: &Carry,
    deferred_passes: Option<Arc<dyn AirPasses + Send + Sync>>,
    spans: &mut viewer_runtime::profiling::BuilderSpans,
) -> io::Result<HourTables> {
    let carry = need.by_key.then_some(carry);
    let ours = vision.ours();
    let divisions = !left_out.has(Kinds::LAND) && need.shown.units;
    let shared = Arc::new(frame.clone());
    let lookups = Lookups::default();
    let CountryInputs {
        index: country_index,
        relations,
        countries,
        peace_conference,
    } = spans.measure("overlay.tables.counters.country_inputs", |_| match carry {
        Some(carry) => carry
            .countries
            .get_or_build(frame, &[], ours, || {
                country_inputs(frame, map, ours, Some(&carry.country_index))
            })
            .map(|inputs| CountryInputs {
                index: Arc::clone(&inputs.index),
                relations: inputs.relations.clone(),
                countries: Arc::clone(&inputs.countries),
                peace_conference: inputs.peace_conference,
            }),
        None => country_inputs(frame, map, ours, None),
    })?;
    let links = country_index.links();
    let country_ids = country_index.ids();
    let geography = spans.measure("overlay.tables.counters.geography", |_| {
        geography::read(frame, content)
    })?;
    let (pings, abilities, guns, movement, actions) =
        spans.measure("overlay.tables.counters.movement_actions", |spans| {
            let pings =
                spans.measure(
                    "overlay.tables.counters.movement_actions.pins",
                    |_| match carry {
                        Some(carry) => carry
                            .pins
                            .get_or_build(
                                frame,
                                &[replay_data::rows::TableId::CountryLinks],
                                ours,
                                || HourPins::read(frame, links, ours, content),
                            )
                            .map(|pins| (*pins).clone()),
                        None => HourPins::read(frame, links, ours, content),
                    },
                )?;
            let abilities = spans.measure(
                "overlay.tables.counters.movement_actions.abilities",
                |_| match carry {
                    _ if !need.shown.units => Ok(AbilityInputs::default()),
                    Some(carry) => carry
                        .abilities
                        .get_or_build(
                            frame,
                            &[replay_data::rows::TableId::CountryLinks],
                            (),
                            || AbilityInputs::read(frame, links),
                        )
                        .map(|abilities| (*abilities).clone()),
                    None => AbilityInputs::read(frame, links),
                },
            )?;
            let hostility = spans
                .measure("overlay.tables.counters.movement_actions.hostility", |_| {
                    counter_action::DestinationHostility::read(frame, links)
                })?;
            let guns = spans.measure("overlay.tables.counters.movement_actions.guns", |_| {
                if left_out.has(Kinds::GUNS) || !need.shown.units {
                    GunInputs::hostility_only(frame, &hostility)
                } else {
                    GunInputs::read(
                        frame,
                        &hostility,
                        content.subunit_tokens.as_ref().map_err(String::as_str),
                    )
                }
            })?;
            let movement =
                spans.measure("overlay.tables.counters.movement_actions.routes", |_| {
                    if need.routes_by_key(left_out)
                        || !(need.shown.units || need.shown.land_combats)
                    {
                        Movements::read_by_key(&shared, lookups.clone())
                    } else {
                        Movements::read(frame, etas, movement_content, content)
                    }
                })?;
            let actions = if divisions {
                spans.measure("overlay.tables.counters.movement_actions.actions", |_| {
                    counter_action::Actions::read(frame, &hostility, Some(vision.ours()))
                })?
            } else {
                counter_action::Actions::default()
            };
            Ok::<_, io::Error>((pings, abilities, guns, movement, actions))
        })?;
    let (engagement_content, land_combats, naval_markers, engaged) =
        spans.measure("overlay.tables.counters.engagement", |_| {
            let engagement_content = EngagementContent::new(content);
            let (land_combats, naval_markers, engaged) = engagement_tables(
                frame,
                &vision,
                links,
                relations.as_ref().map_err(|reason| *reason),
                &movement,
                &engagement_content,
                engaged,
                left_out,
                need.shown,
            )?;
            Ok::<_, io::Error>((engagement_content, land_combats, naval_markers, engaged))
        })?;
    let (native, order, base_icons) =
        spans.measure("overlay.tables.counters.native_base_icons", |spans| {
            let native = spans
                .measure("overlay.tables.counters.native_base_icons.native", |_| {
                    NativeInputs::read(frame)
                })?;
            let order = spans.measure("overlay.tables.counters.native_base_icons.order", |_| {
                counter_order::RecordedOrder::read(frame)
            })?;
            let base_icons = if !need.shown.bases {
                BaseIcons::default()
            } else {
                spans.measure("overlay.tables.counters.native_base_icons.base", |spans| {
                    BaseIcons::read_profiled(
                        frame,
                        &BaseContext {
                            building_icons: &content.building_icons,
                            content,
                            vision: &vision,
                            links,
                            country_ids,
                            relations: relations.as_ref().map_err(|reason| *reason),
                            order: order.as_ref(),
                            carry_tokens: need.by_key,
                        },
                        spans,
                    )
                })?
            };
            Ok::<_, io::Error>((native, order, base_icons))
        })?;
    let icons_shown = need.shown.naval_missions;
    let missions = spans.measure("overlay.tables.counters.missions", |spans| {
        mission_memo.get_or_insert_without_waiting(
            (frame.branch, frame.hour, vision.ours(), icons_shown),
            || {
                spans.measure("overlay.tables.counters.missions.build", |spans| {
                    let (forces, mut icons) =
                        spans.measure("overlay.tables.counters.missions.build.read", |_| {
                            let forces = table_access_rows::<tables::TaskForces>(frame)?
                                .rows()
                                .iter()
                                .map(|row| (row.id, row))
                                .collect();
                            let icons = if icons_shown {
                                MissionIcons::read(
                                    frame,
                                    &vision,
                                    links,
                                    country_ids,
                                    &native,
                                    &forces,
                                )?
                            } else {
                                MissionIcons::default()
                            };
                            Ok::<_, io::Error>((forces, icons))
                        })?;
                    icons.naval_dominance_sources = Some(Arc::new(Ok(BTreeMap::new())));
                    if frame
                        .complete::<tables::NavalMissionLedgers>()
                        .map_or(true, |rows| !rows.is_empty())
                    {
                        let world =
                            spans.measure("overlay.tables.counters.missions.build.world", |_| {
                                dashboard_text::triggers::World::new(
                                    frame,
                                    reads.trigger_content.facts(),
                                )
                                .map_err(|error| error.to_string())
                            });
                        let cells = viewer_unit_reads::movement_reads::Cells::default();
                        let moves = world
                            .as_ref()
                            .map(|world| {
                                viewer_unit_reads::movement_reads::Moves::new(world, reads, &cells)
                            })
                            .map_err(Clone::clone);
                        naval_mission_inputs::decorate_profiled(&mut icons, spans, || {
                            naval_mission_inputs::Hour::read(
                                frame,
                                content,
                                &content.mission_content,
                                &native,
                                (links, country_ids),
                                forces,
                                (mission_topology, moves),
                                vision.ours(),
                            )
                        });
                    }
                    Ok::<_, io::Error>(Arc::new(icons))
                })
            },
        )
    })?;
    let mut tables = spans.measure("overlay.tables.counters.remaining_hour_base", |spans| {
        let hq_providers = if need.shown.units {
            rows::<tables::OrdersGroups>(frame)?
                .rows()
                .iter()
                .filter(|group| group.leader != 0 && group.hq_provider != 0)
                .map(|group| group.hq_provider)
                .collect()
        } else {
            BTreeSet::new()
        };
        let type_tints = spans.measure(
            "overlay.tables.counters.remaining_hour_base.type_tints",
            |_| type_tint::Index::read(type_tints, frame, country_index.links(), vision.ours()),
        );
        let ideologies = spans.measure(
            "overlay.tables.counters.remaining_hour_base.ideologies",
            |_| {
                Ok::<_, io::Error>(
                    rows::<tables::TaskForces>(frame)?
                        .rows()
                        .iter()
                        .filter_map(|force| {
                            country_index.links().get(&force.logical_owner).copied()
                        })
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .map(|slot| {
                            (
                                slot,
                                naval::ideology_frame(
                                    frame,
                                    &content.ideologies,
                                    country_index.ids(),
                                    slot,
                                ),
                            )
                        })
                        .collect(),
                )
            },
        )?;
        let native_relations = spans.measure(
            "overlay.tables.counters.remaining_hour_base.native_relations",
            |_| match carry {
                Some(carry) => carry.native_relations.get_or_build(
                    frame,
                    &[replay_data::rows::TableId::CountryLinks],
                    (),
                    || allegiance::NativeRelations::read(frame, country_index.links()),
                ),
                None => {
                    allegiance::NativeRelations::read(frame, country_index.links()).map(Arc::new)
                }
            },
        )?;
        let tables = HourBase {
            frame: Arc::clone(&shared),
            engagement_content,
            divisions: BTreeMap::new(),
            ports: PortTables::default(),
            naval: spans.measure("overlay.tables.counters.remaining_hour_base.naval", |_| {
                NavalTables::read(frame, &vision)
            })?,
            ideologies,
            bases: spans.measure(
                "overlay.tables.counters.remaining_hour_base.bases",
                |_| match (need.shown.bases, left_out.has(Kinds::BASES)) {
                    (false, _) => Ok(BaseTables::none()),
                    (true, true) => {
                        BaseTables::rest(frame, &vision, &content.volunteer_air_defines)
                    }
                    (true, false) => {
                        BaseTables::read(frame, &vision, &content.volunteer_air_defines)
                    }
                },
            )?,
            air_missions: spans.measure(
                "overlay.tables.counters.remaining_hour_base.air_missions",
                |_| {
                    if left_out.has(Kinds::AIR_MISSIONS) {
                        return Ok(AirMissions::Made(Arc::default()));
                    }
                    if !need.air_missions {
                        return Ok(AirMissions::Deferred(Arc::new(DeferredAir {
                            frame: Arc::clone(&shared),
                            vision: Arc::clone(&vision),
                            passes: deferred_passes.clone(),
                            made: std::sync::OnceLock::new(),
                        })));
                    }
                    air_missions::Prepared::read(
                        &air_missions::HourInputs {
                            content,
                            assets,
                            localisation,
                            passes: air_passes,
                        },
                        frame,
                        &vision,
                    )
                    .map(|made| AirMissions::Made(Arc::new(made)))
                },
            )?,
            base_icons,
            missions,
            portraits: spans.measure(
                "overlay.tables.counters.remaining_hour_base.portraits",
                |_| {
                    if need.shown.units {
                        hq_portraits(frame, &native)
                    } else {
                        Ok(BTreeMap::new())
                    }
                },
            )?,
            hq_providers,
            icon_inputs: spans.measure(
                "overlay.tables.counters.remaining_hour_base.icon_inputs",
                |_| match (need.shown.units, need.by_key) {
                    (false, _) => Ok(land::IconInputs::default()),
                    (true, true) => land::IconInputs::read_by_key(&shared, lookups.clone()),
                    (true, false) => land::IconInputs::read(frame),
                },
            )?,
            native_relations,
            country_index,
            #[cfg(test)]
            links: BTreeMap::new(),
            vision,
            countries,
            geography,
            peace_conference,
            order,
            native,
            orders: spans.measure("overlay.tables.counters.remaining_hour_base.orders", |_| {
                if need.shown.units {
                    counter_inputs::OrdersIndex::read(frame)
                } else {
                    Ok(counter_inputs::OrdersIndex::default())
                }
            })?,
            icons: spans.measure(
                "overlay.tables.counters.remaining_hour_base.template_icons",
                |_| {
                    if need.shown.units {
                        content.template_icons.read(frame)
                    } else {
                        Ok(Arc::default())
                    }
                },
            )?,
            status: spans.measure(
                "overlay.tables.counters.remaining_hour_base.status",
                |_| match (need.shown.units, need.by_key) {
                    (false, _) => Ok(DivisionStatuses::default()),
                    (true, true) => DivisionStatuses::read_by_key(&shared, lookups.clone()),
                    (true, false) => DivisionStatuses::read(frame),
                },
            )?,
            movement,
            actions,
            relations,
            pings,
            guns,
            abilities,
            task_forces: spans.measure(
                "overlay.tables.counters.remaining_hour_base.task_forces",
                |_| TaskForceMotions::read(frame),
            )?,
            type_tints,
            left_out,
            shown: need.shown,
            lookups: lookups.clone(),
        };
        Ok::<_, io::Error>(tables)
    })?;
    if divisions {
        tables.divisions = spans.measure("overlay.tables.counters.divisions", |_| {
            division_counters(
                frame,
                &|raw| tables.links().get(&raw).copied(),
                &tables.land(),
            )
        })?;
    }
    if need.shown.bases {
        tables.ports = spans.measure("overlay.tables.counters.ports", |_| {
            let ideology = |slot: u32| tables.ideologies[&slot].clone();
            let sources = NavalSources {
                type_tints: &tables.type_tints,
                equipment,
                vision: &tables.vision,
                native: &tables.native,
                country_ids: tables.country_ids(),
                aggression: &content.aggression,
                ideology: &ideology,
            };
            PortTables::read(
                frame,
                content,
                map,
                movement_content,
                &tables.land().naval(),
                &NavalContext::cached(frame, sources, &tables.naval)?,
                &|raw| tables.links().get(&raw).copied(),
            )
        })?;
    }
    Ok(HourTables(viewer_counter_kit::HourTables {
        epoch: Arc::new(()),
        base: Arc::new(tables),
        land_combats,
        naval_markers,
        engaged,
    }))
}

#[cfg(test)]
pub fn prepare_cached(
    inputs: &CounterInputs<'_>,
    tables: &HourTables,
) -> io::Result<PreparedCounters> {
    prepare_with(inputs, tables, None)
}

pub fn prepare_retained(
    inputs: &CounterInputs<'_>,
    tables: &HourTables,
    resources: u64,
    retained: &mut Preparation,
) -> io::Result<PreparedCounters> {
    retained.bind(inputs, tables, resources);
    prepare_with(inputs, tables, Some(retained))
}

fn prepare_with(
    inputs: &CounterInputs<'_>,
    tables: &HourTables,
    mut retained: Option<&mut Preparation>,
) -> io::Result<PreparedCounters> {
    let frame: &DecodedFrame = &tables.frame;
    let mut out = CounterFrame::default();
    let owner = |raw: u32| tables.links().get(&raw).copied();
    let layered = retained
        .as_deref()
        .map(|retained| retained.layered().clone())
        .unwrap_or_default();

    out.report.grouping = inputs
        .camera
        .free()
        .is_none()
        .then(|| inputs.content.defines.groups.grouping(inputs.camera.height));
    out.report.type_tint_problem = tables.type_tints.problem().to_owned();
    let grouping_gaps = RefCell::new(BTreeMap::<String, usize>::new());
    let friendly =
        |first: u32, other: u32| match tables.native_relations.co_belligerent(other, first) {
            Ok(friendly) => friendly,
            Err(why) => {
                *grouping_gaps.borrow_mut().entry(why).or_default() += 1;
                false
            }
        };
    let geography = Geography {
        state_of: &tables.geography.state_of,
        extents: &tables.geography.extents,
    };

    let land = tables.land();

    faction_pings::faction_pings(inputs, &tables.pings, &mut out);
    let span = viewer_runtime::profiling::span("pc_bases");
    match retained.as_deref_mut() {
        Some(retained) => retained.bases(inputs, frame, tables, &mut out)?,
        None => bases::bases(inputs, frame, &tables.base_hour(), None, &mut out)?,
    }
    drop(span);
    let span = viewer_runtime::profiling::span("pc_air");
    if inputs.visibility.priorities == PriorityTable::StrategicAir {
        out.native_air = Some(match &layered.air {
            Some(values) => {
                let hour = air_missions::HourInputs {
                    content: inputs.content,
                    assets: inputs.assets,
                    localisation: inputs.localisation,
                    passes: None,
                };
                let make = || {
                    air_missions::Prepared::of_values(values, &hour, frame, tables.vision.ours())
                };
                match retained.as_deref_mut() {
                    Some(retained) => retained.air(values, make),
                    None => Arc::new(make()),
                }
            }
            None => tables.air_missions.get(inputs)?,
        });
    }
    drop(span);
    let ideology = |slot: u32| tables.ideologies[&slot].clone();
    let sources = NavalSources {
        type_tints: &tables.type_tints,
        equipment: inputs.equipment,
        vision: &tables.vision,
        native: &tables.native,
        country_ids: tables.country_ids(),
        aggression: &inputs.content.aggression,
        ideology: &ideology,
    };
    let span = viewer_runtime::profiling::span("pc_naval_context");
    let context = NavalContext::cached(frame, sources, &tables.naval)?;
    let naval = units_shown(inputs, tables.peace_conference).then_some(&context);
    drop(span);
    if let Some(naval) = &naval {
        out.naval.forces = Some(naval_windows::Forces {
            recorded: naval.forces.iter().map(|force| force.id).collect(),
            complete: naval.forces_complete,
        });
    }
    out.naval.countries = Some(Arc::clone(&tables.countries));
    let span = viewer_runtime::profiling::span("pc_land");
    let layered_divisions = layered.land.as_deref().map(land::division_counters_of);
    let prepared_land = match &naval {
        Some(naval) => Some(land_counters(
            inputs,
            layered_divisions.as_ref().unwrap_or(&tables.divisions),
            layered.land.as_deref(),
            layered.guns.as_deref(),
            &owner,
            &land,
            naval,
            &geography,
            &friendly,
            retained
                .as_deref_mut()
                .map(|retained| &mut retained.columns),
            &mut out,
        )?),
        None => None,
    };
    drop(span);
    if inputs.visibility.priorities == PriorityTable::StrategicAir
        && let (Some(land), Some(naval)) = (&prepared_land, &naval)
    {
        let census = air_grid::Census::new(frame);
        for flotilla in &land.flotillas {
            if flotilla.owner == tables.vision.ours() {
                out.naval.carrier_sea.insert(
                    flotilla.source,
                    census.as_ref().map_err(Clone::clone).and_then(|census| {
                        census.carrier(
                            inputs,
                            naval.ship_inputs.as_ref(),
                            &flotilla.members,
                            tables.vision.ours(),
                        )
                    }),
                );
            }
        }
    }
    out.report.grouping_gaps = grouping_gaps.into_inner();
    let span = viewer_runtime::profiling::span("pc_ports");
    if let Some(retained) = retained {
        retained.ports(
            inputs,
            frame,
            &owner,
            &land.naval(),
            naval,
            &tables.base_icons.naval,
            &tables.ports,
            &mut out,
        )?;
    } else {
        port_counters(
            inputs,
            frame,
            &owner,
            &land.naval(),
            naval,
            &tables.base_icons.naval,
            &tables.ports,
            &mut out,
        )?;
    }
    drop(span);
    land_combats(
        inputs,
        layered
            .land_markers
            .as_deref()
            .unwrap_or(&tables.0.land_combats),
        &mut out,
    )?;
    let layered_naval = layered
        .naval_markers
        .as_deref()
        .map(|values| NavalMarkerTables::of_values(values, frame))
        .transpose()?;
    naval_markers::naval_markers(
        inputs,
        frame,
        &MarkerContext {
            vision: &tables.vision,
            owner: &owner,
            naval: Ok(&context),
            countries: &tables.countries,
            flags: &tables.0.engaged,
        },
        layered_naval.as_ref().unwrap_or(&tables.0.naval_markers),
        &mut out,
    )?;
    naval_missions::naval_missions(inputs, &tables.missions, &mut out)?;
    naval_regional::prepare(
        inputs,
        frame,
        &tables.vision,
        &owner,
        Ok(&context),
        &tables.countries,
        &mut out,
    )?;
    out.report.perspective = Arc::clone(&tables.vision);
    out.report.eta = tables.movement.eta.clone();
    Ok(PreparedCounters {
        fixed: out,
        land: prepared_land,
        drawn: DrawnCounters::default(),
        motion_continuity: 0,
    })
}

fn hold_pointer(
    pointer: &mut Option<(Option<(Unit, bool)>, f32)>,
    left: &mut Vec<(Unit, f32)>,
    inputs: &CounterInputs<'_>,
    time: f32,
) -> (Option<CounterPointer>, Vec<CounterExit>) {
    let held = inputs
        .pressed
        .map(|unit| (unit, true))
        .or(inputs.plate_hovered.map(|unit| (unit, false)));
    if pointer.is_none_or(|(last, _)| last != held) {
        if let Some(Some((last, _))) = pointer.map(|(last, _)| last)
            && held.is_none_or(|(unit, _)| unit != last)
        {
            left.retain(|(gone, _)| *gone != last);
            left.push((last, time));
        }
        *pointer = Some((held, time));
    }
    if let Some((unit, _)) = held {
        left.retain(|(gone, _)| *gone != unit);
    }
    let at = pointer.map_or(time, |(_, at)| at);
    let current = held.map(|(unit, down)| CounterPointer {
        unit,
        down,
        since: time - at,
    });
    let exits = left
        .iter()
        .map(|&(unit, at)| CounterExit {
            unit,
            since: time - at,
        })
        .collect();
    (current, exits)
}

pub type CounterFrame = viewer_counter_kit::CounterFrame<
    PreparedPins,
    base_windows::PreparedBases,
    Option<Arc<air_missions::Prepared>>,
    naval_windows::Plan,
>;

pub struct PreparedCounters {
    pub fixed: CounterFrame,
    land: Option<PreparedLand>,
    drawn: DrawnCounters,
    motion_continuity: u64,
}

#[derive(Default)]
struct DrawnCounters {
    batch: Batch,
    combats: Batch,
    report: CounterReport,
    missing_owners: Vec<u32>,
    land_report: CounterReport,
    at_sea: Vec<AtSeaPlate>,
    columns: Vec<Arc<ColumnPlacement>>,
}

impl PreparedCounters {
    pub fn motion_layout(&self) -> Option<&counter_move::DrawnLayout> {
        self.land.as_ref().map(|land| &land.layout)
    }

    pub fn motion_continuity(&self) -> u64 {
        self.motion_continuity
    }

    pub fn set_motion_continuity(&mut self, continuity: u64) {
        self.motion_continuity = continuity;
        if let Some(land) = &self.land {
            land.layout.set_continuity(continuity);
        }
    }

    pub fn flag_source<'a>(
        &'a self,
        assets: &'a CounterArt,
    ) -> Option<viewer_counter_kit::gui_windows::WindowFlags<'a>> {
        self.fixed.naval.countries.as_deref().map(|countries| {
            viewer_counter_kit::gui_windows::WindowFlags {
                countries,
                small: &assets.flags,
                large: &assets.large_flags,
            }
        })
    }

    pub fn detached_flag_source<'a>(
        &self,
        assets: &'a CounterArt,
    ) -> Option<impl viewer_native_gui::native_flags::FlagSource + Send + Sync + 'a + use<'a>> {
        use viewer_native_gui::native_flags::{FlagRecord, FlagSource};
        struct Flags<'a> {
            countries: Arc<Countries>,
            assets: &'a CounterArt,
        }
        impl Flags<'_> {
            fn borrowed(&self) -> viewer_counter_kit::gui_windows::WindowFlags<'_> {
                viewer_counter_kit::gui_windows::WindowFlags {
                    countries: &self.countries,
                    small: &self.assets.flags,
                    large: &self.assets.large_flags,
                }
            }
        }
        impl FlagSource for Flags<'_> {
            fn tag_slot(&self, tag: &[u8]) -> Result<Option<u32>, String> {
                self.borrowed().tag_slot(tag)
            }
            fn first_tag_slot(&self) -> Result<u32, String> {
                self.borrowed().first_tag_slot()
            }
            fn atlas_len(&self, small: bool) -> Result<i32, String> {
                self.borrowed().atlas_len(small)
            }
            fn atlas_record(&self, small: bool, index: i32) -> Result<FlagRecord, String> {
                self.borrowed().atlas_record(small, index)
            }
        }
        self.fixed.naval.countries.as_ref().map(|countries| Flags {
            countries: Arc::clone(countries),
            assets,
        })
    }

    pub fn draw(
        &mut self,
        inputs: &CounterInputs<'_>,
        time: f32,
        naval: &mut naval_windows::Windows,
        pings: &mut PoolView,
        bases: &mut BaseView,
        land_combats: &mut land_windows::View,
    ) -> io::Result<()> {
        let Self {
            fixed,
            land,
            drawn,
            motion_continuity: _,
        } = self;
        let (counter_pointer, counter_exit) = hold_pointer(
            &mut naval.pointer_clock,
            &mut naval.pointer_left,
            inputs,
            time,
        );
        let DrawnCounters {
            batch,
            combats,
            report,
            missing_owners,
            ..
        } = drawn;
        fixed.pings.clear();
        fixed.bases.clear();
        fixed.combats.clear();
        batch.clear();
        combats.clear();
        report.assign_all_but_flags(&fixed.report);
        missing_owners.clear();
        let span = viewer_runtime::profiling::span("dc_pings");
        fixed.native_pings.draw(
            inputs,
            pings,
            time,
            fixed.naval.countries.as_deref(),
            &mut fixed.pings,
            report,
        );
        drop(span);
        let span = viewer_runtime::profiling::span("dc_bases");
        fixed.native_bases.draw(
            inputs,
            bases,
            time,
            fixed.naval.countries.as_ref(),
            &mut fixed.bases,
            report,
        );
        drop(span);
        let mut at_sea = Vec::new();
        let span = viewer_runtime::profiling::span("dc_land_combats");
        fixed.native_land_combats.draw(
            inputs,
            land_combats,
            time,
            fixed.naval.countries.as_deref(),
            &mut fixed.combats,
            report,
        );
        drop(span);
        let span = viewer_runtime::profiling::span("dc_land");
        let columns = match land.as_mut() {
            Some(land) => {
                land.pointer = counter_pointer;
                land.exits = counter_exit;
                land.time = time;
                land.draw(inputs, batch, report, missing_owners, &mut at_sea)?
            }
            None => Vec::new(),
        };
        drop(span);
        {
            let _span = viewer_runtime::profiling::span("dc_naval");
            let plates: Vec<_> = at_sea
                .iter()
                .filter_map(|plate| {
                    let land = land.as_ref()?;
                    Some((
                        &land.flotillas[plate.member],
                        land.countries.as_ref(),
                        plate,
                    ))
                })
                .collect();
            naval_windows::present(
                inputs,
                &fixed.naval,
                &plates,
                &columns,
                time,
                counter_pointer,
                naval,
                &mut fixed.bases,
                batch,
                combats,
                report,
            )?;
            if land
                .as_ref()
                .is_some_and(|land| land.pointer_running.replace(false))
            {
                naval.keep_active();
            }
            let running = land
                .as_ref()
                .map(|land| land.exits_running.take())
                .unwrap_or_default();
            naval
                .pointer_left
                .retain(|(unit, _)| running.contains(unit));
        }
        batch.quads.extend_from_slice(&fixed.counters.quads);
        let span = viewer_runtime::profiling::span("dc_air");
        let empty_air = air_missions::Prepared::default();
        fixed.native_air.as_deref().unwrap_or(&empty_air).draw(
            inputs,
            time,
            &mut naval.air_missions,
            combats,
            report,
        );
        drop(span);
        missing_owners.sort_unstable();
        missing_owners.dedup();
        let tags = land.as_ref().map(|land| land.countries.tags_map());
        assign_union(
            &mut report.missing_flags,
            &fixed.report.missing_flags,
            missing_owners
                .iter()
                .filter_map(|owner| tags.and_then(|tags| tags.get(owner))),
        );
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "separate mutable state for the host's composed icon views"
    )]
    pub fn draw_branched(
        &mut self,
        inputs: &CounterInputs<'_>,
        time: f32,
        owners: viewer_runtime::profiling::FrameOwners,
        naval: &mut naval_windows::Windows,
        pings: &mut PoolView,
        bases: &mut BaseView,
        land_combats: &mut land_windows::View,
    ) -> io::Result<Option<viewer_runtime::profiling::OwnerSample>> {
        let Self {
            fixed,
            land,
            drawn,
            motion_continuity: _,
        } = self;
        let (counter_pointer, counter_exit) = hold_pointer(
            &mut naval.pointer_clock,
            &mut naval.pointer_left,
            inputs,
            time,
        );
        if let Some(land) = land.as_mut() {
            land.pointer = counter_pointer;
            land.exits = counter_exit;
            land.time = time;
        }
        let DrawnCounters {
            batch,
            combats,
            report,
            missing_owners,
            land_report,
            at_sea: drawn_at_sea,
            columns: drawn_columns,
        } = drawn;
        fixed.pings.clear();
        fixed.bases.clear();
        fixed.combats.clear();
        batch.clear();
        combats.clear();
        report.assign_all_but_flags(&fixed.report);
        missing_owners.clear();
        let mut at_sea = Vec::new();
        let countries = fixed.naval.countries.as_deref();
        let held_countries = fixed.naval.countries.as_ref();
        let mut others = {
            let native_pings = &mut fixed.native_pings;
            let ping_batch = &mut fixed.pings;
            let native_bases = &mut fixed.native_bases;
            let base_batch = &mut fixed.bases;
            let native_land_combats = &mut fixed.native_land_combats;
            let combat_batch = &mut fixed.combats;
            let report = &mut *report;
            move || {
                owners.measure("counters_branch", || {
                    let span = viewer_runtime::profiling::span("dc_pings");
                    native_pings.draw(inputs, pings, time, countries, ping_batch, report);
                    drop(span);
                    let span = viewer_runtime::profiling::span("dc_bases");
                    native_bases.draw(inputs, bases, time, held_countries, base_batch, report);
                    drop(span);
                    let _span = viewer_runtime::profiling::span("dc_land_combats");
                    native_land_combats.draw(
                        inputs,
                        land_combats,
                        time,
                        countries,
                        combat_batch,
                        report,
                    );
                })
            }
        };
        let mut land_counters = {
            let land = &mut *land;
            let batch = &mut *batch;
            let land_report = &mut *land_report;
            let missing_owners = &mut *missing_owners;
            let at_sea = &mut at_sea;
            move || {
                let _span = viewer_runtime::profiling::span("dc_land");
                match land.as_mut() {
                    Some(land) => land.draw(inputs, batch, land_report, missing_owners, at_sea),
                    None => Ok(Vec::new()),
                }
            }
        };
        let ((_, sample), columns) = (others(), land_counters());
        report.absorb_later(land_report);
        let columns = columns?;
        drawn_columns.clone_from(&columns);
        {
            let _span = viewer_runtime::profiling::span("dc_naval");
            let plates: Vec<_> = at_sea
                .iter()
                .filter_map(|plate| {
                    let land = land.as_ref()?;
                    Some((
                        &land.flotillas[plate.member],
                        land.countries.as_ref(),
                        plate,
                    ))
                })
                .collect();
            naval_windows::present(
                inputs,
                &fixed.naval,
                &plates,
                &columns,
                time,
                counter_pointer,
                naval,
                &mut fixed.bases,
                batch,
                combats,
                report,
            )?;
            if land
                .as_ref()
                .is_some_and(|land| land.pointer_running.replace(false))
            {
                naval.keep_active();
            }
            let running = land
                .as_ref()
                .map(|land| land.exits_running.take())
                .unwrap_or_default();
            naval
                .pointer_left
                .retain(|(unit, _)| running.contains(unit));
        }
        *drawn_at_sea = at_sea;
        batch.quads.extend_from_slice(&fixed.counters.quads);
        let span = viewer_runtime::profiling::span("dc_air");
        let empty_air = air_missions::Prepared::default();
        fixed.native_air.as_deref().unwrap_or(&empty_air).draw(
            inputs,
            time,
            &mut naval.air_missions,
            combats,
            report,
        );
        drop(span);
        missing_owners.sort_unstable();
        missing_owners.dedup();
        let tags = land.as_ref().map(|land| land.countries.tags_map());
        assign_union(
            &mut report.missing_flags,
            &fixed.report.missing_flags,
            missing_owners
                .iter()
                .filter_map(|owner| tags.and_then(|tags| tags.get(owner))),
        );
        Ok(sample)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "separate mutable state for the host's composed icon views"
    )]
    pub fn draw_quiet(
        &mut self,
        inputs: &CounterInputs<'_>,
        time: f32,
        pools: viewer_native_gui::icon_windows::Pools,
        naval: &mut naval_windows::Windows,
        pings: &mut PoolView,
        bases: &mut BaseView,
        land_combats: &mut land_windows::View,
    ) -> io::Result<()> {
        let Self {
            fixed,
            land,
            drawn,
            motion_continuity: _,
        } = self;
        let (mut scratch, mut report) = (Batch::default(), CounterReport::default());
        let countries = fixed.naval.countries.as_deref();
        if pools.pings {
            fixed
                .native_pings
                .draw(inputs, pings, time, countries, &mut scratch, &mut report);
        }
        if pools.bases {
            fixed.native_bases.draw(
                inputs,
                bases,
                time,
                fixed.naval.countries.as_ref(),
                &mut scratch,
                &mut report,
            );
        }
        if pools.land_combats {
            fixed.native_land_combats.draw(
                inputs,
                land_combats,
                time,
                countries,
                &mut scratch,
                &mut report,
            );
        }
        if !pools.naval {
            return Ok(());
        }
        let plates: Vec<_> = drawn
            .at_sea
            .iter()
            .filter_map(|plate| {
                let land = land.as_ref()?;
                Some((
                    &land.flotillas[plate.member],
                    land.countries.as_ref(),
                    plate,
                ))
            })
            .collect();
        let pointer = land.as_ref().and_then(|land| land.pointer);
        let (mut bases_batch, mut counters_batch, mut combats) =
            (Batch::default(), Batch::default(), Batch::default());
        naval_windows::present(
            inputs,
            &fixed.naval,
            &plates,
            &drawn.columns,
            time,
            pointer,
            naval,
            &mut bases_batch,
            &mut counters_batch,
            &mut combats,
            &mut report,
        )?;
        let empty_air = air_missions::Prepared::default();
        fixed.native_air.as_deref().unwrap_or(&empty_air).draw(
            inputs,
            time,
            &mut naval.air_missions,
            &mut combats,
            &mut report,
        );
        Ok(())
    }

    pub fn keep_drawn(&mut self, old: &mut PreparedCounters) {
        self.set_motion_continuity(old.motion_continuity);
        if let (Some(land), Some(old)) = (self.land.as_mut(), old.land.as_mut()) {
            land.keep_drawn(old);
        }
    }

    pub fn keep_drawn_by_value(&mut self, old: &mut PreparedCounters) {
        if let (Some(land), Some(old)) = (self.land.as_mut(), old.land.as_mut()) {
            land.keep_drawn_by_value(old);
        }
    }

    pub fn drawn(&self) -> (&Batch, &CounterReport) {
        (&self.drawn.batch, &self.drawn.report)
    }

    pub fn drawn_combats(&self) -> &Batch {
        &self.drawn.combats
    }
}

fn ideology_groups(
    frame: &DecodedFrame,
    countries: &BTreeMap<u32, &replay_data::rows::CountryRow>,
) -> io::Result<BTreeMap<u32, Result<Option<String>, String>>> {
    match dashboard_text::flag_choices::ruling_groups(frame)? {
        dashboard_text::flag_choices::RulingGroups::Unavailable(why) => Ok(countries
            .keys()
            .map(|&slot| (slot, Err(why.clone())))
            .collect()),
        dashboard_text::flag_choices::RulingGroups::Available { groups, incomplete } => Ok(countries
            .iter()
            .map(|(&slot, country)| {
                let group = groups.get(&country.id).cloned().unwrap_or_else(|| {
                    Err(incomplete.clone().unwrap_or_else(|| {
                        format!("country {} flag needs its ruling group", country.id)
                    }))
                });
                (slot, group)
            })
            .collect()),
    }
}

impl HourTables {
    pub fn land_markers(&self) -> &LandMarkers {
        &self.0.land_combats
    }
}

impl HourTables {
    pub fn type_tints(&self) -> &Arc<type_tint::Index> {
        &self.type_tints
    }

    pub fn division_action(&self, id: u64) -> counter_action::Action {
        self.actions.get(id)
    }

    pub fn division_members(&self) -> Vec<((u32, u32), Vec<Member>)> {
        self.divisions
            .iter()
            .map(|(&key, units)| {
                let mut members: Vec<Member> = units.iter().map(LandUnit::member).collect();
                members.sort_by_key(|member| member.ordinal);
                (key, members)
            })
            .collect()
    }
}

impl HourTables {
    pub fn gun_members(&self, ground: GunGround<'_>) -> TodayGuns {
        let land = self.land();
        let owner = |raw: u32| self.links().get(&raw).copied();
        let joins = TodayGunJoins {
            land: &land,
            owner: &owner,
        };
        let guns = land
            .guns
            .rows
            .iter()
            .flatten()
            .map(|row| (row.gun, gun_member(&GunFields::of(row), ground, &joins)))
            .collect();
        TodayGuns {
            guns,
            complete: land.guns.rows.is_some(),
            recorded: land.guns.recorded,
            refused: land.guns.refused.clone(),
        }
    }
}

impl HourTables {
    pub fn gun_hostility(&self) -> BTreeMap<u64, Result<bool, &'static str>> {
        self.guns.destination_enemy.clone()
    }
}

impl HourTables {
    pub fn naval_members(
        &self,
        ground: SeaGround<'_>,
        kinds: UnitKinds,
        equipment: Result<&content_reader_key_names::equipment::Database, &str>,
    ) -> io::Result<TodayNaval> {
        let frame = &self.frame;
        let ideology = |slot: u32| self.ideologies.get(&slot).cloned().unwrap_or(Ok(None));
        let sources = NavalSources {
            type_tints: &self.type_tints,
            equipment,
            vision: &self.vision,
            native: &self.native,
            country_ids: self.country_ids(),
            aggression: &[],
            ideology: &ideology,
        };
        let context = NavalContext::cached(frame, sources, &self.naval)?;
        let land = self.land().naval();
        let bases: BTreeSet<u32> = rows::<tables::NavalBases>(frame)?
            .rows()
            .iter()
            .map(|base| base.province)
            .collect();
        let owner = |raw: u32| self.links().get(&raw).copied();
        let members = context
            .forces
            .iter()
            .map(|force| {
                naval_member(
                    ForceFields::of(force),
                    kinds,
                    &HourJoins {
                        land: &land,
                        context: &context,
                        owner: &owner,
                        ground,
                        row: force,
                        bases: &bases,
                    },
                )
            })
            .collect();
        let fleets = rows::<tables::FleetCounterInputs>(frame)?
            .rows()
            .iter()
            .map(|row| {
                (
                    row.fleet,
                    FleetMarker {
                        reserve: context.reserve(row.fleet),
                        colour: [row.red, row.green, row.blue, row.alpha],
                    },
                )
            })
            .collect();
        let ports = self.ports.summary();
        Ok(TodayNaval {
            members,
            fleets,
            ports,
            peace_conference: self.peace_conference,
            forces_complete: context.forces_complete,
        })
    }
}

impl HourTables {
    pub fn naval_marker_values(&self) -> NavalMarkerValues {
        self.0.naval_markers.values()
    }
}

impl HourTables {
    pub fn base_values(&self) -> BaseValues {
        self.bases.values().clone()
    }
}

impl HourTables {
    pub fn air_values(&self) -> TodayAir {
        let Some(air) = self.air_missions.made() else {
            return TodayAir {
                regions: BTreeMap::new(),
                alerts: BTreeSet::new(),
                uncertain: BTreeMap::new(),
                refused: BTreeMap::from([(
                    "the hour's air missions were deferred and no layout in the air mode read them"
                        .to_owned(),
                    1,
                )]),
                supply: BTreeMap::new(),
                supply_complete: false,
                supply_unrecorded: false,
                ground: BTreeMap::new(),
            };
        };
        air.today()
    }
}
impl HourTables {
    fn base_hour(&self) -> bases::BaseHour<'_> {
        bases::BaseHour {
            vision: &self.vision,
            bases: &self.bases,
            base_icons: &self.base_icons,
        }
    }
}

pub fn gui_assets(
    windows: &NativeWindows,
    registry: &native::resources::Registry,
    localisation: &content_reader_localisation::localisation::Localisation,
    tooltip_assets: Assets,
) -> Assets {
    let mut assets = gui_windows::window_assets(windows, registry, localisation);
    assets.extend(naval_missions::mission_assets(
        windows,
        registry,
        localisation,
    ));
    assets.extend(tooltip_assets);
    assets
}

impl CounterAssets {
    pub fn defer_window_art(
        &mut self,
        layers: &ResolvedLayers,
        localisation: &content_reader_localisation::localisation::Localisation,
        tooltip_assets: Assets,
        textures: &Arc<LazyTextures>,
    ) {
        let assets = gui_assets(
            &self.windows,
            self.gui.registry(),
            localisation,
            tooltip_assets,
        );
        self.gui.defer_window_art(layers, &assets, textures);
        self.gui_assets = assets;
    }
}
