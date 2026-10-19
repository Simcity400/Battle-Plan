//! Ordered loading of the map overlay assets.

use installed_content_core::ResolvedLayers;
use installed_content_gfx::gfx;
use installed_content_image::{dds, native::Decode};
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_combat as combat;
use viewer_foundation::map::native_windows::NativeWindows;
use {
    viewer_counters::map::counters::CounterAssets,
    viewer_foundation::{
        errors::invalid,
        map::{
            lazy_icons::{DYNAMIC_BASE, LazyTextures},
            sprites::Interface,
            text,
        },
    },
};

/// Every texture and font the icon layer draws from.
pub struct MapIcons {
    pub combat: Arc<combat::Assets>,
    /// Decoded pages and strips; the batch refers to these by index.
    pub images: Vec<dds::Texture>,
    pub fonts: text::Fonts,
    // counters: the unit-counter, base and combat sheets, and the flag atlas.
    pub counters: Arc<CounterAssets>,
    /// The raid map icon's layout, sprites and raid content.
    pub raids: Arc<viewer_raids::map::raids::RaidAssets>,
    /// The Operatives map mode's intel icons' defines and content.
    pub intel: Arc<viewer_raids::map::intel_icons::IntelAssets>,
    /// Occupation definitions and marker appearance, read once after content names settle.
    pub occupation: Result<viewer_world::map::occupation_icons::Content, String>,
    /// The floating map text types and their fonts, from the recorded content stack.
    pub animated_text: Arc<viewer_world::map::animated_map_text::Assets>,
    /// Flags and portraits decoded on first draw, above [`DYNAMIC_BASE`].
    pub lazy: Arc<LazyTextures>,
}

/// Owned results of the ordered icon load, before the document finishes its operands.
pub struct LoadedIcons {
    pub combat: combat::Assets,
    pub raids: viewer_raids::map::raids::RaidAssets,
    pub images: Vec<dds::Texture>,
    pub fonts: text::Fonts,
    pub counters: CounterAssets,
    pub animated_text: viewer_world::map::animated_map_text::Assets,
    pub lazy: Arc<LazyTextures>,
}

