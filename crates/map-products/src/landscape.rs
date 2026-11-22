use crate::{
    ProductHour,
    dependency_index::{Clock, DependencyIndex},
    witness::{KeyText, Operand},
};
use replay_data::rows::tables;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex},
};
use viewer_runtime::{name_draw::Placements, profiling::BuilderSpans};
use viewer_foundation::{perspective, tables::core};
use viewer_borders::demilitarized::{self, Zone};
use viewer_map_icons::labels;
use viewer_buildings::map_buildings::{self, BuildingsBase, MapBuildingSummary};
use viewer_map_modes::{map_mode::{self, MapMode}, naval_dominance, political};
use map_display_data::landscape::{ColourState, ModeFill};
use viewer_models::ModelField;
use viewer_map_text::name_pass::{self, NameAreas, NameContent, PlacedAreas, PlacementInputs};
use viewer_rail::railways::{self, Bridges, RailGeometry, RailNetwork};

#[derive(Default)]
pub(crate) struct Projections {
    countries: crate::country_index::CountryIndex,
    territory: DependencyIndex<[u8; 32], map_display_data::landscape::Territory>,
    territory_fold: Mutex<Option<([u8; 32], political::TerritoryFold)>>,
    zones: crate::zone_projection::Zones,
    rail_products: Mutex<Option<((u64, u64), Arc<RailProduct>)>>,
    modes:
        DependencyIndex<(u64, MapMode, u32, [u8; 32]), map_display_data::landscape::ModeProjection>,
    label_facts: DependencyIndex<[u8; 32], map_display_data::landscape::LabelFacts>,
    labels: DependencyIndex<(u32, [u8; 32]), map_display_data::landscape::Labels>,
    rails: DependencyIndex<(), RailNetwork>,
    buildings: DependencyIndex<BuildingKey, BuildingProduct>,
    building_bases: std::sync::Mutex<
        Option<(
            BuildingKey,
            replay_frame::frame::ViewIdentity,
            BuildingsBase,
        )>,
    >,
    name_areas: DependencyIndex<NameAreaKey, NameAreas>,
    names: Mutex<Option<NameProjection>>,
}

