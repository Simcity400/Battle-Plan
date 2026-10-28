//! Every content reader over one gathered stack, with every diagnostic it
//! gives: each installed-content reader called as its consumers call it, every
//! channel it reports through (returned warnings, refusal fields, per-entry
//! refusals, a refused read), and the viewer's and dashboard's content
//! preparation on top (the combat content, the scripted text content, the
//! state neighbours and resource bonuses the trigger environment reads).

use std::collections::BTreeSet;
#[cfg(test)]
use std::path::PathBuf;
use std::{
    collections::BTreeMap,
    fmt::{Debug, Display},
    io,
    sync::Arc,
    time::Instant,
};
use viewer_combat as combat;
use viewer_foundation::map::sprites::Interface;
use viewer_foundation::map::text::load;

use installed_content_core::{
    ResolvedLayers, content::ResolvedContent, defines::Defines, gui::native::Cause,
    gui::native::Refusal,
};

use super::report::{Diagnostic, Kind, ReaderRun};

/// The readers' runs and diagnostics, in call order.
#[derive(Default)]
struct Log {
    runs: Vec<ReaderRun>,
    diagnostics: Vec<Diagnostic>,
    state_history: super::report::StateHistory,
    values: Option<super::values::Writer>,
}

fn read_states(
    log: &mut Log,
    read: impl FnOnce() -> io::Result<(
        installed_content_states::states::States,
        Vec<installed_content_core::Diagnostic>,
    )>,
) -> Option<installed_content_states::states::States> {
    let diagnostic_start = log.diagnostics.len();
    let states = log.read("states", read).map(|(states, notes)| {
        log.typed("states", notes);
        for &id in states.definitions().keys() {
            let reasons = states.diagnostics(id);
            if !reasons.is_empty() {
                log.state_history
                    .states
                    .push(super::report::StateHistoryState {
                        id,
                        file: states.source(id).map(ToString::to_string),
                        reasons: reasons.to_vec(),
                    });
            }
        }
        states
    });
    log.state_history
        .notes
        .extend(log.diagnostics[diagnostic_start..].iter().map(Into::into));
    states
}

impl Log {
    /// Runs one reader and records its time; a refused read is a
    /// [`Kind::ReaderError`] and gives `None`.
    fn read<T: Debug>(&mut self, reader: &str, read: impl FnOnce() -> io::Result<T>) -> Option<T> {
        let started = Instant::now();
        let result = read();
        self.runs.push(ReaderRun {
            reader: reader.to_owned(),
            milliseconds: started.elapsed().as_millis(),
            completed: result.is_ok(),
            not_run: None,
            note: None,
        });
        if self.values.is_some() {
            self.value(reader, &result.as_ref().map_err(|error| error.to_string()));
        }
        result
            .inspect_err(|error| {
                self.diagnostics.push(Diagnostic::new(
                    reader,
                    Kind::ReaderError,
                    error.to_string(),
                ))
            })
            .ok()
    }

    fn value(&self, reader: &str, value: &impl Debug) {
        if let Some(writer) = &self.values {
            writer.write(reader, value);
        }
    }

    /// Records `readers` as not run: `needs` refused the stack.
    fn not_run(&mut self, readers: &[&str], needs: &str) {
        for reader in readers {
            self.runs.push(ReaderRun {
                reader: (*reader).to_owned(),
                milliseconds: 0,
                completed: false,
                not_run: Some(format!("needs {needs}")),
                note: None,
            });
        }
    }

    fn note(&mut self, reader: &str, kind: Kind, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::new(reader, kind, message.into()));
    }

    fn about(&mut self, reader: &str, kind: Kind, subject: impl Display, why: &str) {
        self.diagnostics
            .push(Diagnostic::about(reader, kind, &subject.to_string(), why));
    }

    /// Each message of a channel with the producer's declared kind.
    fn notes<'a>(
        &mut self,
        reader: &str,
        kind: Kind,
        messages: impl IntoIterator<Item = &'a String>,
    ) {
        for message in messages {
            self.note(reader, kind, message.clone());
        }
    }

    fn typed(
        &mut self,
        reader: &str,
        messages: impl IntoIterator<Item = installed_content_core::Diagnostic>,
    ) {
        for (kind, message) in messages {
            self.note(reader, kind.into(), message);
        }
    }
}

fn raid_type_notes(log: &mut Log, raid: &installed_content_raids::raids::RaidType) {
    let subject = format!("raid {}", raid.name);
    if let Some(why) = &raid.tooltip.refused {
        log.about("raids", Kind::Refused, &subject, why);
    }
}

/// A native GUI reader's refusal in `file`: a walk the game does not define
/// leaves everything after it unknown; a value of the reading client's
/// session the corpus does not have is only a warning.
fn gui_refusal(log: &mut Log, reader: &str, file: &str, refusal: &Refusal) {
    let kind = match refusal.cause {
        Cause::Unread => Kind::Refused,
        Cause::Unaligned => Kind::Unsettled,
        Cause::Unrecorded => Kind::Warning,
    };
    log.diagnostics.push(Diagnostic::at(
        reader,
        kind,
        file,
        None,
        &refusal.to_string(),
    ));
}

/// Only the defines and the map audio's readers ([`sounds`]).
pub fn run_sounds(
    content: installed_content_core::GatheredContent,
    values: Option<super::values::Writer>,
) -> (Vec<ReaderRun>, Vec<Diagnostic>, super::report::StateHistory) {
    let mut log = Log {
        values,
        ..Log::default()
    };
    let layers = &content.layers;
    let defines = log.read("defines", || installed_content_core::defines::read(layers));
    sounds(&mut log, layers, defines.as_ref());
    (log.runs, log.diagnostics, log.state_history)
}

