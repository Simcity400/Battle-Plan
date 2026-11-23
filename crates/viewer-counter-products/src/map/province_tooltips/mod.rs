use dashboard_effects::ScriptedEffects;
use super::counter_content;
use viewer_native_gui::native_text::FLAG;
use viewer_foundation::tooltip_text::state_name as tooltip_text_state_name;
pub mod air_region;
mod border;
pub mod clock;
mod inputs;
mod mode_lines;
pub mod naval_accidents;
pub mod naval_base;
pub mod naval_mines;
mod occupation;
mod operatives;
mod resources;
mod selected_attack;
mod selection;
mod strategic_air;
pub use strategic_air::{RegionalDetection, assignment_checks, regional_detection};
mod strategic_navy;
pub use strategic_navy::{dominance_hover, region_dominance};
pub use selected_attack::template_display_name;
mod trade_routes;
pub(super) mod units;
mod weather;

use content_reader_localisation::{localisation::Argument as Arg, localisation::Localisation};
use content_reader_states::states::StateDefinition;
use inputs::Names;
use replay_data::{rows::ProvinceRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use viewer_gpu::ribbon_content::TradeRouteLine;
use dashboard_text::table_access::TokenNames;
use viewer_foundation::{perspective::Vision, tooltip_text::Tooltip};
use viewer_runtime::unit_handle::Unit;

pub struct Content {
    labels: Arc<viewer_runtime::recorded_labels::RecordedLabels>,
    members: BTreeMap<u32, Vec<u32>>,
    impassable: BTreeSet<u32>,
    neighbors: Result<BTreeMap<u32, Vec<u32>>, String>,
    temperature: Result<content_reader_weather::weather::TemperatureRanges, String>,
    pass: Result<replay_data::temperature::Content, String>,
    derived: inputs::TemperaturesByFrame,
    pub(super) units: Result<units::Content, String>,
    border: Result<border::Defines, String>,
    air_log_timeout: Result<i32, String>,
    air_detection: Result<viewer_unit_reads::air_detection::Defines, String>,
    air_range: Result<viewer_unit_reads::air_range::Defines, String>,
    pub(super) air_fuel: Result<super::counter_tooltips::air_fuel::Defines, String>,
    navy: strategic_navy::Defines,
    occupation: Result<mode_lines::OccupationDefines, String>,
    network_base_target: Result<i64, String>,
    resources: Result<viewer_runtime::resource_origins::Content, String>,
}

impl Content {
    pub fn impassable_state(&self, state: u32) -> bool {
        self.impassable.contains(&state)
    }

    pub fn resources(&self) -> Result<&viewer_runtime::resource_origins::Content, &str> {
        self.resources.as_ref().map_err(String::as_str)
    }
}

impl Content {
    pub fn state_members(&self) -> &BTreeMap<u32, Vec<u32>> {
        &self.members
    }
    pub fn load(
        layers: &content_reader_layers::ResolvedLayers,
        defines: &content_reader_defines::defines::Defines,
        states: &BTreeMap<u32, StateDefinition>,
        temperature: &Result<content_reader_weather::weather::TemperatureRanges, String>,
        subunits: Result<&content_reader_script::sub_units::SubUnits, &str>,
        labels: Arc<viewer_runtime::recorded_labels::RecordedLabels>,
    ) -> Self {
        let members = states
            .iter()
            .map(|(&id, state)| (id, state.provinces.clone()))
            .collect();
        let graph = content_reader_map::province_graph::ProvinceGraph::read(layers);
        Self {
            labels,
            impassable: states
                .iter()
                .filter_map(|(&id, state)| state.impassable.then_some(id))
                .collect(),
            neighbors: graph
                .as_ref()
                .map_err(|e| e.to_string())
                .and_then(|(graph, _)| graph.neighbor_states(&members).map_err(|e| e.to_string())),
            pass: graph
                .as_ref()
                .map_err(|e| e.to_string())
                .and_then(|(graph, geometry)| {
                    content_reader_weather::weather::temperature::read_with(layers, graph, geometry)
                        .map_err(|e| e.to_string())
                }),
            derived: Default::default(),
            members,
            temperature: temperature.clone(),
            units: units::Content::load(defines, subunits),
            border: border::Defines::load(defines).map_err(|e| e.to_string()),
            air_log_timeout: defines
                .int32("NGame", "AIR_LOG_TIMEOUT_HOURS")
                .map_err(|e| e.to_string()),
            air_detection: viewer_unit_reads::air_detection::Defines::load(defines)
                .map_err(|e| e.to_string()),
            air_range: (|| -> std::io::Result<_> {
                Ok(viewer_unit_reads::air_range::Defines {
                    map_scale: defines.fixed5("NGame", "MAP_SCALE_PIXEL_TO_KM")?,
                    ace_wing_size: defines.fixed5("NAir", "ACE_WING_SIZE")?,
                    ace_wing_size_max_bonus: defines.fixed5("NAir", "ACE_WING_SIZE_MAX_BONUS")?,
                })
            })()
            .map_err(|e| e.to_string()),
            air_fuel: super::counter_tooltips::air_fuel::Defines::load(defines)
                .map_err(|error| error.to_string()),
            navy: strategic_navy::Defines::load(defines),
            occupation: mode_lines::OccupationDefines::load(defines).map_err(|e| e.to_string()),
            network_base_target: defines
                .fixed5(
                    "NOperatives",
                    "INTEL_NETWORK_BASE_STRENGTH_TARGET_COUNTERINTELLIGENCE_OFFSET",
                )
                .map_err(|e| e.to_string()),
            resources: viewer_runtime::resource_origins::Content::load(layers, defines)
                .map_err(|e| e.to_string()),
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = match &self.units {
            Ok(units) => units.warnings().to_vec(),
            Err(reason) => vec![format!(
                "Province hover composition content unavailable: {reason} (1 content_reader)."
            )],
        };
        if let Ok(temperature) = &self.temperature {
            warnings.extend(temperature.notes.iter().cloned());
        }
        match &self.resources {
            Ok(resources) => warnings.extend(resources.warnings.iter().cloned()),
            Err(reason) => warnings.push(format!(
                "The resources map mode's province hover is not built: {reason}."
            )),
        }
        if let Err(reason) = &self.pass {
            warnings.push(format!(
                "Province temperatures cannot be derived from the recorded content ({reason}); recordings that store only their inputs show none."
            ));
        }
        warnings
    }

    pub(crate) fn border_enemy_share(
        &self,
        frame: &DecodedFrame,
        country: u32,
        links: &BTreeMap<u32, u32>,
        state: u32,
    ) -> Result<i64, String> {
        border::enemy_share(self, frame, country, links, state)
    }

    pub fn prepare_temperature(&self, frame: &Arc<DecodedFrame>) {
        self.derived
            .prepare(frame, self.pass.as_ref().map_err(String::as_str));
    }

    pub fn temperature_at(
        &self,
        frame: &DecodedFrame,
        rows: &[replay_data::rows::WeatherProvinceRow],
        province: u32,
    ) -> Result<i64, String> {
        let _weather_span = viewer_runtime::profiling::span("pt_weather_states");
        self.derived.at(
            frame,
            rows,
            self.pass.as_ref().map_err(String::as_str),
            province,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ModeHover {
    Location { category: bool },
    StrategicAir(Arc<Vec<u64>>),
    StrategicNavy(
        Arc<viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap>,
    ),
    SeaRegion(
        Arc<viewer_unit_reads::naval_dominance_sources::RegionalDominanceSourcesMap>,
    ),
    Region,
    Terrain { region: bool },
    Nothing,
    Manpower,
    Infrastructure(Result<i32, String>),
    Diplomacy { very_good: i32, very_bad: i32 },
    Ideology,
    Operatives,
    Occupation,
    OccupationMarker,
    Resources,
    Scripted(String),
    Unbuilt(String),
}

impl ModeHover {
    pub fn region(&self) -> bool {
        matches!(
            self,
            Self::StrategicAir(_) | Self::StrategicNavy(_) | Self::SeaRegion(_) | Self::Region | Self::Terrain { region: true }
        )
    }
}

pub fn under_routes(
    content: &counter_content::Content,
    frame: &DecodedFrame,
    ours: u32,
    routes: &[TradeRouteLine],
    region: bool,
    below: impl FnOnce() -> Tooltip,
) -> Tooltip {
    if routes.is_empty() {
        return below();
    }
    trade_routes::compose(region, route_text(content, frame, ours, routes), below)
}

type CacheKey = (u32, u32, bool, bool, ModeHover, Vec<TradeRouteLine>);
type TooltipSources = (
    Arc<counter_content::Content>,
    Arc<DecodedFrame>,
    Arc<Vision>,
    Arc<Vec<Unit>>,
);

#[derive(Default)]
pub struct Cache {
    held: Holder<CacheKey>,
    source: Option<TooltipSources>,
}
impl Cache {
    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &mut self,
        content: &Arc<counter_content::Content>,
        reads: viewer_unit_reads::movement_reads::ReadOperands<'_>,
        frame: &Arc<DecodedFrame>,
        province: u32,
        vision: &Arc<Vision>,
        nato: bool,
        selected: &Arc<Vec<Unit>>,
        rosters: &Arc<air_region::Sampler>,
        mode: &ModeHover,
        routes: &[TradeRouteLine],
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Arc<Tooltip> {
        let key = (province, vision.ours(), vision.fog(), nato, mode.clone(), routes.to_vec());
        let same = self.source.as_ref().is_some_and(
            |(held_content, held_frame, held_vision, held_selected)| {
                Arc::ptr_eq(content, held_content)
                    && Arc::ptr_eq(frame, held_frame)
                    && Arc::ptr_eq(vision, held_vision)
                    && Arc::ptr_eq(selected, held_selected)
            },
        );
        let text = self.held.get(key, same, rosters, wake, |rosters| {
            under_routes(content, frame, vision.ours(), routes, mode.region(), || {
                build(content, reads, frame, province, vision, nato, selected, rosters, mode)
            })
        });
        self.source = Some((
            Arc::clone(content),
            Arc::clone(frame),
            Arc::clone(vision),
            Arc::clone(selected),
        ));
        text
    }

    pub fn pending(&self) -> bool {
        self.held.pending()
    }

    pub fn wait(&self) {
        self.held.wait();
    }
}

struct Holder<K> {
    held: Option<(K, u64, bool, Arc<Tooltip>)>,
    rosters: Option<air_region::PassRosters>,
}
impl<K> Default for Holder<K> {
    fn default() -> Self {
        Self {
            held: None,
            rosters: None,
        }
    }
}
impl<K: PartialEq> Holder<K> {
    fn get(
        &mut self,
        key: K,
        same: bool,
        sampler: &Arc<air_region::Sampler>,
        wake: impl Fn() + Send + Sync + 'static,
        build: impl FnOnce(&dyn air_region::Rosters) -> Tooltip,
    ) -> Arc<Tooltip> {
        if !self
            .rosters
            .as_ref()
            .is_some_and(|rosters| Arc::ptr_eq(rosters.sampler(), sampler))
        {
            self.rosters = Some(air_region::PassRosters::of(Arc::clone(sampler), wake));
            self.held = None;
        }
        let rosters = self.rosters.as_ref().expect("installed above");
        let generation = rosters.generation();
        if same
            && let Some((held, built, _, text)) = &self.held
            && *held == key
            && *built == generation
        {
            return Arc::clone(text);
        }
        let asked = Asked {
            rosters,
            pending: std::cell::Cell::new(false),
        };
        let text = Arc::new(build(&asked));
        self.held = Some((key, generation, asked.pending.get(), Arc::clone(&text)));
        text
    }

    fn pending(&self) -> bool {
        self.held
            .as_ref()
            .is_some_and(|(_, _, pending, _)| *pending)
    }

    fn wait(&self) {
        if let (Some((_, generation, true, _)), Some(rosters)) = (&self.held, &self.rosters) {
            rosters.wait_beyond(*generation);
        }
    }
}

struct Asked<'a> {
    rosters: &'a air_region::PassRosters,
    pending: std::cell::Cell<bool>,
}
impl air_region::Rosters for Asked<'_> {
    fn visiting_pass(
        &self,
        branch: replay_data::BranchId,
        hour: replay_data::Hour,
        slot: u32,
    ) -> Option<Result<Option<replay_data::Hour>, String>> {
        let answer = air_region::Rosters::visiting_pass(self.rosters, branch, hour, slot);
        if answer.is_none() {
            self.pending.set(true);
        }
        answer
    }
}

fn localise(loc: &Localisation, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
    loc.localise_with(key.as_bytes(), args)
        .map_err(|e| e.to_string())
}

fn line(out: &mut Tooltip, name: &str, indent: usize, value: Result<String, String>) {
    match value {
        Ok(value) if !value.is_empty() => {
            if !out.title.is_empty() && !out.title.ends_with('\n') {
                out.title.push('\n');
            }
            for (i, text) in value.split('\n').enumerate() {
                if i != 0 {
                    out.title.push('\n');
                }
                if !text.is_empty() {
                    out.title.extend(std::iter::repeat_n(' ', indent * 4));
                    out.title.push_str(text);
                }
            }
        }
        Ok(_) => {}
        Err(reason) => {
            *out.refused
                .entry(format!("province {name}: {reason}"))
                .or_default() += 1;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build(
    content: &counter_content::Content,
    reads: viewer_unit_reads::movement_reads::ReadOperands<'_>,
    frame: &DecodedFrame,
    id: u32,
    vision: &Vision,
    nato: bool,
    selected: &[Unit],
    rosters: &dyn air_region::Rosters,
    mode: &ModeHover,
) -> Tooltip {
    let sources_span = viewer_runtime::profiling::span("pt_sources");
    let mut out = Tooltip::default();
    let provinces = match frame.complete::<tables::Provinces>() {
        Ok(rows) => rows,
        Err(e) => {
            line(&mut out, "location", 0, Err(e.to_string()));
            return out;
        }
    };
    let Some(province) = provinces.iter().find(|p| p.id == id) else {
        line(
            &mut out,
            "location",
            0,
            Err(format!("Provinces lacks {id}")),
        );
        return out;
    };
    let links = match frame.complete::<tables::CountryLinks>() {
        Ok(rows) => rows.iter().map(|r| (r.id, r.canonical_slot)).collect(),
        Err(e) => {
            line(&mut out, "country references", 0, Err(e.to_string()));
            return out;
        }
    };
    drop(sources_span);
    let countries = Names::new(frame, &content.localisation);
    let context = Context {
        content,
        reads,
        frame,
        vision,
        province,
        links,
        countries,
        rosters,
    };
    let ModeHover::Location { category } = mode else {
        context.mode_lines(mode, &mut out);
        return out;
    };
    if province.state != 0 {
        context.location(&mut out, *category);
    }
    let units_span = viewer_runtime::profiling::span("pt_units");
    let composition = content
        .province_hover
        .units
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|units| {
            units::summary(
                units,
                frame,
                id,
                &context.links,
                vision,
                &content.localisation,
                nato,
            )
        });
    line(
        &mut out,
        "unit composition",
        1,
        composition.map(|s| {
            if s.is_empty() {
                s
            } else {
                format!("\u{11}Y{s}\u{11}! \n")
            }
        }),
    );
    drop(units_span);
    if !out.title.is_empty() {
        match localise(&content.localisation, "PROV_TOOLTIP_LOCATION_HEADER", &[]) {
            Ok(header) => out.title.insert_str(0, &header),
            Err(e) => line(&mut out, "pin header", 0, Err(e)),
        }
    }
    {
        let _rosters_span = viewer_runtime::profiling::span("pt_rosters");
        air_region::append(&context, &mut out);
    }
    let _selection_span = viewer_runtime::profiling::span("pt_selection");
    let selected = selection::build(&context, selected, nato);
    selection::append(&mut out, selected, &content.localisation);
    click_footer(&content.localisation, content.kinds.get(&id).copied(), &mut out);
    out
}

fn route_text(content: &counter_content::Content, frame: &DecodedFrame, ours: u32, routes: &[TradeRouteLine]) -> trade_routes::Routes {
    let links: Result<BTreeMap<u32, u32>, String> = frame
        .complete::<tables::CountryLinks>()
        .map(|rows| rows.iter().map(|row| (row.id, row.canonical_slot)).collect())
        .map_err(|error| error.to_string());
    let countries = Names::new(frame, &content.localisation);
    let loc = |key: &str, args: &[(&str, Arg<'_>)]| localise(&content.localisation, key, args);
    let state = |id: u32| state_name_of(content, frame, id);
    let country = |raw: u32| {
        let slot = links.as_ref().map_err(Clone::clone)?.get(&raw).copied().ok_or_else(|| format!("CountryLinks lacks {raw}"))?;
        countries.name(slot)
    };
    let resource = |id: u32| {
        let resources = content.province_hover.resources.as_ref().map_err(Clone::clone)?;
        let declared = (id as usize)
            .checked_sub(1)
            .and_then(|at| resources.resources.get(at))
            .ok_or_else(|| format!("recorded resource {id} has no declaration in the content's resource files"))?;
        let key = format!("PRODUCTION_MATERIALS_{}", content_reader_localisation::localisation::crt_case_text(&declared.token, true, false));
        localise(&content.localisation, &key, &[])
    };
    let viewed = |raw: u32| Ok(links.as_ref().map_err(Clone::clone)?.get(&raw) == Some(&ours));
    let weight = |region: u32| {
        let weights = frame.complete::<tables::StrategicRegionWeights>().map_err(|error| error.to_string())?;
        weights
            .binary_search_by_key(&region, |row| row.region)
            .map(|at| weights[at].weight)
            .map_err(|_| format!("strategic region {region} has no recorded route weight"))
    };
    let controls = |raw: u32, region: u32| {
        let links = links.as_ref().map_err(Clone::clone)?;
        let slot = links.get(&raw).copied().ok_or_else(|| format!("CountryLinks lacks {raw}"))?;
        strategic_navy::region_dominance(content, frame, slot, region, links).map(|(_, control, _)| control == 1)
    };
    let names = trade_routes::Names {
        loc: &loc,
        state: &state,
        country: &country,
        resource: &resource,
        viewed: &viewed,
        weight: &weight,
        controls: &controls,
        factor: &content.province_hover.navy.convoy,
    };
    trade_routes::assemble(routes, true, &|key| loc(key, &[]), |line| trade_routes::describe(frame, &names, line))
}

fn state_name_of(content: &counter_content::Content, frame: &DecodedFrame, id: u32) -> Result<String, String> {
    let state = frame
        .complete::<tables::States>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|s| s.id == id)
        .ok_or("state row absent")?;
    tooltip_text_state_name(&content.localisation, &content.recorded_language, id, &state.custom_name, &state.owner_name, || {
        let key = content.state_name_keys.get(&id).ok_or("state content name absent")?.as_ref().map_err(Clone::clone)?;
        localise(&content.localisation, key, &[])
    })
}

fn click_footer(loc: &Localisation, kind: Option<content_reader_map::map::ProvinceKind>, out: &mut Tooltip) {
    match kind {
        Some(content_reader_map::map::ProvinceKind::Land) => match localise(loc, "PROVINCE_CLICK_VIEW", &[]) {
            Ok(text) => {
                viewer_foundation::tooltip_text::ensure_newlines(&mut out.body, 1);
                out.body.push_str(&text);
            }
            Err(why) => out.refuse("click footer", why),
        },
        Some(_) => {}
        None => out.refuse("click footer", "the province has no class in the content".into()),
    }
}

struct Context<'a> {
    reads: viewer_unit_reads::movement_reads::ReadOperands<'a>,
    content: &'a counter_content::Content,
    frame: &'a DecodedFrame,
    vision: &'a Vision,
    province: &'a ProvinceRow,
    links: BTreeMap<u32, u32>,
    countries: Names<'a>,
    rosters: &'a dyn air_region::Rosters,
}
impl Context<'_> {
    fn loc(&self, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
        localise(&self.content.localisation, key, args)
    }
    fn slot(&self, raw: u32) -> Result<u32, String> {
        self.links
            .get(&raw)
            .copied()
            .ok_or_else(|| format!("CountryLinks lacks {raw}"))
    }
    fn marked_country(&self, raw: u32) -> Result<String, String> {
        self.marked_slot(self.slot(raw)?)
    }
    fn marked_slot(&self, slot: u32) -> Result<String, String> {
        let tag = self.countries.tag(slot, "country naming operands absent")?;
        if tag.chars().count() != 3 {
            return Err(format!(
                "country slot {slot} flag tag is not three codepoints"
            ));
        }
        let name = content_reader_localisation::localisation::crt_case_text(
            &self.countries.name_with(slot, "_DEF")?,
            true,
            true,
        );
        Ok(format!("{}{} {name}", FLAG, tag))
    }
    fn scaled_static_modifiers(&self, token: &str, scale: i64) -> Result<String, String> {
        let scripted = self
            .content
            .scripted
            .as_ref()
            .as_ref()
            .map_err(Clone::clone)?;
        let text = dashboard_text::scripted_text::ScriptedText::new(
            self.frame,
            &self.content.localisation,
            scripted,
            self.vision.ours(),
        )
        .viewing(self.vision.ours());
        let effects = scripted.effects()?;
        let context = dashboard_effects::effect_description::Context::new(
            &text,
            &self.content.localisation,
            self.vision.ours(),
            "common/modifiers",
            effects.view(None),
        );
        dashboard_effects::effect_description::scaled_static_modifiers(&context, token, "  ", scale)
    }
    fn state_name(&self) -> Result<String, String> {
        self.state_name_of(self.province.state)
    }
    fn state_name_of(&self, id: u32) -> Result<String, String> {
        state_name_of(self.content, self.frame, id)
    }
    fn location(&self, out: &mut Tooltip, category: bool) {
        let p = self.province;
        let loc = &self.content.localisation;
        line(
            out,
            "terrain/state",
            0,
            (|| {
                let terrain = self
                    .content
                    .terrain
                    .combat_terrain
                    .get(&p.id)
                    .ok_or("province terrain absent")?;
                let terrain = self.loc(terrain, &[])?;
                let state = self.state_name()?;
                self.loc(
                    "PROV_TOOLTIP_LOCATION_STATE",
                    &[
                        ("TERRAIN", Arg::Text(&terrain)),
                        ("NAME", Arg::Text(&state)),
                    ],
                )
            })(),
        );
        line(
            out,
            "owner",
            1,
            self.marked_country(p.owner).and_then(|name| {
                self.loc("PROV_TOOLTIP_LOCATION_OWNER", &[("NAME", Arg::Text(&name))])
            }),
        );
        let controller = (|| {
            if p.owner == p.controller {
                return Ok(String::new());
            }
            if p.owner != 0
                && p.controller != 0
                && self.slot(p.owner)? == self.slot(p.controller)?
            {
                return Ok(String::new());
            }
            let name = self.marked_country(p.controller)?;
            self.loc(
                "PROV_TOOLTIP_LOCATION_CONTROLLER",
                &[("COUNTRY", Arg::Text(&name))],
            )
        })();
        line(out, "controller", 1, controller);
        if category {
            line(out, "state category", 1, self.category());
        }
        let flags = self
            .frame
            .complete::<tables::StateFlags>()
            .map_err(|e| e.to_string())
            .and_then(|rows| {
                rows.iter()
                    .find(|r| r.state == p.state)
                    .ok_or_else(|| "state flags absent".into())
            });
        line(
            out,
            "demilitarized zone",
            1,
            flags.as_ref().map_err(Clone::clone).and_then(|r| {
                if r.demilitarized != 0 {
                    self.loc("DEMILITARIZED_ZONE", &[])
                } else {
                    Ok(String::new())
                }
            }),
        );
        line(
            out,
            "impassable",
            1,
            self.content
                .province_hover
                .members
                .get(&p.state)
                .ok_or_else(|| "state content absent".into())
                .and_then(|_| {
                    if self.content.province_hover.impassable.contains(&p.state) {
                        self.loc("PROVINCE_STATE_IMPASSABLE", &[])
                    } else {
                        Ok(String::new())
                    }
                }),
        );
        line(
            out,
            "weather",
            1,
            (|| {
                let rows = self
                    .frame
                    .complete::<tables::WeatherProvinces>()
                    .map_err(|e| e.to_string())?;
                let weather = rows
                    .iter()
                    .find(|r| r.id == p.id)
                    .ok_or("province weather absent")?;
                let temperature = self
                    .content
                    .province_hover
                    .temperature_at(self.frame, rows, p.id);
                let region = self
                    .content
                    .region_of
                    .get(&(p.id))
                    .copied()
                    .ok_or("province strategic region absent")?;
                let region = self
                    .frame
                    .complete::<tables::WeatherRegions>()
                    .map_err(|e| e.to_string())?
                    .iter()
                    .find(|r| r.id == region)
                    .ok_or("region weather absent")?;
                weather::row(
                    loc,
                    &self
                        .content
                        .province_hover
                        .temperature
                        .as_ref()
                        .map_err(Clone::clone)?
                        .ranges,
                    temperature,
                    weather,
                    region,
                )
            })(),
        );
        line(
            out,
            "local time/night",
            1,
            (|| {
                let weather = self
                    .frame
                    .complete::<tables::WeatherProvinces>()
                    .map_err(|e| e.to_string())?
                    .iter()
                    .find(|row| row.id == p.id)
                    .ok_or("province weather absent")?;
                let clock = clock::resolve(
                    weather,
                    |province| {
                        self.reads
                            .updated_clock(province, self.frame.hour)
                            .map_err(str::to_owned)
                    },
                    || {
                        self.frame
                            .complete::<tables::SimulationFeatures>()
                            .map_err(|e| e.to_string())?
                            .first()
                            .ok_or("simulation features absent")?
                            .weather_suspend
                            .require("SimulationFeatures.weather_suspend")
                            .copied()
                    },
                )?;
                self.loc(
                    "PROV_TOOLTIP_LOCAL_TIME",
                    &[
                        ("TIME", Arg::Text(&format!("{}:00", clock.hour))),
                        ("FRAME", Arg::Int32(clock.icon_frame)),
                    ],
                )
            })(),
        );
        match self.vision.province_byte(p.id) {
            Some(byte) if byte > 49 => self.buildings(out),
            Some(_) => {}
            None => line(
                out,
                "building visibility",
                0,
                Err("province fog byte unavailable".into()),
            ),
        }
        line(
            out,
            "border war",
            0,
            flags.and_then(|flags| {
                border::build(
                    &self.content.province_hover,
                    loc,
                    self.frame,
                    self.vision.ours(),
                    &self.links,
                    p.state,
                    flags,
                )
            }),
        );
        if p.victory_points > 0 {
            line(
                out,
                "victory points",
                1,
                self.loc(
                    "PROVINCE_VP_TOOLTIP",
                    &[("VALUE", Arg::Int32(p.victory_points))],
                ),
            );
        }
    }
    fn buildings(&self, out: &mut Tooltip) {
        let p = self.province;
        let sites = self.content.building_facts.province_sites(p.state, p.id);
        for (site, kind, key) in [
            (sites[0], 0, "PROVINCE_AIRBASE_TOOLTIP"),
            (sites[1], 1, "PROVINCE_ROCKET_TOOLTIP"),
            (sites[2], 2, "Gun emplacement"),
        ] {
            if !site {
                continue;
            }
            line(
                out,
                key,
                1,
                (|| {
                    let bases = self
                        .frame
                        .complete::<tables::AirBases>()
                        .map_err(|e| e.to_string())?;
                    let mut found = bases
                        .iter()
                        .filter(|b| b.state == Some(p.state) && b.kind == kind);
                    let Some(base) = found.next() else {
                        return Ok(String::new());
                    };
                    if found.next().is_some() {
                        return Err(format!(
                            "multiple state/kind air bases for {}/{kind}",
                            p.state
                        ));
                    }
                    if kind == 2 {
                        Ok(key.into())
                    } else {
                        self.loc(key, &[("CAPACITY", Arg::Int32(base.capacity))])
                    }
                })(),
            );
        }
        line(
            out,
            "special facility",
            1,
            (|| {
                let tokens = TokenNames::new(self.frame).map_err(|e| e.to_string())?;
                let buildings = self
                    .frame
                    .complete::<tables::ProvinceBuildings>()
                    .map_err(|e| e.to_string())?;
                let mut rows: Vec<_> = buildings
                    .iter()
                    .filter(|b| b.location == p.id && b.active)
                    .collect();
                rows.sort_by_key(|b| b.ordinal);
                let mut facility = false;
                for building in rows {
                    let name = tokens
                        .name(building.building_type)
                        .map_err(|e| e.to_string())?;
                    if self.content.building_facts.is_facility(name)? {
                        facility = true;
                        break;
                    }
                }
                if !facility {
                    return Ok(String::new());
                }
                let programs = self
                    .frame
                    .complete::<tables::RaidFacilities>()
                    .map_err(|e| e.to_string())?;
                Ok(
                    if programs
                        .iter()
                        .any(|r| r.country == self.vision.ours() && r.province == p.id)
                    {
                        "Special facility".into()
                    } else {
                        String::new()
                    },
                )
            })(),
        );
        line(
            out,
            "naval base",
            1,
            (|| {
                let bases = self
                    .frame
                    .complete::<tables::NavalBases>()
                    .map_err(|e| e.to_string())?;
                let Some(base) = bases.iter().find(|b| b.province == p.id) else {
                    return Ok(String::new());
                };
                let owner = self.slot(base.country)?;
                if self.vision.fog() {
                    self.vision
                        .shares_vision_proven(owner)
                        .map_err(|e| e.describe())?;
                }
                let detail = self
                    .vision
                    .naval_port_detail(owner)
                    .ok_or("naval port detail unavailable")?;
                if detail < 1 {
                    self.loc("PROVINCE_PORT_TOOLTIP", &[])
                } else {
                    self.loc(
                        "PROVINCE_PORT_TOOLTIP_LEVEL",
                        &[("LEVEL", Arg::Int32(base.level))],
                    )
                }
            })(),
        );
    }
}

#[cfg(test)]
mod tests;
