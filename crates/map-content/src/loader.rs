use crate::witness::ContentReads;
use dashboard_text::triggers::LeaderRoles;
use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_map::map::MapData;
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
pub use viewer_gpu::stack_anchors::{StackAnchors, default_anchor, seed_default_anchors};
use viewer_foundation::{
    visibility::FogContent,
    movement_timing::MovementContent,
    trigger_content::TriggerContent,
};

#[derive(Clone)]
pub struct Loader {
    layers: ResolvedLayers,
    pub reads: ContentReads,
    cache_dir: Option<PathBuf>,
}

impl Loader {
    pub fn new(layers: &ResolvedLayers) -> Self {
        Self {
            layers: layers.clone(),
            reads: ContentReads::new(layers),
            cache_dir: None,
        }
    }

    pub fn with_cache(mut self, cache_dir: &Path) -> Self {
        self.cache_dir = Some(cache_dir.to_path_buf());
        self
    }

    pub fn reader(&self, layers: &ResolvedLayers, purpose: &str) -> ResolvedLayers {
        self.reads.reader(layers, purpose)
    }

    pub fn result<T>(&self, result: io::Result<T>) -> io::Result<T> {
        if let Err(error) = &result {
            self.reads.session_error(error);
        }
        result
    }

    pub fn map(&self) -> io::Result<MapData> {
        self.result(content_reader_map::map::load(&self.reader(&self.layers, "map")))
    }

    pub fn terrain(
        &self,
    ) -> io::Result<(
        content_reader_key_names::terrain::TerrainDefinitions,
        content_reader_key_names::terrain::DefaultMap,
    )> {
        let result = self.result(content_reader_key_names::terrain::read(
            &self.reader(&self.layers, "terrain"),
        ))?;
        for reason in &result.0.skipped {
            self.reads.session_note(&format!(
                "Terrain content the game logs and reads past: {reason}."
            ));
        }
        Ok(result)
    }

    pub fn defines(&self) -> io::Result<Defines> {
        self.result(content_reader_defines::defines::read(
            &self.reader(&self.layers, "defines"),
        ))
    }

    pub fn audio(
        &self,
        defines: &Defines,
        user: &Path,
        warnings: &mut Vec<String>,
    ) -> Option<Arc<viewer_audio::Source>> {
        let start = warnings.len();
        let mut audio = audio_source(
            &self.reader(&self.layers, "audio"),
            defines,
            user,
            warnings,
            Some(&self.reads),
        );
        for warning in &warnings[start..] {
            self.reads.session_note(warning);
        }
        if let Some(source) = &mut audio {
            Arc::get_mut(source)
                .expect("new audio source is unshared")
                .layers = self.layers.clone();
        }
        audio
    }

    pub fn heightmap(
        &self,
        default_map: &content_reader_key_names::terrain::DefaultMap,
    ) -> io::Result<content_reader_image::IndexedImage> {
        let layers = self.reader(&self.layers, "heightmap");
        self.result((|| {
            content_reader_map::terrain_images::read_indexed_image(
                &layers,
                &format!("map/{}", default_map.heightmap),
            )
        })())
    }

    pub fn buildings(&self) -> io::Result<content_reader_key_names::buildings::Buildings> {
        self.result(content_reader_key_names::buildings::read(
            &self.reader(&self.layers, "trigger"),
        ))
    }

    pub fn ideologies(
        &self,
    ) -> io::Result<(
        content_reader_key_names::ideologies::Ideologies,
        Vec<content_reader_parser::Diagnostic>,
    )> {
        let result = self.result(content_reader_key_names::ideologies::read(
            &self.reader(&self.layers, "trigger"),
        ))?;
        for (_, warning) in &result.1 {
            self.reads
                .session_note(&format!("Combat tactics: ideologies: {warning}"));
        }
        Ok(result)
    }