/// Every reader over `content`; `decode` is the reading client's texture
/// decode state, which the font pages and map texture files load under.
pub fn run(
    content: installed_content_core::GatheredContent,
    decode: installed_content_image::native::Decode,
    values: Option<super::values::Writer>,
) -> (Vec<ReaderRun>, Vec<Diagnostic>, super::report::StateHistory) {
    let language = content
        .language
        .strip_prefix("l_")
        .unwrap_or(&content.language)
        .to_owned();
    let resolved = Arc::new(ResolvedContent {
        layers: content.layers,
        stats: content.stats,
        changed: content.changed,
    });
    let mut log = Log {
        values,
        ..Log::default()
    };
    let layers: &ResolvedLayers = &resolved.layers;
    let defines = log.read("defines", || installed_content_core::defines::read(layers));
    // The map camera's zoom law and height limits, as the window reads them from the stack.
    match &defines {
        Some(defines) => {
            log.read("viewer.zoom_defines", || {
                viewer_foundation::map::camera::motion::CameraLaw::content_defines(defines)
            });
            log.read("viewer.camera_heights", || {
                Ok([
                    defines.number("NFrontend", "CAMERA_MIN_HEIGHT")?,
                    defines.number("NFrontend", "CAMERA_MAX_HEIGHT")?,
                ])
            });
        }
        None => log.not_run(&["viewer.zoom_defines", "viewer.camera_heights"], "defines"),
    }
    let (interface, localisation) = interface(&mut log, layers, &language, decode);
    let map = map(&mut log, layers, defines.as_ref(), decode);
    politics(&mut log, layers, defines.as_ref());
    units(
        &mut log,
        layers,
        defines.as_ref(),
        &map,
        decode,
        localisation.as_ref(),
    );
    viewer_map_assets(
        &mut log,
        layers,
        defines.as_ref(),
        &map,
        decode,
        interface.as_ref().map(|interface| &interface.registry),
    );
    triggers(&mut log, layers, defines.as_ref());
    sounds(&mut log, layers, defines.as_ref());
    let counts = match (&map.buildings, &map.ideologies, &map.resources) {
        (Some(buildings), Some((ideologies, _)), Some((resources, _))) => {
            let names = installed_content_core::trigger_keys::CountTriggerNames::new(
                buildings, ideologies, resources,
            );
            log.value("trigger_keys", &names);
            if let Some(gap) = &names.ideologies_gap {
                log.note("trigger_keys", Kind::Refused, gap.clone());
            }
            Some((buildings, ideologies, resources))
        }
        _ => None,
    };
    // The viewer's combat content, completed with the count trigger names
    // as the document completes it.
    match (&defines, counts) {
        (Some(defines), Some((buildings, ideologies, resources))) => {
            if let Some(mut combat) = log.read("viewer.combat", || {
                combat::CombatContent::read(layers, defines)
            }) {
                combat.set_count_trigger_names(buildings, ideologies, resources);
                log.value("viewer.combat", &combat);
                let warnings: Vec<String> = combat.warnings().collect();
                log.notes("viewer.combat", Kind::Warning, &warnings);
                let refusals: Vec<String> = combat.refusals().map(str::to_owned).collect();
                log.notes("viewer.combat", Kind::Refused, &refusals);
            }
        }
        _ => log.not_run(
            &["viewer.combat"],
            "defines, buildings, ideologies and resources",
        ),
    }
    log.read("dashboard.scripted_content", || {
        dashboard::scripted_text::ScriptedContent::read(Arc::clone(&resolved), language.clone())
            .map(|content| {
                (
                    content.names,
                    content.recursion,
                    content.tag_aliases,
                    content.recorded_language,
                )
            })
            .map_err(io::Error::other)
    });
    for product in dashboard::content_check::check(Arc::clone(&resolved), &language) {
        let reader = format!("dashboard.{}", product.key);
        log.runs.push(ReaderRun {
            reader: reader.clone(),
            milliseconds: product.elapsed.as_millis(),
            completed: product.error.is_none(),
            not_run: None,
            note: Some(product.note),
        });
        if let Some(error) = product.error {
            log.note(&reader, Kind::ReaderError, error);
        }
        for finding in product.findings {
            let mut diagnostic = Diagnostic::about(
                &reader,
                finding.kind.into(),
                &finding.subject,
                &finding.message,
            );
            diagnostic.count = finding.count;
            diagnostic.examples = finding.examples;
            if finding.file.is_some() {
                diagnostic.file = finding.file;
            }
            if finding.line.is_some() {
                diagnostic.line = finding.line;
            }
            log.diagnostics.push(diagnostic);
        }
    }
    (log.runs, log.diagnostics, log.state_history)
}

/// Localisation, interface resources, GUI files, model and particle descriptors.
fn interface(
    log: &mut Log,
    layers: &ResolvedLayers,
    language: &str,
    decode: installed_content_image::native::Decode,
) -> (
    Option<Interface>,
    Option<installed_content_localisation::localisation::Localisation>,
) {
    let localisation = log.read("localisation", || {
        installed_content_localisation::localisation::read(layers, language)
    });
    if let Some(localisation) = &localisation {
        for message in &localisation.logged {
            log.note("localisation", Kind::Logged, message.clone());
        }
        for message in &localisation.refused {
            log.note("localisation", Kind::Refused, message.clone());
        }
        for duplicate in &localisation.duplicates {
            let (key, files) = duplicate.split_once(' ').unwrap_or((duplicate, ""));
            log.about(
                "localisation",
                Kind::Warning,
                key,
                &format!("duplicate key (kept, ignored): {files}"),
            );
        }
    }
    // The native GUI resources: every `.gfx` file's sprites and font
    // declarations as the game reads them, then the sprite set the viewer
    // and dashboard look sprites up in, and every registered font as the
    // viewer loads it (viewer_foundation::map::text::load) for the language.
    let interface = log
        .read("gui.resources", || {
            installed_content_core::gui::native::resources::Registry::load(layers)
        })
        .map(|registry| Interface {
            sprites: installed_content_gfx::gfx::SpriteSet::new(&registry.files),
            registry,
            decode,
        });
    match &interface {
        Some(interface) => {
            log.value("gfx", &interface.sprites);
            for (file, sprites) in &interface.registry.files {
                let fonts = sprites.fonts.iter().filter_map(|event| match event {
                    installed_content_core::gui::native::fonts::FontEvent::Font(facts) => Some(facts),
                    installed_content_core::gui::native::fonts::FontEvent::Override(_) => None,
                });
                for refusal in sprites
                    .refusals
                    .iter()
                    .chain(sprites.sprites.iter().flat_map(|sprite| &sprite.refusals))
                    .chain(fonts.flat_map(|facts| &facts.refusals))
                {
                    gui_refusal(log, "gui.resources", file, refusal);
                }
            }
            log.notes("gfx", Kind::Refused, &interface.sprites.refused);
            let names: BTreeSet<String> = interface
                .registry
                .fonts
                .fonts()
                .iter()
                .map(|font| String::from_utf8_lossy(&font.facts.name).into_owned())
                .collect();
            let mut loaded = Vec::new();
            let values = log.values.clone();
            log.read("viewer.fonts", || {
                for name in &names {
                    let mut images = Vec::new();
                    let font = load(layers, interface, name, language, &mut images);
                    if let Some(writer) = &values {
                        writer.write("viewer.fonts", &(name, &images));
                    }
                    loaded.push((name.clone(), font));
                }
                Ok(())
            });
            for (name, font) in loaded {
                if let Some(writer) = &log.values {
                    writer.write(
                        "viewer.fonts",
                        &(
                            name.clone(),
                            font.as_ref().map_err(|error| error.to_string()),
                        ),
                    );
                }
                match font {
                    Ok(font) => log.notes("viewer.fonts", Kind::Logged, &font.logged),
                    Err(why) => log.about(
                        "viewer.fonts",
                        Kind::Refused,
                        format_args!("bitmapfont {name}"),
                        &why.to_string(),
                    ),
                }
            }
        }
        None => log.not_run(&["gfx", "viewer.fonts"], "gui.resources"),
    }
    if let Some(guis) = log.read("gui.native", || {
        installed_content_core::gui::native::read_layers(layers)
    }) {
        for (file, gui) in &guis.files {
            for refusal in gui.refusals() {
                gui_refusal(log, "gui.native", file, refusal);
            }
            for template in gui.templates() {
                for refusal in template.subtree_refusals() {
                    gui_refusal(log, "gui.native", file, refusal);
                }
            }
        }
    }
    if let Some(roster) = log.read("mesh.roster", || layers.descriptor_roster()) {
        log.notes("mesh.roster", Kind::Logged, &roster.logged);
        for unreadable in &roster.unreadable {
            log.note("mesh.roster", Kind::ParseFailure, unreadable.clone());
        }
    }
    log.read("mesh.texture_index", || layers.texture_index());
    match log.read("particles", || {
        installed_content_core::particles::Catalog::read(layers)
    }) {
        Some(catalog) => {
            log.read("weather", || {
                installed_content_weather::weather::effects(layers, &catalog)
            });
            // Every pdxparticle object's emitters as the viewer's particle
            // assets read them: each subsystem through RenderDescriptor::read.
            let mut refused = Vec::new();
            let values = log.values.clone();
            log.read("particles.emitters", || {
                for object in catalog.objects() {
                    let mut kind = Kind::Refused;
                    let read = catalog.resolve(object).and_then(|particle| {
                        if let Some(writer) = &values {
                            writer.write("particles.emitters", &(object, &particle));
                        }
                        let document =
                            installed_content_core::parser::parse(&particle.path.read_uncovered(layers)?)
                                .inspect_err(|_| kind = Kind::ParseFailure)?;
                        if let Some(writer) = &values {
                            writer.write("particles.emitters", &(object, &document));
                        }
                        let root = viewer_world::map::weather_effects::particle_root(
                            &document,
                            &particle.path.text(),
                            &particle.name,
                            viewer_world::map::weather_effects::Pick::First,
                        )?;
                        let mut textures = Vec::new();
                        let mut pages = BTreeMap::new();
                        let read = viewer_world::map::weather_effects::read_emitters(
                            layers,
                            decode,
                            root,
                            &mut textures,
                            &mut pages,
                        );
                        if let Some(writer) = &values {
                            writer.write("particles.emitters", &(object, &textures, &pages));
                        }
                        read.map_err(|error| {
                            io::Error::other(format!("{}: {error}", particle.path))
                        })
                    });
                    if let Some(writer) = &values {
                        writer.write(
                            "particles.emitters",
                            &(object, read.as_ref().map_err(|error| error.to_string())),
                        );
                    }
                    if let Err(error) = read {
                        refused.push((object.to_owned(), kind, error.to_string()));
                    }
                }
                Ok(())
            });
            for (object, kind, why) in refused {
                log.about(
                    "particles.emitters",
                    kind,
                    format_args!("pdxparticle {object}"),
                    &why,
                );
            }
        }
        None => log.not_run(&["weather", "particles.emitters"], "particles"),
    }
    if let Some(abilities) = log.read("abilities", || installed_content_abilities::abilities::read(layers))
    {
        log.typed("abilities", abilities.warnings);
    }
    log.read("ambient", || installed_content_core::ambient::read(layers));
    (interface, localisation)
}

