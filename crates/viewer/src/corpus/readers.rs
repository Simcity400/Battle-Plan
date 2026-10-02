//! Every content reader over one gathered stack, with every diagnostic it
//! gives: each installed-content reader called as its consumers call it, every
//! channel it reports through (returned warnings, refusal fields, per-entry
//! refusals, a refused read), and the viewer's and dashboard's content
//! preparation on top (the combat content, the scripted text content, the
//! state neighbours and resource bonuses the trigger environment reads).

use std::{
    collections::BTreeMap,
    fmt::{Debug, Display},
    io,
    sync::Arc,
    time::Instant,
};

use installed_content::{
    ResolvedLayers,
    content::ResolvedContent,
    defines::Defines,
    gui::native::{Cause, Refusal},
};

use super::report::{Diagnostic, Kind, ReaderRun};

/// The readers' runs and diagnostics, in call order.
#[derive(Default)]
struct Log {
    runs: Vec<ReaderRun>,
    diagnostics: Vec<Diagnostic>,
    values: Option<super::values::Writer>,
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

    /// Each message of a channel that mixes kinds, classified by its words
    /// ([`classify`]), else `default`.
    fn notes<'a>(
        &mut self,
        reader: &str,
        default: Kind,
        messages: impl IntoIterator<Item = &'a String>,
    ) {
        for message in messages {
            self.note(reader, classify(message, default), message.clone());
        }
    }
}

/// The kind a mixed channel's message names in the readers' own wording:
/// a tree parse failure, where a file's reading stops short, where it
/// leaves the text, or a refusal.
fn classify(message: &str, default: Kind) -> Kind {
    const UNSETTLED: [&str; 7] = [
        "cannot be followed",
        "cannot be determined",
        "is unknown from",
        "the rest of the file",
        "everything after it",
        "unavailable in",
        "cannot settle",
    ];
    const MISREAD: [&str; 4] = [
        "read as the game reads it",
        "follows the game's reading",
        "follow the game's reading",
        "leaves the text",
    ];
    // The tree parser's error (parser.rs `invalid`) and its repair of a file
    // that ends inside a block.
    if message.contains("Clausewitz text at byte") || message.contains("ends inside an open block")
    {
        Kind::ParseFailure
    } else if UNSETTLED.iter().any(|words| message.contains(words)) {
        Kind::Unsettled
    } else if message.contains("Deferred effect `") {
        Kind::Unsupported
    } else if MISREAD.iter().any(|words| message.contains(words)) {
        Kind::Misread
    } else if message.contains("refused") || message.contains("not registered") {
        Kind::Refused
    } else {
        default
    }
}

/// The `Debug` string literal starting at `body` (after its opening
/// quote), its escapes undone, and where it ends.
fn debug_string(text: &str, body: usize) -> (String, usize) {
    let mut out = String::new();
    let mut chars = text[body..].char_indices();
    while let Some((offset, c)) = chars.next() {
        match c {
            '"' => return (out, body + offset + 1),
            '\\' => match chars.next().map(|(_, c)| c) {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('0') => out.push('\0'),
                Some('u') => {
                    let code: String = chars
                        .by_ref()
                        .map(|(_, c)| c)
                        .skip(1)
                        .take_while(|&c| c != '}')
                        .collect();
                    out.extend(u32::from_str_radix(&code, 16).ok().and_then(char::from_u32));
                }
                Some(other) => out.push(other),
                None => {}
            },
            other => out.push(other),
        }
    }
    (out, text.len())
}

/// Every refusal a reader's value holds, found in its `Debug` text by the
/// names the readers give them: an `Unsupported`, `Unknown` or `Err` text,
/// and a `refused`, `unsupported` or `lost` field that holds one. A value
/// whose type grows another refusal-bearing field of those names is covered
/// without a change here.
fn debug_refusals(value: &impl std::fmt::Debug) -> Vec<String> {
    const MARKERS: [&str; 6] = [
        "Unsupported(\"",
        "Unknown(\"",
        "Err(\"",
        "refused: Some(\"",
        "unsupported: Some(\"",
        "lost: Some(\"",
    ];
    let text = format!("{value:?}");
    let mut found = Vec::new();
    let mut at = 0;
    while let Some((start, marker)) = MARKERS
        .iter()
        .filter_map(|marker| Some((text[at..].find(marker)? + at, marker)))
        .min()
    {
        let (string, end) = debug_string(&text, start + marker.len());
        found.push(string);
        at = end;
    }
    found
}