    pub fn resources(
        &self,
    ) -> io::Result<(
        content_reader_key_names::resources::Resources,
        Vec<content_reader_parser::Diagnostic>,
    )> {
        let result = self.result(content_reader_key_names::resources::read(
            &self.reader(&self.layers, "trigger"),
        ))?;
        for (_, warning) in &result.1 {
            self.reads
                .session_note(&format!("Combat tactics: resources: {warning}"));
        }
        Ok(result)
    }

    pub fn localisation(
        &self,
        stack: &ResolvedLayers,
        language: &str,
    ) -> io::Result<Arc<content_reader_localisation::localisation::Localisation>> {
        let reader = self.reader(stack, "localisation");
        let read = match &self.cache_dir {
            Some(cache_dir) => {
                content_reader_localisation::localisation::read_cached(&reader, language, cache_dir)
            }
            None => content_reader_localisation::localisation::read(&reader, language),
        };
        let localisation = self.result(read.map(Arc::new))?;
        for note in localisation
            .logged
            .iter()
            .chain(&localisation.refused)
            .chain(&localisation.duplicates)
        {
            self.reads.session_note(note);
        }
        Ok(localisation)
    }

    pub fn validate_heightmap(
        &self,
        image: &content_reader_image::IndexedImage,
        map: [u32; 2],
    ) -> io::Result<()> {
        self.result(viewer_gpu::textures::expect_size(
            "map/heightmap.bmp",
            image.width,
            image.height,
            map,
        ))
    }

    pub fn movement(&self, defines: &Defines) -> io::Result<MovementContent> {
        let layers = self.reader(&self.layers, "movement");
        self.result(movement(&layers, defines))
    }

    pub fn anchors(&self, stack: &ResolvedLayers) -> io::Result<StackAnchors> {
        self.result(StackAnchors::read(&self.reader(stack, "anchors")))
    }

    pub fn fog(
        &self,
        stack: &ResolvedLayers,
        map: &MapData,
        defines: &Defines,
    ) -> Arc<Result<FogContent, String>> {
        Arc::new(
            self.result(FogContent::load(&self.reader(stack, "fog"), map, defines))
                .map_err(|error| error.to_string()),
        )
    }

    pub fn combat(&self, stack: &ResolvedLayers) -> io::Result<CombatInputs> {
        self.result(CombatInputs::read(&self.reader(stack, "trigger")))
    }
}

pub struct TriggerLoader {
    layers: ResolvedLayers,
    loader: Option<Loader>,
}

impl TriggerLoader {
    pub fn new(layers: &ResolvedLayers, loader: Option<&Loader>) -> Self {
        Self {
            layers: loader
                .map_or_else(|| layers.clone(), |loader| loader.reader(layers, "trigger")),
            loader: loader.cloned(),
        }
    }

    fn checked<T>(&self, result: io::Result<T>) -> io::Result<T> {
        checked(self.loader.as_ref(), result)
    }

    pub fn buildings(&self) -> io::Result<content_reader_key_names::buildings::Buildings> {
        self.checked(content_reader_key_names::buildings::read(&self.layers))
    }

    pub fn tag_aliases(&self) -> io::Result<content_reader_key_names::tag_aliases::TagAliases> {
        self.checked(content_reader_key_names::tag_aliases::read(&self.layers))
    }

    pub fn leader_roles(&self) -> Result<Arc<LeaderRoles>, String> {
        let result = leader_roles(&self.layers);
        if let Err(error) = &result {
            self.note(&format!("Character templates cannot be read ({error}); unit leaders whose visibility has a condition are undecided."));
        }
        result
    }

    fn note(&self, note: &str) {
        if let Some(loader) = &self.loader {
            loader.reads.session_note(note);
        }
    }

    pub fn states(
        &self,
        defines: &Defines,
        map: &MapData,
    ) -> io::Result<(
        content_reader_states::states::States,
        Vec<content_reader_parser::Diagnostic>,
    )> {
        self.checked(content_reader_states::states::read(
            &self.layers,
            defines,
            &map.definitions,
        ))
    }