/// What the map readers give the later ones.
struct Map {
    terrain: Option<(
        installed_content_core::terrain::TerrainDefinitions,
        installed_content_core::terrain::DefaultMap,
    )>,
    buildings: Option<installed_content_core::buildings::Buildings>,
    ideologies: Option<(
        installed_content_core::ideologies::Ideologies,
        Vec<installed_content_core::Diagnostic>,
    )>,
    resources: Option<(
        Vec<installed_content_core::resources::Resource>,
        Vec<installed_content_core::Diagnostic>,
    )>,
    data: Option<installed_content_core::map::MapData>,
    regions: Option<BTreeMap<u32, installed_content_regions::regions::StrategicRegion>>,
}

/// The map and what the viewer's document and the trigger environment read
/// with it.
fn map(
    log: &mut Log,
    layers: &ResolvedLayers,
    defines: Option<&Defines>,
    decode: installed_content_image::native::Decode,
) -> Map {
    let map = log.read("map", || installed_content_core::map::load(layers));
    log.read("map.country_colors", || {
        installed_content_core::map::read_country_colors(layers)
    });
    let terrain = log.read("terrain", || installed_content_core::terrain::read(layers));
    match &terrain {
        Some((definitions, default_map)) => {
            log.notes("terrain", Kind::Misread, &definitions.skipped);
            log.read("terrain.map_images", || {
                installed_content_core::terrain::read_map_images(layers, default_map, decode)
            });
        }
        None => log.not_run(&["terrain.map_images"], "terrain"),
    }
    let regions = log.read("regions", || installed_content_regions::regions::read(layers));
    if let Some(regions) = &regions {
        log.notes("regions", Kind::Warning, &regions.notes);
    }
    let regions = regions.map(|database| database.regions);
    log.read("adjacencies", || installed_content_core::adjacencies::read(layers));
    if let Some(rules) = log.read("adjacency_rules", || {
        installed_content_core::adjacency_rules::read(layers)
    }) {
        for rule in &rules {
            if let Some(why) = &rule.unsupported {
                log.about(
                    "adjacency_rules",
                    Kind::Unsupported,
                    format_args!("rule {}", rule.name),
                    why,
                );
            }
        }
    }
    log.read("cities", || installed_content_core::cities::read(layers));
    log.read("railways", || installed_content_core::railways::read(layers));
    log.read("positions.unit_stacks", || {
        installed_content_core::positions::read_unit_stacks(layers)
    });
    log.read("positions.buildings", || {
        installed_content_core::positions::read_buildings(layers)
    });
    let graph = log.read("province_graph", || {
        installed_content_core::province_graph::ProvinceGraph::read(layers)
    });
    if let Some((graph, _)) = &graph {
        for ((from, to), why) in &graph.unavailable {
            log.about(
                "province_graph",
                Kind::Refused,
                format_args!("edge {from} -> {to}"),
                why,
            );
        }
    }
    let buildings = log.read("buildings", || installed_content_core::buildings::read(layers));
    if let Some(buildings) = &buildings {
        for slot in &buildings.ambiguous_slots {
            log.about(
                "buildings",
                Kind::Refused,
                format_args!("slot {slot}"),
                "reached by a differing repeated declaration",
            );
        }
        for name in buildings.repeated_types.keys() {
            log.about(
                "buildings",
                Kind::Warning,
                format_args!("building type {name}"),
                "declared more than once",
            );
        }
        for (name, building) in &buildings.types {
            for why in [&building.tooltip_refused, &building.raid_refused]
                .into_iter()
                .flatten()
            {
                log.about("buildings", Kind::Refused, name, why);
            }
        }
    }
    let resources = log.read("resources", || installed_content_core::resources::read(layers));
    if let Some((_, warnings)) = &resources {
        log.typed("resources", warnings.clone());
    }
    let ideologies = log.read("ideologies", || installed_content_core::ideologies::read(layers));
    if let Some((_, warnings)) = &ideologies {
        log.typed("ideologies", warnings.clone());
    }
    if let Some(modes) = log.read("scripted_map_modes", || {
        installed_content_core::scripted_map_modes::read(layers)
    }) {
        for (file, field) in &modes.refused {
            log.about("scripted_map_modes", Kind::Refused, file, field);
        }
        // The viewer draws a layer that hands over to one of the game's layers; a colour
        // script is not evaluated.
        for mode in &modes.modes {
            for (side, layer) in [("top", &mode.top), ("bottom", &mode.bottom)] {
                use installed_content_core::scripted_map_modes::LayerType;
                if matches!(
                    layer.kind,
                    LayerType::Country | LayerType::State | LayerType::StateController
                ) {
                    log.about(
                        "scripted_map_modes",
                        Kind::Refused,
                        format_args!("{}.{side}", mode.name),
                        "colour script is not evaluated",
                    );
                }
            }
        }
    }
    const NEEDS_DEFINES: [&str; 4] = [
        "movement",
        "movement.defines",
        "dashboard.resource_bonuses",
        "states",
    ];
    let Some(defines) = defines else {
        log.not_run(&NEEDS_DEFINES, "defines");
        log.not_run(&["province_graph.neighbor_states"], "states");
        return Map {
            terrain,
            buildings,
            ideologies,
            resources,
            data: map,
            regions,
        };
    };
    if let Some(movement) = log.read("movement", || {
        installed_content_core::movement::MovementMap::read(layers, defines)
    }) {
        for ((from, to), why) in movement.unavailable_edges() {
            log.about(
                "movement",
                Kind::Refused,
                format_args!("edge {from} -> {to}"),
                why,
            );
        }
    }
    // The arrival estimates' organisation term and the railway and task-force domains
    // refuse independently of the common movement defines.
    if let Some(movement) = log.read("movement.defines", || {
        installed_content_core::movement::MovementDefines::read(defines)
    }) {
        for (part, refusal) in [
            ("organisation", movement.organisation.as_ref().err()),
            ("rail", movement.rail.as_ref().err()),
            ("naval", movement.naval.as_ref().err()),
        ] {
            if let Some(why) = refusal {
                log.about("movement.defines", Kind::Refused, part, why);
            }
        }
    }
    log.read("dashboard.resource_bonuses", || {
        dashboard::state_resources::resource_bonuses(defines)
    });
    let Some(data) = &map else {
        log.not_run(&["states", "province_graph.neighbor_states"], "map");
        return Map {
            terrain,
            buildings,
            ideologies,
            resources,
            data: None,
            regions,
        };
    };
    let states = read_states(log, || {
        installed_content_states::states::read(layers, defines, &data.definitions)
    });
    match states {
        Some(states) => {
            for (id, state) in states.definitions() {
                for why in [
                    state.name_key.as_ref().err(),
                    state.resources.as_ref().err(),
                ]
                .into_iter()
                .flatten()
                {
                    log.about("states", Kind::Refused, format_args!("state {id}"), why);
                }
                for key in &state.skipped_history {
                    log.about(
                        "states",
                        Kind::Warning,
                        format_args!("state {id}"),
                        &format!("history block {key} is skipped"),
                    );
                }
            }
            // The trigger environment's neighbour states
            // (dashboard::scripted_text, raids).
            match &graph {
                Some((graph, _)) => {
                    let members = states
                        .definitions()
                        .iter()
                        .map(|(&id, state)| (id, state.provinces.clone()))
                        .collect();
                    log.read("province_graph.neighbor_states", || {
                        graph.neighbor_states(&members)
                    });
                }
                None => log.not_run(&["province_graph.neighbor_states"], "province_graph"),
            }
        }
        None => log.not_run(&["province_graph.neighbor_states"], "states"),
    }
    Map {
        terrain,
        buildings,
        ideologies,
        resources,
        data: map,
        regions,
    }
}