fn default_mode_projection() -> Arc<map_display_data::landscape::ModeProjection> {
    static VALUE: std::sync::OnceLock<Arc<map_display_data::landscape::ModeProjection>> =
        std::sync::OnceLock::new();
    Arc::clone(VALUE.get_or_init(|| {
        Arc::new(map_display_data::landscape::ModeProjection::Stock {
            fill: ModeFill::default(),
        })
    }))
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BuildingKey {
    assets: u64,
    perspective: u32,
    fog: bool,
    content: [u8; 32],
    city: Operand<Vec<bool>>,
    fog_bytes: Option<Operand<perspective::values::Values<u32, u8>>>,
    visibility_gap: Option<&'static str>,
}
impl BuildingKey {
    fn same_base(&self, other: &Self) -> bool {
        self.assets == other.assets
            && self.perspective == other.perspective
            && self.fog == other.fog
            && self.content == other.content
            && self.city == other.city
    }
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NameAreaKey {
    modes: Option<u64>,
    layers: (u8, u8),
    witness: [u8; 32],
    localisation: Option<Operand<content_reader_localisation::localisation::Localisation>>,
    regions: Option<Operand<BTreeMap<u32, content_reader_regions::regions::StrategicRegion>>>,
    states: Option<(
        Operand<content_reader_localisation::localisation::Localisation>,
        Operand<BTreeMap<u32, Result<String, String>>>,
        KeyText,
    )>,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NameKey {
    areas: NameAreaKey,
    names: u64,
    placement: u64,
    font: u64,
    heightmap: Operand<content_reader_image::IndexedImage>,
    witness: [u8; 32],
}
struct NameProjection {
    key: NameKey,
    source: replay_frame::frame::ViewIdentity,
    areas: Arc<NameAreas>,
    placements: Arc<Placements>,
    placed: PlacedAreas,
}

pub struct PoliticalProduct {
    pub layers: political::PoliticalLayers,
    pub colour: Option<Arc<ColourState>>,
    pub zone: map_display_data::retained::Retained<Zone>,
    pub projection: Arc<map_display_data::landscape::ModeProjection>,
}
#[derive(Debug, Default, PartialEq)]
pub struct RailProduct(pub railways::RailMesh, pub ModelField, pub Arc<RailNetwork>);
pub struct BuildingProduct {
    pub prepared: Arc<map_display_data::buildings::Buildings>,
    pub summary: MapBuildingSummary,
}

fn diplomatic_selection(mode: &MapMode, ours: u32, selected: Option<u32>) -> u32 {
    if matches!(mode, MapMode::Stock(map_mode::Stock::Diplomacy)) {
        selected.unwrap_or(ours)
    } else {
        ours
    }
}

pub struct NameInputs<'a> {
    pub content: &'a map_mode::Content,
    pub names: &'a NameContent,
    pub regions: &'a Arc<BTreeMap<u32, content_reader_regions::regions::StrategicRegion>>,
    pub heightmap: &'a Arc<content_reader_image::IndexedImage>,
    pub counter: &'a viewer_counter_products::map::counter_content::Content,
    pub placement: PlacementInputs<'a>,
}

impl ProductHour<'_> {
    pub fn labels(&self, ours: u32) -> io::Result<Arc<map_display_data::landscape::Labels>> {
        self.content.landscape.labels.project_fields(
            self.frame,
            (ours, self.content.reads.binding(&["localisation"])?),
            Clock::Relations,
            label_fields,
            || {
                let links = core::<tables::CountryLinks>(self.frame)?
                    .iter()
                    .map(|row| (row.id, row.canonical_slot))
                    .collect();
                core::<tables::Countries>(self.frame)?;
                let allegiances =
                    viewer_foundation::allegiance::war_table(self.frame, &links, ours)?;
                let facts = self.content.landscape.label_facts.project_fields(
                    self.frame,
                    self.content.reads.binding(&["localisation"])?,
                    Clock::Stable,
                    label_fields,
                    || labels::LabelTables::facts(self.frame, &self.content.localisation),
                )?;
                Ok(map_display_data::landscape::Labels {
                    perspective: facts.tags.get(&ours).cloned(),
                    candidates: Arc::clone(&facts.candidates),
                    allegiances: allegiances
                        .into_iter()
                        .map(|(slot, side)| (slot, side.index()))
                        .collect(),
                })
            },
        )
    }

    pub fn mode_projection(
        &self,
        content: &map_mode::Content,
        mode: &MapMode,
        ours: u32,
    ) -> io::Result<Arc<map_display_data::landscape::ModeProjection>> {
        let _scope = viewer_runtime::profiling::thread_span("modes");
        let _cost = replay_frame::cpu::scope("viewer", Some("modes/styles"));
        if *mode == MapMode::default() {
            return Ok(default_mode_projection());
        }
        let clock = match mode {
            MapMode::Scripted(_) => Clock::Hour,
            _ => Clock::Relations,
        };
        self.content.landscape.modes.project_fields(
            self.frame,
            (
                content.projection_id(),
                mode.clone(),
                ours,
                self.content.reads.dependent_binding(
                    "modes",
                    &[
                        "map",
                        "defines",
                        "trigger",
                        "display/modes",
                        "session-error",
                    ],
                )?,
            ),
            clock,
            |table| {
                if matches!(mode, MapMode::Scripted(_)) {
                    table.full_mask()
                } else {
                    mode_fields(table)
                }
            },
            || {
                content.bind_membership_map(&self.content.map);
                let projection = map_mode::resolved::project_lazy(
                    mode,
                    &map_mode::Inputs {
                        content,
                        map: &self.content.map,
                        frame: self.frame,
                        selected: Some(ours),
                        world: None,
                        colour: None,
                    },
                    &|| self.trigger_world(),
                )?;
                Ok(projection)
            },
        )
    }

    pub fn country_styles(
        &self,
        content: &map_mode::Content,
        mode: &MapMode,
        native: u32,
        slot: u32,
    ) -> io::Result<map_display_data::landscape::CountryStyles> {
        let (projection, reads) = replay_frame::frame::view_reads::record(|| {
            if !self
                .content
                .landscape
                .countries
                .contains(self.frame, native, slot)?
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "country instance is not present this hour",
                ));
            }
            self.mode_projection(content, mode, slot)
        });
        let observed: Vec<_> = replay_data::rows::TableId::ALL
            .iter()
            .copied()
            .filter(|table| {
                reads
                    .iter()
                    .any(|read| read.is(&self.frame.tables[table.index()]))
            })
            .collect();
        Ok(map_display_data::landscape::CountryStyles {
            country: map_display_data::identities::CountryKey {
                branch: self.frame.branch,
                native,
                canonical_slot: slot,
                observed_hour: self.frame.hour,
                continuity: "observation only; country lifetime continuity is unknown".into(),
            },
            mode: mode.token(),
            projection: projection?,
            evidence: crate::display::evidence(self.frame, &observed),
        })
    }
    pub fn visit_country_styles(
        &self,
        content: &map_mode::Content,
        mode: &MapMode,
        mut visit: impl FnMut(map_display_data::landscape::CountryStyles) -> io::Result<()>,
    ) -> io::Result<()> {
        for country in core::<tables::Countries>(self.frame)? {
            visit(self.country_styles(content, mode, country.id, country.canonical_slot)?)?;
        }
        Ok(())
    }

    pub fn mode(
        &self,
        content: &map_mode::Content,
        mode: &MapMode,
        choice: &perspective::Choice,
        selected: Option<u32>,
        previous: Option<&ColourState>,
    ) -> io::Result<ModeFill> {
        if *mode == MapMode::default() {
            return Ok(ModeFill::default());
        }
        let ours = self.vision(choice)?.ours();
        let selected = diplomatic_selection(mode, ours, selected);
        map_display_data::landscape::apply_mode(
            self.mode_projection(content, mode, selected)?.as_ref(),
            previous,
        )
    }

    pub fn political(
        &self,
        content: &map_mode::Content,
        navy: &naval_dominance::Content,
        mode: &MapMode,
        choice: &perspective::Choice,
        selected: Option<u32>,
        previous: Option<&ColourState>,
    ) -> io::Result<PoliticalProduct> {
        let _cost = replay_frame::cpu::scope("territory operands", Some("territory"));
        let started = std::time::Instant::now();
        let ours = self.vision(choice)?.ours();
        let selected = diplomatic_selection(mode, ours, selected);
        let projection = self.mode_projection(content, mode, selected)?;
        let fill = map_display_data::landscape::apply_mode(&projection, previous)?;
        let colour = fill.colour.clone();
        let territory = self.territory()?;
        let naval = {
            let _cost = replay_frame::cpu::scope("viewer", Some("air/naval source choices"));
            navy.read(self.frame, ours)
        };
        let mut regions = (*territory.paint).clone();
        regions.strategic_attributes = naval
            .attributes
            .into_iter()
            .map(|(id, attributes)| {
                id.checked_add(1)
                    .map(|key| (key, attributes))
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "strategic region id exceeds the region key range",
                        )
                    })
            })
            .collect::<io::Result<_>>()?;
        regions.warnings.extend(naval.warnings);
        regions.warnings.extend(fill.warnings);
        regions.mode = fill.draw.map(Arc::new);
        let zone = self
            .content
            .landscape
            .zones
            .read(self.frame, &self.content.map, &regions)?;
        Ok(PoliticalProduct {
            layers: political::PoliticalLayers {
                regions: Arc::new(regions),
                build: started.elapsed(),
            },
            colour,
            zone,
            projection,
        })
    }

    pub fn territory(&self) -> io::Result<Arc<map_display_data::landscape::Territory>> {
        let _scope = viewer_runtime::profiling::thread_span("territory");
        let _cost = replay_frame::cpu::scope("territory rows", Some("territory"));
        let binding = self
            .content
            .reads
            .dependent_binding("territory", &["map", "session-error"])?;
        self.content.landscape.territory.project_fields(
            self.frame,
            binding,
            Clock::Hour,
            territory_fields,
            || {
                let mut fold = self
                    .content
                    .landscape
                    .territory_fold
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if fold.as_ref().is_none_or(|(key, _)| *key != binding) {
                    *fold = Some((binding, political::TerritoryFold::default()));
                }
                fold.as_mut()
                    .unwrap()
                    .1
                    .read_input_shared(&self.content.map, self.changes.input())
                    .map(|value| value.as_ref().clone())
            },
        )
    }

    pub fn territory_projection(
        &self,
    ) -> io::Result<map_display_data::landscape::TerritoryProjection> {
        let (facts, reads) = replay_frame::frame::view_reads::record(|| self.territory());
        let observed: Vec<_> = replay_data::rows::TableId::ALL
            .iter()
            .copied()
            .filter(|table| {
                reads
                    .iter()
                    .any(|read| read.is(&self.frame.tables[table.index()]))
            })
            .collect();
        Ok(map_display_data::landscape::TerritoryProjection {
            facts: facts?,
            evidence: crate::display::evidence(self.frame, &observed),
        })
    }
    pub fn rail_projection(&self) -> io::Result<map_display_data::landscape::RailProjection> {
        Ok(map_display_data::landscape::RailProjection {
            graph: self.rail_graph()?.shared_display(),
            evidence: crate::display::evidence(
                self.frame,
                &[
                    replay_data::rows::TableId::RailEdges,
                    replay_data::rows::TableId::ProvinceSupply,
                    replay_data::rows::TableId::NavalBases,
                ],
            ),
        })
    }
    pub fn rail_graph(&self) -> io::Result<Arc<RailNetwork>> {
        self.content.landscape.rails.project_equal_fields(
            self.frame,
            (),
            Clock::Stable,
            rail_fields,
            || RailNetwork::read(self.frame),
        )
    }
    pub fn rails(
        &self,
        geometry: &RailGeometry,
        bridges: &Bridges,
    ) -> io::Result<Arc<RailProduct>> {
        let graph = self.rail_graph()?;
        let key = (geometry.projection_id(), bridges.projection_id());
        let mut products = self
            .content
            .landscape
            .rail_products
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((old, product)) = &*products {
            if *old == key && Arc::ptr_eq(&product.2, &graph) {
                return Ok(Arc::clone(product));
            }
        }
        let mesh = geometry.build(&graph)?;
        let field = bridges.field(&mesh);
        let product = Arc::new(RailProduct(mesh, field, graph));
        *products = Some((key, Arc::clone(&product)));
        Ok(product)
    }
    pub fn zone(&self, regions: &map_display_data::landscape::Regions, extra: &BTreeSet<u32>) -> io::Result<Zone> {
        let mut states = demilitarized::recorded_states(self.frame)?;
        states.extend(extra.iter().copied());
        Ok(Zone::new(&self.content.map, regions, &states))
    }
    pub fn buildings(
        &self,
        assets: &map_buildings::MapBuildingAssets,
        choice: &perspective::Choice,
        city: &Arc<Vec<bool>>,
        spans: &mut BuilderSpans,
    ) -> io::Result<(map_display_data::buildings::Buildings, MapBuildingSummary)> {
        let _scope = viewer_runtime::profiling::thread_span("buildings");
        let _cost = replay_frame::cpu::scope("viewer", Some("buildings"));
        let vision = self.vision(choice)?;
        let binding = self.content.reads.dependent_binding(
            "buildings",
            &[
                "map",
                "defines",
                "trigger",
                "heightmap",
                "display/buildings",
                "session-error",
            ],
        )?;
        let key = BuildingKey {
            assets: assets.projection_id(),
            perspective: vision.ours(),
            fog: vision.fog(),
            content: binding,
            city: Operand::new(city),
            fog_bytes: vision.shared_province_fog().map(Operand::new),
            visibility_gap: vision.visibility_gap(),
        };
        let terrain = map_buildings::CityTerrain {
            width: self.content.map.width,
            height: self.content.map.height,
            spawn_city: city,
        };
        let result = self.content.landscape.buildings.project_fields(
            self.frame,
            key.clone(),
            Clock::Stable,
            building_fields,
            || {
                let mut bases = self
                    .content
                    .landscape
                    .building_bases
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let previous = bases
                    .as_ref()
                    .filter(|(old, source, _)| {
                        old.same_base(&key)
                            && source
                                .is(&self.frame.tables
                                    [replay_data::rows::TableId::Campaign.index()])
                    })
                    .map(|(_, _, base)| base);
                let (prepared, summary, mut base) = assets.field_patched_input(
                    self.changes.input(), &vision, &terrain, city, previous, spans,
                )?;
                let prepared = Arc::new(assets.display(
                    &prepared,
                    self.content.display_binding()?,
                    vision.ours(),
                    vision.fog(),
                    summary.warnings(),
                    previous,
                )?);
                base.set_display(Arc::clone(&prepared));
                *bases = Some((
                    key,
                    self.frame.tables[replay_data::rows::TableId::Campaign.index()].identity(),
                    base,
                ));
                Ok(BuildingProduct { prepared, summary })
            },
        );
        let value = result?;
        Ok((
            map_display_data::buildings::Buildings {
                branch: self.frame.branch,
                hour: self.frame.hour,
                ..value.prepared.as_ref().clone()
            },
            value.summary.clone(),
        ))
    }

    pub fn name_areas(&self, inputs: &NameInputs<'_>, mode: &MapMode) -> io::Result<NameAreas> {
        let layers = inputs.content.name_layers(mode);
        let far = self.layer_areas(inputs, layers.far)?;
        Ok(match layers.near {
            map_mode::NameLayer::Nothing => far,
            near => far.with_near(self.layer_areas(inputs, near)?),
        })
    }
    fn layer_areas(
        &self,
        inputs: &NameInputs<'_>,
        layer: map_mode::NameLayer,
    ) -> io::Result<NameAreas> {
        use map_mode::NameLayer;
        let groups = match layer {
            NameLayer::Nothing => return Ok(NameAreas::default()),
            NameLayer::StrategicRegions => {
                return Ok(NameAreas::strategic(
                    &self.content.map,
                    inputs.regions,
                    |region| {
                        viewer_foundation::tooltip_text::localise(
                            &inputs.names.localisation,
                            &region.name_key,
                            &[],
                        )
                    },
                ));
            }
            NameLayer::States => {
                return NameAreas::states(&self.content.map, self.frame, |state| {
                    viewer_foundation::tooltip_text::state_name(
                        &inputs.counter.localisation,
                        &inputs.counter.recorded_language,
                        state.id,
                        &state.custom_name,
                        &state.owner_name,
                        || {
                            let key = inputs
                                .counter
                                .state_name_keys
                                .get(&state.id)
                                .ok_or("state content name absent")?
                                .as_ref()
                                .map_err(Clone::clone)?;
                            viewer_foundation::tooltip_text::localise(
                                &inputs.counter.localisation,
                                key,
                                &[],
                            )
                        },
                    )
                });
            }
            NameLayer::Factions => Some(map_mode::faction_labels(
                inputs.content,
                self.frame,
                &inputs.names.localisation,
            )),
            NameLayer::Players => Some(map_mode::player_labels(self.frame)),
            NameLayer::Countries => None,
        };
        NameAreas::read(
            &self.content.map,
            &inputs.names.localisation,
            self.frame,
            groups.as_ref(),
        )
    }
    pub fn names(&self, inputs: &NameInputs<'_>, mode: &MapMode) -> io::Result<Arc<Placements>> {
        let layers = inputs.content.name_layers(mode);
        let uses = |wanted| layers.far == wanted || layers.near == wanted;
        let names_localised = [layers.far, layers.near].iter().any(|layer| {
            !matches!(
                layer,
                map_mode::NameLayer::Nothing | map_mode::NameLayer::States
            )
        });
        let area_key = NameAreaKey {
            modes: uses(map_mode::NameLayer::Factions).then(|| inputs.content.projection_id()),
            layers: (layers.far as u8, layers.near as u8),
            witness: self.content.reads.binding(if names_localised {
                &["map", "localisation"]
            } else {
                &["map"]
            })?,
            localisation: names_localised.then(|| Operand::new(&inputs.names.localisation)),
            regions: uses(map_mode::NameLayer::StrategicRegions)
                .then(|| Operand::new(inputs.regions)),
            states: if uses(map_mode::NameLayer::States) {
                Some((
                    Operand::new(&inputs.counter.localisation),
                    Operand::new(&inputs.counter.state_name_keys),
                    KeyText::new(&inputs.counter.recorded_language),
                ))
            } else {
                None
            },
        };
        let key = NameKey {
            areas: area_key.clone(),
            names: inputs.names.projection_id(),
            placement: inputs.placement.content.projection_id(),
            font: inputs.placement.font.projection_id(),
            heightmap: Operand::new(inputs.heightmap),
            witness: self.content.reads.binding(&["map", "heightmap"])?,
        };
        let source = &self.frame.tables[replay_data::rows::TableId::Campaign.index()];
        let areas = self.content.landscape.name_areas.project_equal_fields(
            self.frame,
            area_key,
            Clock::Stable,
            |table| name_fields(table, inputs.content.name_layers(mode)),
            || self.name_areas(inputs, mode),
        )?;
        let mut names = self
            .content
            .landscape
            .names
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(previous) = names.as_ref() {
            if previous.key == key
                && previous.source.is(source)
                && Arc::ptr_eq(&previous.areas, &areas)
            {
                return Ok(Arc::clone(&previous.placements));
            }
        }
        let placed = match names.take() {
            Some(previous) if previous.key == key && previous.source.is(source) => previous.placed,
            _ => PlacedAreas::default(),
        };
        let placements = Arc::new(name_pass::place_areas(
            &inputs.placement,
            &areas,
            Some(&placed),
        ));
        *names = Some(NameProjection {
            key,
            source: source.identity(),
            areas,
            placements: Arc::clone(&placements),
            placed,
        });
        Ok(placements)
    }
}