    pub fn graph(
        &self,
    ) -> io::Result<(
        Result<content_reader_map::province_graph::ProvinceGraph, String>,
        content_reader_map::province_geometry::ProvinceGeometry,
    )> {
        let result = self.checked(graph(&self.layers))?;
        if let Err(error) = &result.0 {
            self.note(&format!("The province graph cannot be built ({error}); naval and building raid sources cannot be ranged."));
        }
        Ok(result)
    }

    pub fn dlcs(&self) -> io::Result<std::collections::BTreeSet<String>> {
        self.checked(self.layers.dlc_names())
    }

    pub fn continents(&self) -> Result<Vec<String>, String> {
        let result = continents(&self.layers);
        if let Err(error) = &result {
            self.note(&format!("continents: {error}"));
        }
        result
    }

    pub fn resource_bonuses(&self, defines: &Defines) -> io::Result<[i64; 2]> {
        self.checked(dashboard_text::state_resources::resource_bonuses(
            defines,
        ))
    }

    pub fn neighbor_states(
        &self,
        graph: Result<&content_reader_map::province_graph::ProvinceGraph, &String>,
        states: &BTreeMap<u32, Vec<u32>>,
    ) -> Result<BTreeMap<u32, Vec<u32>>, String> {
        let result = neighbors(graph, states);
        if let Err(error) = &result {
            self.note(&format!("State neighbours cannot be derived ({error}); any_neighbor_state and all_neighbor_state are unknown."));
        }
        result
    }

    pub fn invasion_prepare_days(&self, defines: &Defines) -> Result<i32, String> {
        let result = defines
            .int32("NNavy", "NAVAL_INVASION_PREPARE_DAYS")
            .map_err(|error| error.to_string());
        if let Err(error) = &result {
            self.note(&format!("naval invasion preparation: {error}"));
        }
        result
    }

    pub fn constants(&self) -> Result<content_reader_politics::script_constants::Constants, String> {
        let result =
            content_reader_politics::script_constants::read(&self.layers).map_err(|error| error.to_string());
        if let Err(error) = &result {
            self.note(&format!("script constants: {error}"));
        }
        result
    }

    pub fn load(&self, defines: &Defines, map: &MapData) -> io::Result<TriggerContent> {
        let buildings = self.buildings()?;
        let tag_aliases = Arc::new(self.tag_aliases()?);
        let leader_roles = self.leader_roles();
        let (states, _) = self.states(defines, map)?;
        let states = states.definitions();
        let (state_provinces, impassable) = state_content(states);
        let (graph, _) = self.graph()?;
        let neighbor_states = self.neighbor_states(graph.as_ref(), &state_provinces);
        let dlcs = self.dlcs()?;
        let resource_bonuses = self.resource_bonuses(defines)?;
        let continents = self.continents();
        let invasion_prepare_days = self.invasion_prepare_days(defines);
        let script_constants = self.constants();
        Ok(assemble_trigger_content(
            tag_aliases,
            leader_roles,
            buildings,
            state_provinces,
            impassable,
            neighbor_states,
            dlcs,
            resource_bonuses,
            continents,
            invasion_prepare_days,
            script_constants,
        ))
    }
}

pub fn state_content(
    states: &BTreeMap<u32, content_reader_states::states::StateDefinition>,
) -> (BTreeMap<u32, Vec<u32>>, std::collections::BTreeSet<u32>) {
    (
        states
            .iter()
            .map(|(&id, state)| (id, state.provinces.clone()))
            .collect(),
        states
            .iter()
            .filter(|(_, state)| state.impassable)
            .map(|(&id, _)| id)
            .collect(),
    )
}