impl MapIcons {
    /// Reads fonts and map-icon sprites through the resolved content stack, using the client's
    /// texture decode state for their pages. `defines` are the recorded defines and the
    /// graphics defines (the recorded defines with the visual mods' graphics sections), which
    /// the counters, combat window, raids and floating map text read.
    #[allow(clippy::too_many_arguments)] // The content stack plus the client's settings.
    pub fn load(
        layers: &ResolvedLayers,
        (defines, graphics): (
            &installed_content_core::defines::Defines,
            &installed_content_core::defines::Defines,
        ),
        map: &installed_content_core::map::MapData,
        regions: &BTreeMap<u32, installed_content_regions::regions::StrategicRegion>,
        language: &str,
        map_size: [u32; 2],
        gui_viewport: [u32; 2],
        decode: Decode,
    ) -> io::Result<LoadedIcons> {
        let mut images = Vec::new();
        // The natively drawn map-icon windows read nothing the legacy readers give. This read
        // decides only which sprites, fonts and textures the windows' resources load, a set no
        // GUI viewport changes (`max_resolution` changes a `windowType`'s size, never which
        // elements exist); every read a view lays out and draws is at its own viewport
        // (`WindowCache::at`).
        let windows = NativeWindows::load_at(layers, gui_viewport)?;
        let layouts = viewer_counters::map::counters::Layouts::read(&windows);
        let mut font_warnings = Vec::new();
        // The battle-plan label font is named by `maparrows.txt`'s `arrowText`.
        let arrow_font = viewer_foundation::map::ribbon_content::arrow_text_font(layers)?;
        // Interface definitions and graphics files share registries, so later font and colour
        // overrides affect both world and GUI text.
        let registry = installed_content_core::gui::native::resources::Registry::from_files(
            installed_content_core::gui::native::sprites::read_object_layers(layers)?,
        );
        let interface = Interface {
            sprites: gfx::SpriteSet::new(&registry.files),
            registry,
            decode,
        };
        let fonts = text::Fonts::load(
            layers,
            &interface,
            language,
            &text::DeclaredFonts {
                counter: layouts
                    .land
                    .as_ref()
                    .ok()
                    .and_then(|land| land.count.font.as_deref()),
                arrow: arrow_font.as_deref(),
            },
            &mut images,
            &mut font_warnings,
        )?;
        let sprites = &interface.sprites;
        // Flag and portrait reads happen after this load returns, so the lazy
        // stores keep their own handle on the layers.
        let shared = Arc::new(layers.clone());
        let lazy = LazyTextures::new();
        // counters
        let mut counters = CounterAssets::load(
            &shared,
            (layouts, windows),
            graphics,
            (sprites, &interface.registry),
            (language, map_size, decode),
            &mut images,
            &lazy,
        )?;
        counters.warnings.extend(font_warnings);
        // What the game does not define in the interface files (each file's own level, each
        // sprite and each font declaration), named and counted; a consumer asking for a refused
        // sprite is told why ([`gfx::SpriteSet::lookup`]), and a refused font does not load
        // ([`text::load`]).
        let refused: Vec<String> = sprites
            .refused
            .iter()
            .cloned()
            .chain(
                interface
                    .registry
                    .fonts
                    .fonts()
                    .iter()
                    .flat_map(|font| &font.facts.refusals)
                    .map(|refusal| format!("font {refusal}")),
            )
            .collect();
        if !refused.is_empty() {
            counters.warnings.push(format!(
                "Interface files: {} refusals of what the game does not define: {}.",
                refused.len(),
                refused.join("; ")
            ));
        }
        let combat = combat::Assets::load(
            &shared,
            regions,
            graphics,
            &interface,
            &counters.large_flags,
            (language, map_size, &counters.windows),
            &mut images,
            &lazy,
        )?;
        let mut raids =
            viewer_raids::map::raids::RaidAssets::load(layers, graphics, map, &interface.registry)?;
        raids.trigger_content.triggers = Arc::new(combat.tactics.trigger_names());
        raids.trigger_content.character_traits = combat.tactics.trait_databases();
        raids.transport_sub_units = combat.sub_units().cloned().map_err(str::to_owned);
        counters
            .gui
            .load(layers, &raids.gui_assets(), &mut images)?;
        // The floating map text's fonts load into the counters' GUI resources.
        let animated_text = viewer_world::map::animated_map_text::Assets::load(
            layers,
            (defines, graphics),
            map_size,
            decode,
            &interface.registry.files,
        );
        counters
            .gui
            .load(layers, &animated_text.gui_assets(), &mut images)?;
        // The resource icons' rows draw the resource strip inline.
        counters.gui.load(
            layers,
            &viewer_world::map::resource_icons::gui_assets(),
            &mut images,
        )?;
        if images.len() >= DYNAMIC_BASE {
            return Err(invalid("more static icon pages than texture ids"));
        }
        Ok(LoadedIcons {
            combat,
            raids,
            images,
            fonts,
            counters,
            animated_text,
            lazy,
        })
    }

    /// Loads the map-icon windows' resources ([`CounterAssets::load_gui`]) once the names pass
    /// has read `localisation`, appending their textures after every other static page. Returns
    /// what does not load, for the warning area.
    pub fn load_gui(
        combat: &mut combat::Assets,
        counters: &mut CounterAssets,
        images: &mut Vec<dds::Texture>,
        layers: &ResolvedLayers,
        localisation: &installed_content_localisation::localisation::Localisation,
    ) -> io::Result<Vec<String>> {
        // Both registries consume this one harvest of localisation and combat shortcut keycaps.
        let (extra, mut warnings) = combat.localisation_assets(localisation);
        warnings.extend(counters.load_gui(layers, localisation, extra.clone(), images)?);
        warnings.extend(combat.load_gui(layers, localisation, &extra, images)?);
        if images.len() >= DYNAMIC_BASE {
            return Err(invalid("more static icon pages than texture ids"));
        }
        Ok(warnings)
    }
}