pub(crate) fn named_fields(table: replay_data::rows::TableId, names: &[&str]) -> u64 {
    names
        .iter()
        .try_fold(0, |mask, name| {
            (0..table.schema().len())
                .find(|&index| table.field_name(index) == Some(*name))
                .map(|index| mask | (1 << index))
        })
        .unwrap_or_else(|| table.full_mask())
}
fn territory_fields(table: replay_data::rows::TableId) -> u64 {
    use replay_data::rows::TableId;
    match table {
        TableId::Provinces => {
            named_fields(table, &["state", "owner", "controller", "victory_points"])
        }
        TableId::Countries => named_fields(table, &["canonical_slot", "tag"]),
        _ => table.full_mask(),
    }
}
fn label_fields(table: replay_data::rows::TableId) -> u64 {
    use replay_data::rows::TableId;
    match table {
        TableId::Provinces => named_fields(table, &["state", "controller", "victory_points"]),
        TableId::States => named_fields(table, &["owner", "controller"]),
        TableId::Countries => named_fields(table, &["canonical_slot", "tag"]),
        _ => table.full_mask(),
    }
}
fn mode_fields(table: replay_data::rows::TableId) -> u64 {
    use replay_data::rows::TableId;
    match table {
        TableId::Provinces => {
            named_fields(table, &["state", "owner", "controller", "victory_points"])
        }
        _ => table.full_mask(),
    }
}
fn rail_fields(table: replay_data::rows::TableId) -> u64 {
    use replay_data::rows::TableId;
    match table {
        TableId::ProvinceSupply => named_fields(
            table,
            &[
                "supply_node_level",
                "naval_base_level",
                "capital_country_index",
                "flags",
            ],
        ),
        TableId::NavalBases => 0,
        _ => table.full_mask(),
    }
}
fn name_fields(table: replay_data::rows::TableId, layers: map_mode::NameLayers) -> u64 {
    use map_mode::NameLayer;
    use replay_data::rows::TableId;
    match table {
        TableId::Provinces => {
            let mut mask = 0;
            for layer in [layers.far, layers.near] {
                mask |= match layer {
                    NameLayer::States => named_fields(table, &["state"]),
                    NameLayer::Countries | NameLayer::Factions | NameLayer::Players => {
                        named_fields(table, &["controller"])
                    }
                    _ => 0,
                };
            }
            mask
        }
        TableId::Countries => named_fields(table, &["canonical_slot", "tag"]),
        TableId::States => named_fields(table, &["custom_name", "owner_name"]),
        _ => table.full_mask(),
    }
}