/// Ideas, characters, traits, names, focus, technology and the modifier
/// definitions: the dashboard's politics and artwork content.
fn politics(log: &mut Log, layers: &ResolvedLayers, defines: Option<&Defines>) {
    if let Some((_, warnings)) = log.read("modifier_definitions", || {
        installed_content_core::modifier_definitions::read(layers)
    }) {
        log.typed("modifier_definitions", warnings);
    }
    // The officer corps groups (NCharacter.OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU)
    // the dashboard hands the idea reader.
    let officers = match defines {
        Some(defines) => log.read("ideas.officers", || {
            defines.strings("NCharacter", "OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU")
        }),
        None => {
            log.not_run(&["ideas.officers"], "defines");
            None
        }
    };
    let ideas = match officers {
        Some(officers) => log.read("ideas", || installed_content_core::ideas::read(layers, &officers)),
        None => {
            log.not_run(&["ideas"], "ideas.officers");
            None
        }
    };
    match ideas {
        Some((_, advisors, warnings)) => {
            log.typed("ideas", warnings);
            if let Some((_, warnings)) = log.read("characters", || {
                installed_content_core::characters::read(layers, advisors)
            }) {
                log.typed("characters", warnings);
            }
        }
        None => log.not_run(&["characters"], "ideas"),
    }
    // The viewer's counter portraits (army, large), file by file.
    let values = log.values.clone();
    if let Some(stops) = log.read("characters.portraits", || {
        let names = layers.key_names()?;
        installed_content_core::characters::paths(layers)
            .into_iter()
            .map(|path| {
                let (_, stop) = {
                    let read = installed_content_core::characters::portraits(
                        &path.read(layers)?,
                        &names,
                        "army",
                        "large",
                    )?;
                    if let Some(writer) = &values {
                        writer.write("characters.portraits", &(&path.to_string(), &read));
                    }
                    read
                };
                Ok(stop.map(|stop| format!("{path}: {stop}")))
            })
            .collect::<io::Result<Vec<_>>>()
    }) {
        log.notes(
            "characters.portraits",
            Kind::Unsettled,
            stops.iter().flatten(),
        );
    }
    if let Some(templates) = log.read("characters.leader_templates", || {
        installed_content_core::characters::leader_templates(layers).map_err(io::Error::other)
    }) {
        for template in &templates {
            if let Err(why) = &template.role {
                log.diagnostics.push(Diagnostic::at(
                    "characters.leader_templates",
                    Kind::Refused,
                    &template.path.text(),
                    template.line,
                    why,
                ));
            }
        }
    }
    if let Some((_, warnings)) = log.read("leader_traits", || {
        installed_content_core::leader_traits::read(layers)
    }) {
        log.typed("leader_traits", warnings);
    }
    if let Some(names) = log.read("names", || installed_content_core::names::read(layers)) {
        log.notes("names", Kind::Unsettled, &names.unknown);
    }
    if let Some((_, warnings)) = log.read("focus", || installed_content_focus::focus::read(layers)) {
        log.typed("focus", warnings);
    }
    if let Some(None) = log.read("focus.point_days", || {
        installed_content_focus::focus::point_days(layers)
    }) {
        log.note(
            "focus.point_days",
            Kind::Refused,
            "no positive literal focus-day define",
        );
    }
    if let Some((_, warnings)) = log.read("technology", || {
        installed_content_core::technology::read_equipment(layers)
    }) {
        log.typed("technology", warnings);
    }
}