pub fn assemble_trigger_content(
    tag_aliases: Arc<content_reader_key_names::tag_aliases::TagAliases>,
    leader_roles: Result<Arc<LeaderRoles>, String>,
    buildings: content_reader_key_names::buildings::Buildings,
    state_provinces: BTreeMap<u32, Vec<u32>>,
    impassable: std::collections::BTreeSet<u32>,
    neighbor_states: Result<BTreeMap<u32, Vec<u32>>, String>,
    dlcs: std::collections::BTreeSet<String>,
    resource_bonuses: [i64; 2],
    continents: Result<Vec<String>, String>,
    invasion_prepare_days: Result<i32, String>,
    script_constants: Result<content_reader_politics::script_constants::Constants, String>,
) -> TriggerContent {
    TriggerContent {
        triggers: Default::default(),
        ideologies: None,
        character_traits: None,
        terrain: None,
        tag_aliases,
        leader_roles,
        buildings,
        state_provinces,
        impassable,
        neighbor_states,
        dlcs,
        resource_bonuses,
        continents,
        invasion_prepare_days,
        script_constants,
    }
}

pub fn checked<T>(loader: Option<&Loader>, result: io::Result<T>) -> io::Result<T> {
    match loader {
        Some(loader) => loader.result(result),
        None => result,
    }
}

pub struct CombatInputs {
    pub traits: content_reader_key_names::unit_leader::Traits,
    pub sub_units: Result<content_reader_script::sub_units::SubUnits, String>,
    pub country_traits: Result<Arc<content_reader_military::leader_traits::LeaderTraits>, String>,
    pub database: content_reader_military::combat_tactics::Database,
    pub scripted: content_reader_script::scripted_trigger_grammar::ScriptedTriggers,
}
impl CombatInputs {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let traits = content_reader_key_names::unit_leader::Traits::registered(layers)?;
        let sub_units =
            content_reader_key_names::unit_leader::sub_units(layers).map_err(|error| error.to_string());
        let country_traits = content_reader_military::leader_traits::read(layers)
            .map(|(traits, _)| Arc::new(traits))
            .map_err(|error| error.to_string());
        let database = content_reader_military::combat_tactics::read(layers)?;
        let scripted = content_reader_key_names::scripted_triggers::read(layers)?;
        Ok(Self {
            traits,
            sub_units,
            country_traits,
            database,
            scripted,
        })
    }

    pub fn names(
        &self,
        buildings: &content_reader_key_names::buildings::Buildings,
        ideologies: &content_reader_key_names::ideologies::Ideologies,
        resources: &content_reader_key_names::resources::Resources,
    ) -> content_reader_script::trigger_keys::TriggerNames {
        content_reader_script::trigger_keys::TriggerNames {
            scripted: resolved_scripted(&self.database.phases, self.scripted.clone()),
            counts: Some(content_reader_key_names::key_name_products::count_trigger_names(
                buildings, ideologies, resources,
            )),
        }
    }
}

pub fn resolved_scripted(
    phases: &[Vec<u8>],
    mut scripted: content_reader_script::scripted_trigger_grammar::ScriptedTriggers,
) -> content_reader_script::scripted_trigger_grammar::ScriptedTriggers {
    for template in scripted.templates.values_mut() {
        if let Ok(body) = &mut template.body {
            content_reader_military::combat_tactics::resolve_phase_triggers(phases, body);
        }
    }
    scripted
}

pub fn movement(layers: &ResolvedLayers, defines: &Defines) -> io::Result<MovementContent> {
    Ok(MovementContent::new(
        content_reader_map::movement::MovementMap::read(layers, defines)?,
        content_reader_map::movement::MovementDefines::read(defines)?,
    ))
}

pub fn leader_roles(layers: &ResolvedLayers) -> Result<Arc<LeaderRoles>, String> {
    content_reader_key_names::characters::leader_templates(layers)
        .map(|templates| LeaderRoles::read_through(templates, layers.clone()))
        .map(Arc::new)
}

pub fn graph(
    layers: &ResolvedLayers,
) -> io::Result<(
    Result<content_reader_map::province_graph::ProvinceGraph, String>,
    content_reader_map::province_geometry::ProvinceGeometry,
)> {
    match content_reader_map::province_graph::ProvinceGraph::read(layers) {
        Ok((graph, geometry)) => Ok((Ok(graph), geometry)),
        Err(error) => Ok((
            Err(error.to_string()),
            content_reader_map::province_geometry::ProvinceGeometry::read(layers)?,
        )),
    }
}