fn building_fields(table: replay_data::rows::TableId) -> u64 {
    use replay_data::rows::TableId;
    match table {
        TableId::Provinces => named_fields(table, &["state", "controller"]),
        TableId::States => named_fields(table, &["controller"]),
        TableId::CountryLinks => named_fields(table, &["canonical_slot"]),
        TableId::NavalBases => 0,
        TableId::ProvinceSupply => named_fields(table, &["flags", "controller_index"]),
        TableId::StateBuildings | TableId::ProvinceBuildings => named_fields(
            table,
            &[
                "level",
                "healthy_level",
                "partial_health",
                "status_flags",
                "country",
            ],
        ),
        TableId::RaidTargetStatuses => named_fields(
            table,
            &[
                "scope.building_type",
                "scope.building_location",
                "scope.building_flags",
                "scope.building_country",
                "detected",
            ],
        ),
        _ => table.full_mask(),
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn default_projection_is_source_independent_and_keeps_default_wire_bytes() {
        let first = default_mode_projection();
        let second = default_mode_projection();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            product_codec::encode(first.as_ref()).unwrap(),
            product_codec::encode(&map_display_data::landscape::ModeProjection::Stock {
                fill: ModeFill::default(),
            })
            .unwrap(),
        );
    }
    #[test]
    fn name_fitting_inputs_do_not_invalidate_area_grouping_keys() {
        let localisation = Arc::new(Default::default());
        let heightmap = Arc::new(content_reader_image::IndexedImage {
            width: 1,
            height: 1,
            indices: vec![0].into(),
            palette: [[0; 4]; 256],
        });
        let areas = NameAreaKey {
            modes: None,
            layers: (0, 2),
            witness: [1; 32],
            localisation: Some(Operand::new(&localisation)),
            regions: None,
            states: None,
        };
        let first = NameKey {
            areas,
            names: 1,
            placement: 1,
            font: 1,
            heightmap: Operand::new(&heightmap),
            witness: [2; 32],
        };
        let mut font_change = first.clone();
        font_change.font += 1;
        assert!(first != font_change);
        assert!(first.areas == font_change.areas);
        let replacement = Arc::new(heightmap.as_ref().clone());
        let mut height_change = first.clone();
        height_change.heightmap = Operand::new(&replacement);
        assert!(first != height_change);
        assert!(first.areas == height_change.areas);
    }
    #[test]
    fn changed_fog_reuses_only_the_same_building_bank_continuation() {
        let city = Arc::new(vec![true]);
        let old = BuildingKey {
            assets: 1,
            perspective: 7,
            fog: true,
            content: [3; 32],
            city: Operand::new(&city),
            fog_bytes: None,
            visibility_gap: None,
        };
        let mut next = old.clone();
        let fog = Arc::new(perspective::values::Values::from_iter([(1, 255u8)]));
        next.fog_bytes = Some(Operand::new(&fog));
        assert!(old != next);
        assert!(old.same_base(&next));
        let replacement = Arc::new(vec![true]);
        next.city = Operand::new(&replacement);
        assert!(!old.same_base(&next));
        next.city = old.city.clone();
        next.perspective += 1;
        assert!(!old.same_base(&next));
    }
}