/// Equipment, division names, unit leaders and sub-units, and the train
/// database.
fn units(
    log: &mut Log,
    layers: &ResolvedLayers,
    defines: Option<&Defines>,
    map: &Map,
    decode: installed_content_image::native::Decode,
    localisation: Option<&installed_content_localisation::localisation::Localisation>,
) {
    let equipment = log.read("equipment", || {
        installed_content_core::equipment::Database::load(layers)
    });
    if let Some(equipment) = &equipment {
        log.notes("equipment", Kind::Misread, &equipment.misreads);
        if let Some(lost) = &equipment.lost {
            log.note("equipment", Kind::Unsettled, lost.clone());
        }
    }
    model_labels(log, layers, equipment.as_ref(), localisation);
    if let Some(groups) = log.read("equipment.groups", || {
        installed_content_core::equipment::Groups::load(layers)
    }) {
        log.notes("equipment.groups", Kind::Misread, &groups.misreads);
        if let Some(lost) = &groups.lost {
            log.note("equipment.groups", Kind::Unsettled, lost.clone());
        }
    }
    if let Some((_, warnings)) = log.read("division_names", || {
        installed_content_division_names::division_names::read::<installed_content_core::Diagnostic>(layers)
    }) {
        log.typed("division_names", warnings);
    }
    match &map.terrain {
        Some((definitions, _)) => {
            // The commander content's terrain lookup: the
            // registered spelling of a type found without case.
            let names: Vec<String> = definitions
                .categories
                .iter()
                .map(|category| String::from_utf8_lossy(&category.name).into_owned())
                .collect();
            let spelling = |name: &str| {
                names
                    .iter()
                    .find(|known| known.eq_ignore_ascii_case(name))
                    .cloned()
            };
            if let Some(leaders) = log.read("unit_leader", || {
                installed_content_core::unit_leader::Leaders::<installed_content_core::Diagnostic>::load(
                    layers, &spelling,
                )
            }) {
                log.typed("unit_leader", leaders.misreads);
                log.notes(
                    "unit_leader",
                    Kind::Unsupported,
                    &leaders.traits.unsupported,
                );
                log.notes("unit_leader", Kind::Misread, leaders.skills.warnings());
                for condition in leaders.skills.conditions() {
                    log.note("unit_leader", Kind::Refused, condition.to_string());
                }
                for effects in leaders.traits.iter() {
                    let subject = format!("trait {}", effects.name);
                    for why in &effects.unsupported {
                        log.about("unit_leader", Kind::Unsupported, &subject, why);
                    }
                    for why in &effects.ignored {
                        log.about("unit_leader", Kind::Logged, &subject, why);
                    }
                    for condition in &effects.conditions {
                        log.about(
                            "unit_leader",
                            Kind::Refused,
                            &subject,
                            &condition.to_string(),
                        );
                    }
                }
            }
        }
        None => log.not_run(&["unit_leader"], "terrain"),
    }
    // The viewer's unit model content: model selection, equipment
    // graphics, geography, camouflage and ships.
    match (defines, &map.data, &map.terrain) {
        (Some(defines), Some(data), Some((_, default_map))) => {
            let mut warnings = Vec::new();
            let assets = log.read("viewer.unit_models", || {
                viewer_world::map::unit_models::UnitModelAssets::try_load(
                    layers,
                    decode,
                    defines,
                    data,
                    default_map,
                    &mut warnings,
                )
            });
            log.typed("viewer.unit_models", warnings);
            for (file, why) in assets.map(|assets| assets.refusals()).unwrap_or_default() {
                log.diagnostics.push(Diagnostic::split(
                    "viewer.unit_models",
                    Kind::Refused,
                    file.as_deref(),
                    &why,
                ));
            }
        }
        _ => log.not_run(&["viewer.unit_models"], "defines, map and terrain"),
    }
    if let Some(traits) = log.read("unit_leader.traits", || {
        installed_content_core::unit_leader::Traits::registered(layers)
    }) {
        log.notes("unit_leader.traits", Kind::Unsupported, &traits.unsupported);
    }
    if let Some(sub_units) = log.read("unit_leader.sub_units", || {
        installed_content_core::unit_leader::sub_units(layers)
    }) {
        log.notes("unit_leader.sub_units", Kind::Misread, sub_units.warnings());
    }
    // The train database with the registries the viewer's train assets give
    // it: the country tags, the graphical cultures and the entity roster.
    let cultures = log.read("train_gfx.cultures", || {
        installed_content_core::train_gfx::cultures(layers)
    });
    let tags = log.read("viewer.country_tags", || {
        viewer_world::map::trains::country_tag_index(layers)
    });
    let roster = layers.descriptor_roster();
    if log.values.is_some() {
        log.value(
            "mesh.roster",
            &roster.as_ref().map_err(|error| error.to_string()),
        );
    }
    match (cultures, tags, roster) {
        (Some(cultures), Some(tags), Ok(roster)) => {
            if let Some(warning) = tags.warning() {
                log.notes("viewer.country_tags", Kind::Warning, &[warning]);
            }
            let is_tag = |tag: &str| tags.files.contains_key(tag);
            let has_entity = |entity: &str| roster.descriptors.entities.contains_key(entity);
            if let Some(database) = log.read("train_gfx", || {
                installed_content_core::train_gfx::Database::read(
                    layers,
                    &installed_content_core::train_gfx::Registries {
                        is_tag: &is_tag,
                        cultures: &cultures,
                        has_entity: &has_entity,
                    },
                )
            }) {
                log.notes("train_gfx", Kind::Logged, &database.diagnostics);
            }
        }
        _ => log.not_run(
            &["train_gfx"],
            "train_gfx.cultures, viewer.country_tags and mesh.roster",
        ),
    }
}

/// The content the land-model fallback's option labels read: the dynamic
/// country tags, the display language's first-row label and each equipment
/// type's variant name keys. A key the language leaves undefined labels its
/// rows with the key's own text, as the game's localiser does.
fn model_labels(
    log: &mut Log,
    layers: &ResolvedLayers,
    equipment: Option<&installed_content_core::equipment::Database>,
    localisation: Option<&installed_content_localisation::localisation::Localisation>,
) {
    const READER: &str = "viewer.model_labels";
    let (Some(equipment), Some(localisation)) = (equipment, localisation) else {
        log.not_run(&[READER], "equipment and localisation");
        return;
    };
    let Some(tags) = log.read(READER, || {
        installed_content_core::country_tags::read_definitions(layers)
    }) else {
        return;
    };
    let dynamic: Vec<&String> = tags
        .iter()
        .filter(|(_, tag)| tag.dynamic)
        .map(|(name, _)| name)
        .collect();
    let keys: Vec<(String, String)> = equipment
        .equipment
        .iter()
        .flat_map(|kind| {
            [&kind.derived_variant_name, &kind.variant_name]
                .into_iter()
                .filter(|key| !key.is_empty())
                .map(|key| {
                    (
                        String::from_utf8_lossy(&kind.name).into_owned(),
                        String::from_utf8_lossy(key).into_owned(),
                    )
                })
        })
        .collect();
    log.value(READER, &(&dynamic, &keys));
    let first =
        std::str::from_utf8(viewer_world::map::model_labels::DEFAULT_MODEL_KEY).unwrap_or_default();
    if localisation.get(first).is_none() {
        log.about(
            READER,
            Kind::Fallback,
            first,
            "the display language does not define the option list's first row; it shows the key",
        );
    }
    for (kind, key) in &keys {
        if localisation.get(key).is_none() {
            log.about(
                READER,
                Kind::Fallback,
                format!("equipment {kind}"),
                &format!("variant name key {key} is not defined in the display language; unnamed variants show the key"),
            );
        }
    }
}

