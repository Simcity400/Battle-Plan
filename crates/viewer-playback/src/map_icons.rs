use content_reader_layers::ResolvedLayers;
use content_reader_gfx::gfx;
use content_reader_image::{dds, native::Decode};
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_combat as combat;
use viewer_native_gui::native_windows::NativeWindows;
use viewer_counters::counters::CounterAssets;
use viewer_runtime::errors::invalid;
use viewer_native_gui::{lazy_icons::{DYNAMIC_BASE, LazyTextures}, sprites::Interface, text};

pub struct MapIcons {
    pub combat: Arc<combat::Assets>,
    pub images: Vec<dds::Texture>,
    pub fonts: text::Fonts,
    pub counters: Arc<CounterAssets>,
    pub raids: Arc<viewer_raids::map::raids::RaidAssets>,
    pub intel: Arc<viewer_raids::map::intel_icons::IntelAssets>,
    pub occupation: Result<viewer_map_icons::occupation_icons::Content, String>,
    pub animated_text: Arc<viewer_map_text::animated_map_text::Assets>,
    pub lazy: Arc<LazyTextures>,
}

pub struct LoadedIcons {
    pub combat: combat::Assets,
    pub raids: viewer_raids::map::raids::RaidAssets,
    pub images: Vec<dds::Texture>,
    pub fonts: text::Fonts,
    pub counters: CounterAssets,
    pub animated_text: viewer_map_text::animated_map_text::Assets,
    pub lazy: Arc<LazyTextures>,
}

impl MapIcons {
    #[allow(clippy::too_many_arguments, reason = "The content stack plus the client's settings.")]
    pub fn load_with_products(
        products: Option<&map_content::loader::Loader>,
        layers: &ResolvedLayers,
        (defines, graphics): (
            &content_reader_defines::defines::Defines,
            &content_reader_defines::defines::Defines,
        ),
        map: &content_reader_map::map::MapData,
        regions: &BTreeMap<u32, content_reader_regions::regions::StrategicRegion>,
        language: &str,
        map_size: [u32; 2],
        gui_viewport: [u32; 2],
        decode: Decode,
    ) -> io::Result<LoadedIcons> {
        std::thread::scope(|scope| {
            let strict = content_reader_layers::content::is_strict();
            let raids = content_reader_layers::parallel::spawn(scope, move || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    content_reader_layers::content::observe_overlay_refusals_isolated(|| {
                        let load = || {
                            viewer_raids::map::raids::RaidAssets::load_content_with_products(
                                products, layers, graphics, map,
                            )
                        };
                        if strict {
                            content_reader_layers::content::strict(load)
                        } else {
                            load()
                        }
                    })
                }))
            });
            let mut images = Vec::new();
            let windows = NativeWindows::load_at(layers, gui_viewport)?;
            let layouts = viewer_counter_kit::Layouts::read(&windows);
            let mut font_warnings = Vec::new();
            let arrow_font = viewer_gpu::ribbon_content::arrow_text_font(layers)?;
            let registry = content_reader_gui::gui::native::resources::Registry::from_files(
                content_reader_gui::gui::native::sprites::read_object_layers(layers)?,
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
            let shared = Arc::new(layers.clone());
            let lazy = LazyTextures::new();
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
            let combat = combat::Assets::load_with_products(
                products,
                &shared,
                regions,
                graphics,
                &interface,
                &counters.large_flags,
                (language, map_size, &counters.windows),
                &mut images,
                &lazy,
            )?;
            let (raids, refusals) = content_reader_layers::parallel::join(raids)
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            content_reader_layers::content::credit_overlay_refusals(&refusals);
            let mut raids = raids?;
            raids.retain_glyphs(&interface.registry);
            raids.trigger_content.triggers = Arc::new(combat.tactics.trigger_names());
            raids.trigger_content.character_traits = combat.tactics.trait_databases();
            raids.transport_sub_units = combat.sub_units().cloned().map_err(str::to_owned);
            counters
                .gui
                .load(layers, &raids.gui_assets(), &mut images)?;
            let animated_text = viewer_map_text::animated_map_text::Assets::load(
                layers,
                (defines, graphics),
                map_size,
                decode,
                &interface.registry.files,
            );
            counters
                .gui
                .load(layers, &animated_text.gui_assets(), &mut images)?;
            counters.gui.load(
                layers,
                &viewer_map_icons::resource_icons::gui_assets(),
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
        })
    }

    pub fn load_gui(
        combat: &mut combat::Assets,
        counters: &mut CounterAssets,
        images: &mut Vec<dds::Texture>,
        layers: &ResolvedLayers,
        localisation: &content_reader_localisation::localisation::Localisation,
    ) -> io::Result<Vec<String>> {
        let (extra, mut warnings) = combat.localisation_assets(localisation);
        warnings.extend(counters.load_gui(layers, localisation, extra.clone(), images)?);
        warnings.extend(combat.load_gui(layers, localisation, &extra, images)?);
        if images.len() >= DYNAMIC_BASE {
            return Err(invalid("more static icon pages than texture ids"));
        }
        Ok(warnings)
    }
}