/// The texts of every `notes` list a reader's value holds, found in its
/// `Debug` text, so a reader that grows one (as the strategic regions'
/// deletions and replacements do) is covered without a change here.
fn debug_notes(value: &impl std::fmt::Debug) -> Vec<String> {
    const MARKER: &str = "notes: [";
    let text = format!("{value:?}");
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(start) = text[at..].find(MARKER) {
        at += start + MARKER.len();
        while text[at..].starts_with('"') {
            let (string, end) = debug_string(&text, at + 1);
            found.push(string);
            at = end;
            if text[at..].starts_with(", ") {
                at += 2;
            }
        }
    }
    found
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

/// Every reader over `content`; `decode` is the reading client's texture
/// decode state, which the font pages and map texture files load under.
pub fn run(
    content: installed_content::GatheredContent,
    decode: installed_content::image::native::Decode,
    values: Option<super::values::Writer>,
) -> (Vec<ReaderRun>, Vec<Diagnostic>) {
    let language = content
        .language
        .strip_prefix("l_")
        .unwrap_or(&content.language)
        .to_owned();
    let resolved = Arc::new(ResolvedContent {
        layers: content.layers,
        stats: content.stats,
    });
    let mut log = Log {
        values,
        ..Log::default()
    };
    let layers: &ResolvedLayers = &resolved.layers;
    let defines = log.read("defines", || installed_content::defines::read(layers));
    let interface = interface(&mut log, layers, &language, decode);
    let map = map(&mut log, layers, defines.as_ref(), decode);
    politics(&mut log, layers, defines.as_ref());
    units(&mut log, layers, defines.as_ref(), &map, decode);
    viewer_map_assets(
        &mut log,
        layers,
        defines.as_ref(),
        &map,
        decode,
        interface.as_ref().map(|interface| &interface.registry),
    );
    triggers(&mut log, layers, defines.as_ref());
    let counts = match (&map.buildings, &map.ideologies, &map.resources) {
        (Some(buildings), Some((ideologies, _)), Some((resources, _))) => {
            let names =
                installed_content::trigger_keys::CountTriggerNames::new(buildings, ideologies, resources);
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
                crate::combat::CombatContent::read(layers, defines)
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
    (log.runs, log.diagnostics)
}

/// Localisation, interface resources, GUI files, model and particle descriptors.
fn interface(
    log: &mut Log,
    layers: &ResolvedLayers,
    language: &str,
    decode: installed_content::image::native::Decode,
) -> Option<crate::map::sprites::Interface> {
    if let Some(localisation) = log.read("localisation", || {
        installed_content::localisation::read(layers, language)
    }) {
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
    // viewer loads it (map::text::load) for the language.
    let interface = log
        .read("gui.resources", || {
            installed_content::gui::native::resources::Registry::load(layers)
        })
        .map(|registry| crate::map::sprites::Interface {
            sprites: installed_content::gfx::SpriteSet::new(&registry.files),
            registry,
            decode,
        });
    match &interface {
        Some(interface) => {
            log.value("gfx", &interface.sprites);
            for (file, sprites) in &interface.registry.files {
                let fonts = sprites.fonts.iter().filter_map(|event| match event {
                    installed_content::gui::native::fonts::FontEvent::Font(facts) => Some(facts),
                    installed_content::gui::native::fonts::FontEvent::Override(_) => None,
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
            let names: std::collections::BTreeSet<String> = interface
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
                    let font =
                        crate::map::text::load(layers, interface, name, language, &mut images);
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
    if let Some(guis) = log.read("gui.native", || installed_content::gui::native::read_layers(layers)) {
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
    // The tree reader retained for combat windows and the remaining map icons.
    let values = log.values.clone();
    if let Some(failed) = log.read("gui.tree", || {
        Ok(layers
            .descriptor_paths("interface", &[".gui"])?
            .into_iter()
            .filter_map(|path| {
                let read = installed_content::gui::read(layers, &path);
                if let Some(writer) = &values {
                    writer.write(
                        "gui.tree",
                        &(&path, read.as_ref().map_err(|error| error.to_string())),
                    );
                }
                let error = read.err()?;
                Some((path, error.to_string()))
            })
            .collect::<Vec<_>>())
    }) {
        for (file, error) in failed {
            let kind = classify(&error, Kind::ReaderError);
            log.diagnostics
                .push(Diagnostic::in_file("gui.tree", kind, &file, error));
        }
    }
    if let Some(roster) = log.read("mesh.roster", || layers.descriptor_roster()) {
        log.notes("mesh.roster", Kind::Logged, &roster.logged);
        for unreadable in &roster.unreadable {
            log.note("mesh.roster", Kind::ParseFailure, unreadable.clone());
        }
    }
    log.read("mesh.texture_index", || layers.texture_index());
    match log.read("particles", || installed_content::particles::Catalog::read(layers)) {
        Some(catalog) => {
            log.read("weather", || installed_content::weather::effects(layers, &catalog));
            // Every pdxparticle object's emitters as the viewer's particle
            // assets read them: each subsystem through RenderDescriptor::read.
            let mut refused = Vec::new();
            let values = log.values.clone();
            log.read("particles.emitters", || {
                for object in catalog.objects() {
                    let read = catalog.resolve(object).and_then(|particle| {
                        if let Some(writer) = &values {
                            writer.write("particles.emitters", &(object, &particle));
                        }
                        let document =
                            installed_content::parser::parse(&layers.read_uncovered(&particle.path)?)?;
                        if let Some(writer) = &values {
                            writer.write("particles.emitters", &(object, &document));
                        }
                        let root = crate::map::weather_effects::particle_root(
                            &document,
                            &particle.path,
                            &particle.name,
                            crate::map::weather_effects::Pick::First,
                        )?;
                        let mut textures = Vec::new();
                        let mut pages = BTreeMap::new();
                        let read = crate::map::weather_effects::read_emitters(
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
                        refused.push((object.to_owned(), error.to_string()));
                    }
                }
                Ok(())
            });
            for (object, why) in refused {
                log.about(
                    "particles.emitters",
                    classify(&why, Kind::Refused),
                    format_args!("pdxparticle {object}"),
                    &why,
                );
            }
        }
        None => log.not_run(&["weather", "particles.emitters"], "particles"),
    }
    if let Some(abilities) = log.read("abilities", || installed_content::abilities::read(layers)) {
        log.notes("abilities", Kind::Misread, &abilities.warnings);
    }
    log.read("ambient", || installed_content::ambient::read(layers));
    interface
}

/// What the map readers give the later ones.
struct Map {
    terrain: Option<(
        installed_content::terrain::TerrainDefinitions,
        installed_content::terrain::DefaultMap,
    )>,
    buildings: Option<installed_content::buildings::Buildings>,
    ideologies: Option<(installed_content::ideologies::Ideologies, Vec<String>)>,
    resources: Option<(Vec<installed_content::resources::Resource>, Vec<String>)>,
    data: Option<installed_content::map::MapData>,
    regions: Option<BTreeMap<u32, installed_content::regions::StrategicRegion>>,
}

/// The map and what the viewer's document and the trigger environment read
/// with it.
fn map(
    log: &mut Log,
    layers: &ResolvedLayers,
    defines: Option<&Defines>,
    decode: installed_content::image::native::Decode,
) -> Map {
    let map = log.read("map", || installed_content::map::load(layers));
    log.read("map.country_colors", || {
        installed_content::map::read_country_colors(layers)
    });
    let terrain = log.read("terrain", || installed_content::terrain::read(layers));
    match &terrain {
        Some((definitions, default_map)) => {
            log.notes("terrain", Kind::Misread, &definitions.skipped);
            log.read("terrain.map_images", || {
                installed_content::terrain::read_map_images(layers, default_map, decode)
            });
        }
        None => log.not_run(&["terrain.map_images"], "terrain"),
    }
    let regions = log.read("regions", || installed_content::regions::read(layers));
    if let Some(regions) = &regions {
        log.notes("regions", Kind::Warning, &debug_notes(regions));
    }
    let regions = regions.map(|database| database.regions);
    log.read("adjacencies", || installed_content::adjacencies::read(layers));
    if let Some(rules) = log.read("adjacency_rules", || {
        installed_content::adjacency_rules::read(layers)
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
    log.read("cities", || installed_content::cities::read(layers));
    log.read("railways", || installed_content::railways::read(layers));
    log.read("positions.unit_stacks", || {
        installed_content::positions::read_unit_stacks(layers)
    });
    log.read("positions.buildings", || {
        installed_content::positions::read_buildings(layers)
    });
    let graph = log.read("province_graph", || {
        installed_content::province_graph::ProvinceGraph::read(layers)
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
    let buildings = log.read("buildings", || installed_content::buildings::read(layers));
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
    let resources = log.read("resources", || installed_content::resources::read(layers));
    if let Some((_, warnings)) = &resources {
        log.notes("resources", Kind::Warning, warnings);
    }
    let ideologies = log.read("ideologies", || installed_content::ideologies::read(layers));
    if let Some((_, warnings)) = &ideologies {
        log.notes("ideologies", Kind::Misread, warnings);
    }
    const NEEDS_DEFINES: [&str; 3] = ["movement", "dashboard.resource_bonuses", "states"];
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
        installed_content::movement::MovementMap::read(layers, defines)
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
    match log.read("states", || {
        installed_content::states::read(layers, defines, &data.definitions)
    }) {
        Some((states, warnings)) => {
            log.notes("states", Kind::Misread, &warnings);
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
        installed_content::modifier_definitions::read(layers)
    }) {
        log.notes("modifier_definitions", Kind::Misread, &warnings);
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
        Some(officers) => log.read("ideas", || installed_content::ideas::read(layers, &officers)),
        None => {
            log.not_run(&["ideas"], "ideas.officers");
            None
        }
    };
    match ideas {
        Some((_, advisors, warnings)) => {
            log.notes("ideas", Kind::Misread, &warnings);
            if let Some((_, warnings)) = log.read("characters", || {
                installed_content::characters::read(layers, advisors)
            }) {
                log.notes("characters", Kind::Misread, &warnings);
            }
        }
        None => log.not_run(&["characters"], "ideas"),
    }
    // The viewer's counter portraits (army, large), file by file.
    let values = log.values.clone();
    if let Some(stops) = log.read("characters.portraits", || {
        let names = layers.key_names()?;
        installed_content::characters::paths(layers)
            .into_iter()
            .map(|path| {
                let (_, stop) = {
                    let read = installed_content::characters::portraits(
                        &layers.read(&path)?,
                        &names,
                        "army",
                        "large",
                    )?;
                    if let Some(writer) = &values {
                        writer.write("characters.portraits", &(&path, &read));
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
        installed_content::characters::leader_templates(layers).map_err(io::Error::other)
    }) {
        for template in &templates {
            if let Err(why) = &template.role {
                log.diagnostics.push(Diagnostic::at(
                    "characters.leader_templates",
                    Kind::Refused,
                    &template.path,
                    template.line,
                    why,
                ));
            }
        }
    }
    if let Some((_, warnings)) =
        log.read("leader_traits", || installed_content::leader_traits::read(layers))
    {
        log.notes("leader_traits", Kind::Misread, &warnings);
    }
    if let Some(names) = log.read("names", || installed_content::names::read(layers)) {
        log.notes("names", Kind::Unsettled, &names.unknown);
    }
    if let Some((_, warnings)) = log.read("focus", || installed_content::focus::read(layers)) {
        log.notes("focus", Kind::Misread, &warnings);
    }
    if let Some(None) = log.read("focus.point_days", || installed_content::focus::point_days(layers)) {
        log.note(
            "focus.point_days",
            Kind::Refused,
            "no positive literal focus-day define",
        );
    }
    if let Some((_, warnings)) = log.read("technology", || {
        installed_content::technology::read_equipment(layers)
    }) {
        log.notes("technology", Kind::Misread, &warnings);
    }
}

/// Equipment, division names, unit leaders and sub-units, and the train
/// database.
fn units(
    log: &mut Log,
    layers: &ResolvedLayers,
    defines: Option<&Defines>,
    map: &Map,
    decode: installed_content::image::native::Decode,
) {
    if let Some(equipment) = log.read("equipment", || installed_content::equipment::Database::load(layers))
    {
        log.notes("equipment", Kind::Misread, &equipment.misreads);
        if let Some(lost) = &equipment.lost {
            log.note("equipment", Kind::Unsettled, lost.clone());
        }
    }
    if let Some(groups) = log.read("equipment.groups", || {
        installed_content::equipment::Groups::load(layers)
    }) {
        log.notes("equipment.groups", Kind::Misread, &groups.misreads);
        if let Some(lost) = &groups.lost {
            log.note("equipment.groups", Kind::Unsettled, lost.clone());
        }
    }
    if let Some((_, warnings)) =
        log.read("division_names", || installed_content::division_names::read(layers))
    {
        log.notes("division_names", Kind::Misread, &warnings);
    }
    match &map.terrain {
        Some((definitions, _)) => {
            // The commander content's terrain lookup (1403BCA00): the
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
                installed_content::unit_leader::Leaders::load(layers, &spelling)
            }) {
                log.notes("unit_leader", Kind::Misread, &leaders.misreads);
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
                crate::map::unit_models::UnitModelAssets::try_load(
                    layers,
                    decode,
                    defines,
                    data,
                    default_map,
                    &mut warnings,
                )
            });
            log.notes("viewer.unit_models", Kind::Warning, &warnings);
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
        installed_content::unit_leader::Traits::registered(layers)
    }) {
        log.notes("unit_leader.traits", Kind::Unsupported, &traits.unsupported);
    }
    if let Some(sub_units) = log.read("unit_leader.sub_units", || {
        installed_content::unit_leader::sub_units(layers)
    }) {
        log.notes("unit_leader.sub_units", Kind::Misread, sub_units.warnings());
    }
    // The train database with the registries the viewer's train assets give
    // it: the country tags, the graphical cultures and the entity roster.
    let cultures = log.read("train_gfx.cultures", || {
        installed_content::train_gfx::cultures(layers)
    });
    let tags = log.read("viewer.country_tags", || {
        crate::map::trains::country_tag_index(layers)
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
                installed_content::train_gfx::Database::read(
                    layers,
                    &installed_content::train_gfx::Registries {
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

/// The trigger readers, scripted localisation, combat tactics and raids.
fn triggers(log: &mut Log, layers: &ResolvedLayers, defines: Option<&Defines>) {
    if let Some(scripted) = log.read("scripted_triggers", || {
        installed_content::scripted_triggers::read(layers)
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
    if let Some(aliases) = log.read("tag_aliases", || installed_content::tag_aliases::read(layers)) {
        log.notes("tag_aliases", Kind::Unsettled, &aliases.unknown);
    }
    if let Some(names) = log.read("scripted_localisation", || {
        installed_content::scripted_localisation::names(layers)
    }) {
        // `warnings` repeats each of `unknown`.
        log.notes("scripted_localisation", Kind::Misread, &names.warnings);
    }
    if let Some(database) = log.read("combat_tactics", || installed_content::combat_tactics::read(layers)) {
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
    if let Some(catalog) = log.read("raids", || installed_content::raids::read(layers)) {
        log.notes("raids", Kind::Refused, &catalog.refused);
        for (name, why) in &catalog.unsupported {
            log.about("raids", Kind::Unsupported, format_args!("raid {name}"), why);
        }
        for uncertain in &catalog.uncertain {
            log.about(
                "raids",
                Kind::Refused,
                format_args!("raid {}", uncertain.name),
                &uncertain.reason,
            );
        }
        // Every refusal each type, uncertain declaration and category holds.
        let subjects = catalog
            .types
            .iter()
            .map(|raid| (format!("raid {}", raid.name), debug_refusals(raid)))
            .chain(catalog.uncertain.iter().flat_map(|uncertain| {
                uncertain
                    .declarations
                    .iter()
                    .map(|raid| (format!("raid {}", raid.name), debug_refusals(raid)))
            }))
            .chain(catalog.categories.values().map(|category| {
                (
                    format!("raid category {}", category.name),
                    debug_refusals(category),
                )
            }));
        for (subject, refusals) in subjects.collect::<Vec<_>>() {
            for why in refusals {
                log.about("raids", classify(&why, Kind::Refused), &subject, &why);
            }
        }
    }
    if let Some(groups) = log.read("raids.names", || {
        installed_content::raids::names::Groups::read(layers)
    }) {
        // Each refused file names itself and its line in its warning; the
        // groups it declares or could declare are refused by name.
        log.notes("raids.names", Kind::Misread, &groups.warnings);
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
                installed_content::raids::RaidGraphicsDefines::read(defines)
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
    decode: installed_content::image::native::Decode,
    registry: Option<&installed_content::gui::native::resources::Registry>,
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
        crate::map::trains::TrainAssets::load(layers, decode, defines, &mut warnings)
    });
    log.notes("viewer.trains", Kind::Warning, &warnings);
    let mut warnings = Vec::new();
    log.read("viewer.bridges", || {
        crate::map::railways::Bridges::load(layers, decode, defines, &mut warnings)
    });
    log.notes("viewer.bridges", Kind::Warning, &warnings);
    let mut warnings = Vec::new();
    if let Some(orders) = log.read("viewer.orders", || {
        crate::map::orders::OrderAssets::load(layers, decode, defines, &mut warnings)
    }) && let Err(why) = &orders.raid_defines
    {
        log.note("viewer.orders", Kind::Refused, why.clone());
    }
    log.notes("viewer.orders", Kind::Warning, &warnings);
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
                crate::map::aircraft::AircraftAssets::load(
                    layers,
                    decode,
                    data,
                    defines,
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
            log.notes("viewer.aircraft", Kind::Warning, &warnings);
        }
        None => log.not_run(&["viewer.aircraft"], "regions"),
    }
    let mut warnings = Vec::new();
    log.read("viewer.map_buildings", || {
        crate::map::map_buildings::MapBuildingAssets::load(
            layers,
            decode,
            data,
            defines,
            &mut warnings,
        )
    });
    log.notes("viewer.map_buildings", Kind::Warning, &warnings);
    let Some(registry) = registry else {
        log.not_run(&["viewer.raids"], "gui.resources");
        return;
    };
    if let Some(raids) = log.read("viewer.raids", || {
        crate::map::raids::RaidAssets::load(layers, defines, data, registry)
    }) {
        log.notes("viewer.raids", Kind::Warning, &raids.warnings);
        log.notes("viewer.raids", Kind::Refused, &raids.refusals());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content::raids::RaidType;

    #[test]
    fn counted_effect_warnings_reach_the_corpus_report() {
        let warning = "common/national_focus/gap.txt: line 3: Deferred effect `set_capital`: value reader 0x140534280, key reader 0x1403B4F40; by-key reading retained (count: 2)".to_owned();
        let mut log = Log::default();
        log.notes("focus", Kind::Misread, &[warning]);
        assert_eq!(log.diagnostics.len(), 1);
        let note = &log.diagnostics[0];
        assert_eq!(note.kind, Kind::Unsupported);
        assert_eq!(note.file.as_deref(), Some("common/national_focus/gap.txt"));
        assert_eq!(note.line, Some(3));
        assert!(note.message.contains("key reader 0x1403B4F40"));
        assert!(note.message.contains("count: 2"));
    }

    #[test]
    fn raids_are_not_run_without_the_interface_registry() {
        let layers = ResolvedLayers::from_folders(&[]).unwrap();
        let defines = installed_content::defines::read(&layers).unwrap();
        let map = Map {
            terrain: None,
            buildings: None,
            ideologies: None,
            resources: None,
            regions: None,
            data: Some(installed_content::map::MapData {
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

    /// A value read without error still holds its refusals: a raid's
    /// refused tooltip, and every marked variant and field, nested or not,
    /// each with its text as the reader gave it.
    #[test]
    fn every_nested_refusal_of_a_read_value_is_found() {
        let mut raid = RaidType {
            name: "odd".into(),
            ..Default::default()
        };
        raid.tooltip.refused = Some("tooltip \"x\" \\ é".into());
        assert_eq!(debug_refusals(&raid), ["tooltip \"x\" \\ é"]);
        assert!(debug_refusals(&RaidType::default()).is_empty());
        #[derive(Debug)]
        #[allow(dead_code)]
        enum Arrow {
            Kind(u8),
            Unsupported(String),
        }
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Read {
            arrows: Vec<Arrow>,
            lost: Option<String>,
            role: Result<u8, String>,
        }
        let read = Read {
            arrows: vec![Arrow::Kind(1), Arrow::Unsupported("zeppelin".into())],
            lost: Some("line 3".into()),
            role: Err("no role".into()),
        };
        assert_eq!(debug_refusals(&read), ["zeppelin", "line 3", "no role"]);
    }

    /// A reader's `notes` list, wherever its value holds one.
    #[test]
    fn every_note_a_read_value_holds_is_found() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Read {
            regions: Vec<u32>,
            notes: Vec<String>,
        }
        let read = Read {
            regions: vec![1],
            notes: vec![
                "map/strategicregions/a b.txt: strategic region with id 0 is deleted".into(),
                "x \"y\"".into(),
            ],
        };
        assert_eq!(debug_notes(&read), read.notes);
        assert!(debug_notes(&vec![1]).is_empty());
    }
}