/// Map audio as the viewer resolves it: the sound definitions and `NSound`
/// defines, every entity event's sound effect, and every sample an effect can
/// choose. Samples a DLC or mod layer supplies are decoded; the base game's
/// are only looked up.
fn sounds(log: &mut Log, layers: &ResolvedLayers, defines: Option<&Defines>) {
    let Some(catalog) = log.read("viewer.sounds", || {
        installed_content_sound::sound::Catalog::read(layers)
    }) else {
        return;
    };
    for (message, count) in catalog.notes.entries() {
        for _ in 0..*count {
            log.note("viewer.sounds", Kind::Refused, message.clone());
        }
    }
    match defines {
        Some(defines) => {
            if let Err(error) = installed_content_sound::sound::Defines::load(defines) {
                log.note(
                    "viewer.sounds",
                    Kind::Refused,
                    format!("NSound defines: {error}"),
                );
            }
            // The battle ambience: its defines and distant effect.
            match installed_content_sound::combat_sounds::BattleDefines::load(defines) {
                Ok(battle) if catalog.effect(&battle.effect).is_none() => log.about(
                    "viewer.sounds",
                    Kind::Refused,
                    &battle.effect,
                    "NSound.BATTLE_SOUND_NAME names a sound effect no sound asset declares",
                ),
                Ok(_) => {}
                Err(error) => log.note(
                    "viewer.sounds",
                    Kind::Refused,
                    format!("NSound battle defines: {error}"),
                ),
            }
        }
        None => log.not_run(&["viewer.sounds.defines"], "defines"),
    }
    // The battle ambience layers of sound/combat_sounds.
    if let Some(combat_sounds) = log.read("viewer.battle_sounds", || {
        installed_content_sound::combat_sounds::Layers::read(layers)
    }) {
        for (message, count) in combat_sounds.notes.entries() {
            for _ in 0..*count {
                log.note("viewer.battle_sounds", Kind::Refused, message.clone());
            }
        }
        for sound in &combat_sounds.sounds {
            if catalog.effect(&sound.effect).is_none() {
                log.about(
                    "viewer.battle_sounds",
                    Kind::Refused,
                    &sound.source,
                    &format!(
                        "battle sound {} names sound effect {}, which no sound asset declares",
                        sound.name, sound.effect
                    ),
                );
            }
        }
    }
    if let Ok(roster) = layers.descriptor_roster() {
        let named: BTreeSet<&str> = roster
            .descriptors
            .entities
            .values()
            .flat_map(|entity| &entity.states)
            .flat_map(|state| &state.events)
            .filter_map(|event| event.sound.as_deref())
            .collect();
        for effect in named {
            if catalog.effect(effect).is_none() {
                log.about(
                    "viewer.sounds",
                    Kind::Refused,
                    effect,
                    "entity events name this sound effect, which no sound asset declares",
                );
            }
        }
    }
    let chosen: BTreeSet<usize> = catalog
        .effects
        .iter()
        .flat_map(|effect| &effect.choices)
        .map(|choice| choice.sound)
        .collect();
    let (mut found, mut decoded) = (0usize, 0usize);
    for sound in chosen {
        let sound = &catalog.sounds[sound];
        match layers.has_file(&sound.path) {
            Ok(true) => found += 1,
            Ok(false) => {
                log.about(
                    "viewer.sounds",
                    Kind::Refused,
                    &sound.path,
                    &format!("sound {} names a missing sample file", sound.name),
                );
                continue;
            }
            Err(error) => {
                log.note(
                    "viewer.sounds",
                    Kind::ReaderError,
                    format!("{}: {error}", sound.path),
                );
                continue;
            }
        }
        if layers.uncovered_from_base(&sound.path).unwrap_or(false) {
            continue;
        }
        match layers.read_uncovered(&sound.path) {
            Ok(bytes) => match installed_content_sound::wav::decode(&bytes) {
                Ok(_) => decoded += 1,
                Err(why) => log.about(
                    "viewer.sounds",
                    Kind::Unsupported,
                    &sound.path,
                    &why.to_string(),
                ),
            },
            Err(error) => log.note(
                "viewer.sounds",
                Kind::ReaderError,
                format!("{}: {error}", sound.path),
            ),
        }
    }
    // Music: every song's track is found, and a DLC or mod track opens and decodes.
    if let Some(music) = log.read("viewer.music", || {
        installed_content_sound::music::Music::read(layers)
    }) {
        for (message, count) in music.notes.entries() {
            for _ in 0..*count {
                log.note("viewer.music", Kind::Refused, message.clone());
            }
        }
        let tracks: BTreeSet<usize> = music.songs.iter().map(|song| song.track).collect();
        for track in tracks {
            let track = &music.tracks[track];
            match layers.has_file(&track.path) {
                Ok(true) => {}
                Ok(false) => {
                    log.about(
                        "viewer.music",
                        Kind::Refused,
                        &track.path,
                        &format!("music {} names a missing track file", track.name),
                    );
                    continue;
                }
                Err(error) => {
                    log.note(
                        "viewer.music",
                        Kind::ReaderError,
                        format!("{}: {error}", track.path),
                    );
                    continue;
                }
            }
            if layers.uncovered_from_base(&track.path).unwrap_or(false) {
                continue;
            }
            match layers.read_uncovered(&track.path) {
                Ok(bytes) => {
                    if let Err(why) = installed_content_sound::ogg::probe(bytes) {
                        log.about("viewer.music", Kind::Unsupported, &track.path, &why);
                    }
                }
                Err(error) => log.note(
                    "viewer.music",
                    Kind::ReaderError,
                    format!("{}: {error}", track.path),
                ),
            }
        }
        log.value("viewer.music", &(music.tracks.len(), music.songs.len()));
    }
    log.value(
        "viewer.sounds",
        &(
            catalog.files.len(),
            catalog.sounds.len(),
            catalog.effects.len(),
            found,
            decoded,
        ),
    );
}