pub fn neighbors(
    graph: Result<&content_reader_map::province_graph::ProvinceGraph, &String>,
    states: &BTreeMap<u32, Vec<u32>>,
) -> Result<BTreeMap<u32, Vec<u32>>, String> {
    match graph {
        Ok(graph) => graph
            .neighbor_states(states)
            .map_err(|error| error.to_string()),
        Err(error) => Err(format!("the province graph cannot be built ({error})")),
    }
}

pub fn continents(layers: &ResolvedLayers) -> Result<Vec<String>, String> {
    content_reader_key_names::terrain::read_default_map(layers)
        .and_then(|map| layers.read(&map.file("continent")?))
        .and_then(|bytes| {
            content_reader_equipment_graphics::equipment_graphics::context::continent_names(&bytes)
        })
        .map_err(|why| why.to_string())
}
fn audio_source(
    layers: &content_reader_layers::ResolvedLayers,
    defines: &content_reader_defines::defines::Defines,
    user: &std::path::Path,
    warnings: &mut Vec<String>,
    reads: Option<&ContentReads>,
) -> Option<Arc<viewer_audio::Source>> {
    let catalog = match content_reader_sound::sound::Catalog::read(layers) {
        Ok(catalog) => catalog,
        Err(error) => {
            warnings.push(format!(
                "Map audio is unavailable: the sound definitions are unreadable ({error})."
            ));
            return None;
        }
    };
    if !catalog.notes.is_empty() {
        warnings.push(format!(
            "Sound definitions the game refuses or reads past: {}.",
            catalog.notes.lines().join("; ")
        ));
    }
    let recently_played = defines
        .int32("NGame", "MUSIC_PLAYER_RECENTLY_PLAYED_SIZE")
        .map_err(|error| warnings.push(format!("Map music repeats songs freely: {error}.")))
        .map_or(0, |size| usize::try_from(size).unwrap_or(0));
    let battle = content_reader_sound::combat_sounds::BattleDefines::load(defines)
        .map_err(|error| warnings.push(format!("Map battles are silent: {error}.")))
        .ok();
    let combat_sounds = match content_reader_sound::combat_sounds::Layers::read(layers) {
        Ok(combat_sounds) => {
            if !combat_sounds.notes.is_empty() {
                warnings.push(format!(
                    "Battle sound definitions the game refuses or reads past: {}.",
                    combat_sounds.notes.lines().join("; ")
                ));
            }
            combat_sounds
        }
        Err(error) => {
            warnings.push(format!("Map battles play no layers: {error}."));
            Default::default()
        }
    };
    let defines = content_reader_sound::sound::Defines::load(defines)
        .map_err(|error| {
            warnings.push(format!(
                "Map audio plays its height and voice categories without their volumes: {error}."
            ))
        })
        .ok();
    let music = match content_reader_sound::music::Music::read(layers) {
        Ok(music) => {
            if !music.notes.is_empty() {
                warnings.push(format!(
                    "Music definitions the game refuses or reads past: {}.",
                    music.notes.lines().join("; ")
                ));
            }
            music
        }
        Err(error) => {
            warnings.push(format!("Map music is unavailable: {error}."));
            Default::default()
        }
    };
    let stations = content_reader_sound::music::enabled_stations_with(user, |path| {
        let result = std::fs::read(path);
        if let Some(reads) = reads {
            reads.user_bytes(b"@user/playlists/settings.txt", &result);
        }
        result
    })
    .map_err(|error| {
        warnings.push(format!(
            "Map music plays every station: the playlist settings are unreadable ({error})."
        ))
    })
    .ok()
    .flatten();
    Some(Arc::new(viewer_audio::Source {
        content: Arc::new(
            viewer_audio::Content::new(catalog, defines)
                .with_music(music, stations, recently_played)
                .with_battles(battle, combat_sounds),
        ),
        layers: layers.clone(),
    }))
}