/// The trigger readers, scripted localisation, combat tactics and raids.
fn triggers(log: &mut Log, layers: &ResolvedLayers, defines: Option<&Defines>) {
    if let Some(scripted) = log.read("scripted_triggers", || {
        installed_content_core::scripted_triggers::read(layers)
    }) {
        log.notes("scripted_triggers", Kind::Logged, &scripted.logged);
        for (name, template) in &scripted.templates {
            if let Err(why) = &template.body {
                log.about(
                    "scripted_triggers",
                    Kind::Refused,
                    String::from_utf8_lossy(name),
                    why,
                );
            }
        }
    }
    if let Some(aliases) = log.read("tag_aliases", || installed_content_core::tag_aliases::read(layers)) {
        log.notes("tag_aliases", Kind::Unsettled, &aliases.unknown);
    }
    if let Some(names) = log.read("scripted_localisation", || {
        installed_content_scripted_localisation::scripted_localisation::names(layers)
    }) {
        log.typed("scripted_localisation", names.warnings);
    }
    if let Some(database) = log.read("combat_tactics", || {
        installed_content_core::combat_tactics::read(layers)
    }) {
        log.notes("combat_tactics", Kind::Unsettled, &database.unsupported);
        log.notes("combat_tactics", Kind::Logged, &database.logged);
        for tactic in &database.tactics {
            if let Some(why) = &tactic.unproven {
                log.about(
                    "combat_tactics",
                    Kind::Refused,
                    format_args!("tactic {}", String::from_utf8_lossy(&tactic.token)),
                    why,
                );
            }
        }
    }
    if let Some(catalog) = log.read("raids", || installed_content_raids::raids::read(layers)) {
        log.typed("raids", catalog.misreads.iter().cloned());
        log.notes("raids", Kind::Refused, &catalog.refused);
        for (name, why) in &catalog.unsupported {
            log.about("raids", Kind::Unsupported, format_args!("raid {name}"), why);
        }
        for uncertain in &catalog.uncertain {
            log.about(
                "raids",
                uncertain.kind.into(),
                format_args!("raid {}", uncertain.name),
                &uncertain.reason,
            );
        }
        for raid in catalog.types.iter().chain(
            catalog
                .uncertain
                .iter()
                .flat_map(|uncertain| &uncertain.declarations),
        ) {
            raid_type_notes(log, raid);
        }
    }
    if let Some(groups) = log.read("raids.names", || {
        installed_content_raids::raids::names::Groups::read(layers)
    }) {
        // Each refused file names itself and its line in its warning; the
        // groups it declares or could declare are refused by name.
        log.typed("raids.names", groups.warnings.iter().cloned());
        for (name, why) in groups.refused() {
            log.about(
                "raids.names",
                Kind::Refused,
                format_args!("name group {name}"),
                why,
            );
        }
    }
    match defines {
        Some(defines) => {
            log.read("raids.graphics_defines", || {
                installed_content_raids::raids::RaidGraphicsDefines::read(defines)
            });
        }
        None => log.not_run(&["raids.graphics_defines"], "defines"),
    }
}

/// The viewer's other map content loaders that need no device: trains,
/// bridges and orders from the defines, aircraft, map buildings and raids
/// from the map too, each with its warnings and the refusals its prepared
/// content holds. A loader whose input another reader refused is not run,
/// by name.
fn viewer_map_assets(
    log: &mut Log,
    layers: &ResolvedLayers,
    defines: Option<&Defines>,
    map: &Map,
    decode: installed_content_image::native::Decode,
    registry: Option<&installed_content_core::gui::native::resources::Registry>,
) {
    let Some(defines) = defines else {
        log.not_run(
            &[
                "viewer.trains",
                "viewer.bridges",
                "viewer.orders",
                "viewer.aircraft",
                "viewer.map_buildings",
                "viewer.raids",
            ],
            "defines",
        );
        return;
    };
    let mut warnings = Vec::new();
    log.read("viewer.trains", || {
        viewer_world::map::trains::TrainAssets::load(layers, decode, defines, &mut warnings)
    });
    log.typed("viewer.trains", warnings);
    let mut warnings = Vec::new();
    log.read("viewer.bridges", || {
        viewer_world::map::railways::Bridges::load(layers, decode, defines, &mut warnings)
    });
    log.typed("viewer.bridges", warnings);
    let mut warnings = Vec::new();
    if let Some(orders) = log.read("viewer.orders", || {
        viewer_foundation::map::ribbon_content::OrderAssets::load(
            layers,
            decode,
            (defines, defines),
            &mut warnings,
        )
    }) && let Err(why) = &orders.raid_defines
    {
        log.note("viewer.orders", Kind::Refused, why.clone());
    }
    log.typed("viewer.orders", warnings);
    let Some(data) = &map.data else {
        log.not_run(
            &["viewer.aircraft", "viewer.map_buildings", "viewer.raids"],
            "map",
        );
        return;
    };
    match &map.regions {
        Some(regions) => {
            let mut warnings = Vec::new();
            if let Some(aircraft) = log.read("viewer.aircraft", || {
                viewer_world::map::aircraft::AircraftAssets::load(
                    layers,
                    decode,
                    data,
                    (defines, defines),
                    regions,
                    &mut warnings,
                )
            }) {
                for why in aircraft.refusals() {
                    log.diagnostics.push(Diagnostic::split(
                        "viewer.aircraft",
                        Kind::Refused,
                        None,
                        &why,
                    ));
                }
            }
            log.typed("viewer.aircraft", warnings);
        }
        None => log.not_run(&["viewer.aircraft"], "regions"),
    }
    let mut warnings = Vec::new();
    log.read("viewer.map_buildings", || {
        viewer_world::map::map_buildings::MapBuildingAssets::load(
            layers,
            decode,
            data,
            defines,
            &mut warnings,
        )
    });
    log.typed("viewer.map_buildings", warnings);
    let Some(registry) = registry else {
        log.not_run(&["viewer.raids", "viewer.map_text"], "gui.resources");
        return;
    };
    // The floating map text's two types, their fonts' shader and its cutoff.
    if let Some(refusals) = log.read("viewer.map_text", || {
        Ok(viewer_world::map::animated_map_text::Assets::load(
            layers,
            (defines, defines),
            [data.width, data.height],
            decode,
            &registry.files,
        )
        .refusals())
    }) {
        log.notes("viewer.map_text", Kind::Refused, &refusals);
    }
    if let Some(raids) = log.read("viewer.raids", || {
        viewer_raids::map::raids::RaidAssets::load(layers, defines, data, registry)
    }) {
        log.typed("viewer.raids", raids.warnings.iter().cloned());
        log.notes("viewer.raids", Kind::Refused, &raids.refusals());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content_raids::raids::RaidType;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn state_history_uses_kept_state_sources_and_diagnostics_from_the_same_read() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_corpus_states_{}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(root);
        for (path, text) in [
            (
                "01-kept.txt",
                "state = { id = 1 impassable = maybe } state = { id = 2 }",
            ),
            (
                "02-filewide.txt",
                "@a bad orphan = yes state = { id = 3 } state = { id = 4 }",
            ),
            (
                "03-discarded.txt",
                "state = { id = 2 state_startup_effect = { free_operative = { operative = authored } } }",
            ),
            ("04-empty.txt", "@a bad orphan = yes"),
        ] {
            let path = fixture.0.join("history/states").join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&fixture.0, &[][..])]).unwrap();
        let defines = Defines::new().unwrap();
        defines
            .load(
                "fixture.lua",
                b"NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 15 } }",
            )
            .unwrap();
        let mut reads = 0;
        let mut log = Log::default();
        let states = read_states(&mut log, || {
            reads += 1;
            installed_content_states::states::read(&layers, &defines, &BTreeMap::new())
        })
        .unwrap();
        assert_eq!(reads, 1);
        assert_eq!(
            log.state_history
                .states
                .iter()
                .map(|state| state.id)
                .collect::<Vec<_>>(),
            [1, 3, 4]
        );
        for state in &log.state_history.states {
            assert_eq!(
                state.file,
                states.source(state.id).map(ToString::to_string)
            );
            assert_eq!(state.reasons, states.diagnostics(state.id));
        }
        assert!(states.diagnostics(2).is_empty());
        assert!(
            log.state_history
                .notes
                .iter()
                .any(|note| note.message.contains("defined by both"))
        );
        assert!(
            log.state_history
                .notes
                .iter()
                .any(|note| note.message.contains("04-empty.txt"))
        );
        assert!(
            log.state_history
                .notes
                .iter()
                .any(|note| note.message.contains("03-discarded.txt")
                    && note.message.contains("Deferred effect"))
        );
        assert_eq!(log.state_history.notes.len(), log.diagnostics.len());
        let mut refused = Log::default();
        assert!(
            read_states(&mut refused, || Err(io::Error::other(
                "history/states/refused.txt: line 8: reader stopped"
            )))
            .is_none()
        );
        assert!(refused.state_history.states.is_empty());
        assert_eq!(refused.state_history.notes.len(), 1);
        assert_eq!(refused.state_history.notes[0].kind, Kind::ReaderError);
        assert_eq!(refused.state_history.notes[0].line, Some(8));
    }

    #[test]
    fn counted_effect_warnings_reach_the_corpus_report() {
        let warning = "common/national_focus/gap.txt: line 3: Deferred effect `synthetic_effect`: value reading is unavailable; by-key reading retained (count: 2)".to_owned();
        let mut log = Log::default();
        log.typed(
            "focus",
            [(installed_content_core::DiagnosticKind::Unsupported, warning)],
        );
        assert_eq!(log.diagnostics.len(), 1);
        let note = &log.diagnostics[0];
        assert_eq!(note.kind, Kind::Unsupported);
        assert_eq!(note.file.as_deref(), Some("common/national_focus/gap.txt"));
        assert_eq!(note.line, Some(3));
        assert!(note.message.contains("by-key reading retained"));
        assert!(note.message.contains("count: 2"));
    }

    #[test]
    fn raid_reader_notes_reach_the_corpus_once_with_their_producer_kinds() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_corpus_raids_{}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(root);
        let path = fixture.0.join("common/raids/fixture.txt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            "types = {
                kept = { unit_model = { type = x } success_levels = { success = { actor_effects = { free_operative = { operative = x } } } } }
                undecided = { unit_requirements = { battalion_types = { \"authored\" = { min = 1 } } } }
            }",
        )
        .unwrap();
        let buildings = fixture.0.join("common/buildings/fixture.txt");
        fs::create_dir_all(buildings.parent().unwrap()).unwrap();
        fs::write(buildings, "buildings = { mod_building = {} }").unwrap();
        let layers = ResolvedLayers::from_folders(&[(&fixture.0, &[][..])]).unwrap();
        let catalog = installed_content_raids::raids::read(&layers).unwrap();
        assert!(
            catalog
                .misreads
                .iter()
                .any(|(kind, _)| *kind == installed_content_core::DiagnosticKind::Misread),
            "{catalog:?}"
        );
        assert!(
            catalog
                .misreads
                .iter()
                .any(|(kind, _)| *kind == installed_content_core::DiagnosticKind::Unsupported),
            "{catalog:?}"
        );
        assert!(!catalog.uncertain.is_empty(), "{catalog:?}");
        assert_eq!(
            catalog.uncertain[0].kind,
            installed_content_core::DiagnosticKind::Unsupported
        );
        let mut log = Log::default();
        triggers(&mut log, &layers, None);
        for (kind, message) in &catalog.misreads {
            let found: Vec<_> = log
                .diagnostics
                .iter()
                .filter(|note| note.reader == "raids" && note.message == *message)
                .collect();
            assert_eq!(found.len(), 1, "{message}: {found:?}");
            assert_eq!(found[0].kind, (*kind).into());
        }
        for uncertain in &catalog.uncertain {
            let message = format!("raid {}: {}", uncertain.name, uncertain.reason);
            let found: Vec<_> = log
                .diagnostics
                .iter()
                .filter(|note| note.reader == "raids" && note.message == message)
                .collect();
            assert_eq!(found.len(), 1, "{message}: {found:?}");
            assert_eq!(found[0].kind, uncertain.kind.into());
        }
    }

    #[test]
    fn warning_words_do_not_change_kind() {
        let mut log = Log::default();
        log.notes(
            "viewer.unit_models",
            Kind::Warning,
            &[
                "entity E is not registered".to_owned(),
                "geometry-less entities".to_owned(),
            ],
        );
        assert!(
            log.diagnostics
                .iter()
                .all(|note| note.kind == Kind::Warning)
        );
        for (producer, message, expected) in [
            (
                installed_content_core::DiagnosticKind::Logged,
                "unsupported colour",
                Kind::Logged,
            ),
            (
                installed_content_core::DiagnosticKind::Warning,
                "file cannot be followed",
                Kind::Warning,
            ),
            (
                installed_content_core::DiagnosticKind::ParseFailure,
                "unknown form",
                Kind::ParseFailure,
            ),
            (
                installed_content_core::DiagnosticKind::Fallback,
                "shown by its script name",
                Kind::Fallback,
            ),
            (
                installed_content_core::DiagnosticKind::Ambiguous,
                "declared more than once with differing fields",
                Kind::Ambiguous,
            ),
        ] {
            log.typed("reader", [(producer, message.to_owned())]);
            assert_eq!(log.diagnostics.last().unwrap().kind, expected);
        }
    }

    #[test]
    fn raids_are_not_run_without_the_interface_registry() {
        let layers = ResolvedLayers::from_folders(&[]).unwrap();
        let defines = installed_content_core::defines::read(&layers).unwrap();
        let map = Map {
            terrain: None,
            buildings: None,
            ideologies: None,
            resources: None,
            regions: None,
            data: Some(installed_content_core::map::MapData {
                width: 1,
                height: 1,
                province_ids: vec![0],
                definitions: Default::default(),
                country_colors: Default::default(),
            }),
        };
        let mut log = Log::default();
        viewer_map_assets(
            &mut log,
            &layers,
            Some(&defines),
            &map,
            Default::default(),
            None,
        );
        let raids = log
            .runs
            .iter()
            .find(|run| run.reader == "viewer.raids")
            .unwrap();
        assert_eq!(raids.not_run.as_deref(), Some("needs gui.resources"));
        assert!(!raids.completed);
        assert!(
            log.diagnostics
                .iter()
                .all(|note| note.reader != "viewer.raids")
        );
    }

    #[test]
    fn raid_tooltip_kinds_follow_their_fields() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut raid = RaidType {
            name: "odd".into(),
            ..Default::default()
        };
        raid.tooltip.refused = Some("tooltip \"x\" \\ é".into());
        let mut log = Log::default();
        raid_type_notes(&mut log, &raid);
        assert_eq!(
            log.diagnostics
                .iter()
                .map(|note| (note.kind, note.message.as_str()))
                .collect::<Vec<_>>(),
            [(Kind::Refused, "raid odd: tooltip \"x\" \\ é")]
        );
    }
}
