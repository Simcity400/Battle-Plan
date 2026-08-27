//! THE GAME-FILE LEDGER: the canonical account of every game-install and
//! game-derived file the replay layer reads, and what pins each one.
//!
//! The `.battleplan` file is the only extractor->replay connection
//! (SINGLE-FILE LAW). Game-install files are a separate, legitimate dependency
//! class: the replay renders by reading the SAME install the capture ran
//! against. This module states, once, what that class contains and how each
//! member is bound to the recording, and the tests below refuse a read route
//! that escapes it.

use std::collections::BTreeMap;

use replay_core::{RecorderProductAdmission, ValidatedContentAdmission};

use crate::install_paths::RenderSettingsPin;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validation {
    /// Carried inside the recording, digested, and bound to the recording's
    /// content epoch before any consumer sees it.
    PackagedDigested,
    /// Read from the local install, but only through the recording's own
    /// content admission: the payload digest, byte length, winning layer and
    /// source container of every file are compared at open, the bytes are
    /// snapshotted read-only, and every later read re-checks them.
    InstallReadPinned,
    /// Read from the local install with no recording to pin it against. Only
    /// reachable on routes that carry no production recording; the blocker
    /// registry refuses these surfaces whenever one is open.
    InstallReadUnpinnedDebugOnly,
    /// Read from the local install on a route a recording DOES reach, because
    /// the recording carries no pin for it. Never silent: the open-time
    /// statement names the class and the values this machine supplied.
    InstallReadUnpinnedNamed,
    /// Read with no pin on a route that a recording CAN reach. The ledger must
    /// carry no row in this state.
    #[cfg_attr(not(test), allow(dead_code))]
    Unaccounted,
}

impl Validation {
    pub const fn token(self) -> &'static str {
        match self {
            Validation::PackagedDigested => "PACKAGED+DIGESTED",
            Validation::InstallReadPinned => "INSTALL-READ-PINNED",
            Validation::InstallReadUnpinnedDebugOnly => "INSTALL-READ-UNPINNED",
            Validation::InstallReadUnpinnedNamed => "INSTALL-READ-UNPINNED-NAMED",
            Validation::Unaccounted => "UNACCOUNTED",
        }
    }
}

pub struct LedgerRow {
    /// Stable identity used by the open-time statement and by the tests.
    pub class: &'static str,
    #[cfg_attr(not(test), allow(dead_code))]
    pub subject: &'static str,
    pub route: &'static str,
    /// Every module that reads this class. Across the rows these cover the
    /// whole census: `every_census_file_is_named_by_a_row` proves it.
    pub read_sites: &'static [&'static str],
    #[cfg_attr(not(test), allow(dead_code))]
    pub logical_path_examples: &'static [&'static str],
    pub validation: Validation,
    /// What a version / DLC / mod mismatch does to the viewer.
    pub mismatch: &'static str,
}

/// Classes whose membership is decided by the logical path of an admitted
/// content row. Order matters: the first matching prefix rule wins.
pub const MAP_GEOMETRY: &str = "map_geometry";
pub const MAP_TERRAIN_TEXTURE: &str = "map_terrain_texture";
pub const SHADER: &str = "shader";
pub const MODEL_MESH: &str = "model_mesh";
pub const INTERFACE_DEFINITION: &str = "interface_definition";
pub const INTERFACE_TEXTURE: &str = "interface_texture";
pub const COUNTRY_FLAG: &str = "country_flag";
pub const PORTRAIT: &str = "portrait";
pub const FONT: &str = "font";
pub const LOCALISATION: &str = "localisation";
pub const SCRIPT_DEFINITION: &str = "script_definition";
pub const EVENT_DEFINITION: &str = "event_definition";
pub const WORLD_GFX_TABLE: &str = "world_gfx_table";
pub const OTHER_ADMITTED: &str = "other_admitted";

/// Classes that are not admitted-content rows.
pub const PROFILE_IDENTITY: &str = "profile_identity";
pub const RENDER_SETTINGS: &str = "render_settings";
pub const PACKAGED_STATIC_ARTIFACT: &str = "packaged_static_artifact";
pub const DEBUG_EVIDENCE_INPUT: &str = "debug_evidence_input";
pub const EVIDENCE_USER_DATA_STAGING: &str = "evidence_user_data_staging";
pub const CONTENT_ROOT: &str = "content_root";
pub const PASS_THROUGH_READER: &str = "pass_through_reader";
pub const RESOLVED_PATH_DECODER: &str = "resolved_path_decoder";
pub const REPLAY_CONTAINER: &str = "replay_container";
pub const FROZEN_CENSUS_ROUTE: &str = "frozen_census_route";
pub const SHADER_CACHE: &str = "shader_cache";
pub const AUDIO: &str = "audio";

const PINNED_MISMATCH: &str =
    "refused at open with the exact issue class (wrong game version / executable / DLC / mod \
     identity / load order / replace_path outcome / language / setting / winning layer / payload \
     hash or length) and the path that failed; nothing is substituted";

pub const GAME_FILE_LEDGER: &[LedgerRow] = &[
    LedgerRow {
        class: MAP_GEOMETRY,
        subject: "province/state geometry and the map tables derived from it: map/provinces.bmp, \
                  map/definition.csv, map/heightmap.bmp, map/rivers.bmp, map/trees.bmp, \
                  map/terrain.bmp, map/adjacencies.csv, map/default.map, map/cities.txt, \
                  map/buildings.txt, map/railways.txt, map/supply_nodes.txt, map/unitstacks.txt, \
                  map/weatherpositions.txt, map/seasons.txt, map/strategicregions/*, \
                  history/states/*",
        route: "GameFileResolver under the recording's ValidatedContentAdmission",
        read_sites: &[
        "map_viewer/src/border_world_synthesis.rs",
        "map_viewer/src/bundle_gen.rs",
        "map_viewer/src/frame_bundle.rs",
        "map_viewer/src/game_data.rs",
        "map_viewer/src/map_furniture.rs",
        "map_viewer/src/map_icon_runtime.rs",
        "map_viewer/src/map_mode_terrain_resource.rs",
        "map_viewer/src/map_static_data.rs",
        "map_viewer/src/mapmode_air.rs",
        "map_viewer/src/naval_supremacy_tint.rs",
        "map_viewer/src/o05_fixed_object_law.rs",
        "map_viewer/src/political_color_synthesis.rs",
        "map_viewer/src/posteffect_region_weights.rs",
        "map_viewer/src/production_features/xmap04.rs",
        "map_viewer/src/production_features/xmap05.rs",
        "map_viewer/src/production_features/xmap07.rs",
        "map_viewer/src/production_features/xmap08.rs",
        "map_viewer/src/production_features/xmap15.rs",
        "map_viewer/src/production_features/xmap17.rs",
        "map_viewer/src/production_features/xmap18.rs",
        "map_viewer/src/production_foundation.rs",
        "map_viewer/src/rail_files.rs",
        "map_viewer/src/river_mesh_synthesis.rs",
        "map_viewer/src/river_world_synthesis.rs",
        "map_viewer/src/seasons.rs",
        "map_viewer/src/snow_law.rs",
        "map_viewer/src/terrain_id_map_synthesis.rs",
        "map_viewer/src/terrain_mesh_law.rs",
        "map_viewer/src/tree_files.rs",
        "map_viewer/src/weather_law.rs",
        "map_viewer/src/xmap04_d3d_handoff.rs",
        "map_viewer/src/xmap07_d3d_handoff.rs",
        "map_viewer/src/province_positions.rs",
        "map_viewer/src/xmap12_population.rs",
        "replay_app/src/content_compatibility.rs",
        ],
        logical_path_examples: &["map/provinces.bmp", "map/definition.csv", "history/states"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: MAP_TERRAIN_TEXTURE,
        subject: "map/terrain/* diffuse, normal, mask, city-light, snow, mud, strait, border and \
                  reflection textures",
        route: "GameFileResolver under the recording's ValidatedContentAdmission",
        read_sites: &[
        "map_viewer/src/border_law.rs",
        "map_viewer/src/border_world_synthesis.rs",
        "map_viewer/src/bundle_gen.rs",
        "map_viewer/src/frame_bundle.rs",
        "map_viewer/src/game_data.rs",
        "map_viewer/src/mod_resolver.rs",
        "map_viewer/src/production_features/xmap04.rs",
        "map_viewer/src/production_features/xmap05.rs",
        "map_viewer/src/production_features/xmap07.rs",
        "map_viewer/src/production_features/xmap08.rs",
        "map_viewer/src/production_features/xmap12.rs",
        "map_viewer/src/reflection_cube_law.rs",
        "map_viewer/src/tree_files.rs",
        "map_viewer/src/underwater_pass.rs",
        "map_viewer/src/xmap12_population.rs",
        "replay_app/src/app.rs",
        ],
        logical_path_examples: &["map/terrain/reflection.dds", "map/terrain/Tree_tint.bmp"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: SHADER,
        subject: "Clausewitz effect sources and their include graph: gfx/FX/*.shader, \
                  gfx/FX/*.fxh, gfx/FX/*.lua, gfx/models/**/*.shader",
        route: "GameFileResolver under the recording's ValidatedContentAdmission; every #include \
                is resolved through the same resolver, so a mod-added include is refused unless \
                its exact identity was admitted",
        read_sites: &[
        "game_ui/src/compose.rs",
        "game_ui/src/runtime_graphics.rs",
        "map_viewer/src/border_pass.rs",
        "map_viewer/src/border_world_synthesis.rs",
        "map_viewer/src/building_files.rs",
        "map_viewer/src/bundle_gen.rs",
        "map_viewer/src/frame_bundle.rs",
        "map_viewer/src/game_data.rs",
        "map_viewer/src/mod_resolver.rs",
        "map_viewer/src/political_color_synthesis.rs",
        "map_viewer/src/production_features/xmap04.rs",
        "map_viewer/src/production_features/xmap05.rs",
        "map_viewer/src/production_features/xmap06.rs",
        "map_viewer/src/production_features/xmap07.rs",
        "map_viewer/src/production_features/xmap08.rs",
        "map_viewer/src/production_features/xmap10.rs",
        "map_viewer/src/production_features/xmap15.rs",
        "map_viewer/src/production_features/xmap17.rs",
        "map_viewer/src/production_features/xmap19.rs",
        "map_viewer/src/rail_files.rs",
        "map_viewer/src/shader_compile.rs",
        "map_viewer/src/shadow_law.rs",
        "map_viewer/src/terrain_id_map_synthesis.rs",
        "map_viewer/src/tree_files.rs",
        "map_viewer/src/underwater_pass.rs",
        "map_viewer/src/weather_law.rs",
        "map_viewer/src/xmap10_d3d_handoff.rs",
        "map_viewer/src/xmap16_supply_arrow_handoff.rs",
        ],
        logical_path_examples: &["gfx/FX/pdxmap.shader", "gfx/FX/constants.fxh"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: MODEL_MESH,
        subject: "entity/mesh/animation content: gfx/models/**/*.mesh and *.anim, the \
                  gfx/models/**/*.dds material textures those meshes name, and \
                  gfx/entities/*.asset and *.gfx",
        route: "GameFileResolver under the recording's ValidatedContentAdmission",
        read_sites: &[
        "map_viewer/src/building_files.rs",
        "map_viewer/src/bundle_gen.rs",
        "map_viewer/src/o05_fixed_object_law.rs",
        "map_viewer/src/pdx_mesh.rs",
        "map_viewer/src/production_features/xmap19.rs",
        "map_viewer/src/rail_files.rs",
        "map_viewer/src/tree_files.rs",
        "map_viewer/src/weather_law.rs",
        ],
        logical_path_examples: &["gfx/entities/buildings.asset", "gfx/entities/trees.gfx"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: INTERFACE_DEFINITION,
        subject: "interface/*.gui window definitions and interface/*.gfx sprite registries, \
                  including interface/core.gfx (which also carries the bitmapfont registry)",
        route: "ProductionContentInterpreter over an AssetResolver restricted to the admitted \
                content set",
        read_sites: &[
        "game_ui/src/active_abilities.rs",
        "game_ui/src/authored_role_index.rs",
        "game_ui/src/character_ui.rs",
        "game_ui/src/combat_detail_canvas.rs",
        "game_ui/src/compose.rs",
        "game_ui/src/decision_chrome.rs",
        "game_ui/src/designer_observer.rs",
        "game_ui/src/event_chrome.rs",
        "game_ui/src/focus_observer.rs",
        "game_ui/src/font.rs",
        "game_ui/src/hud.rs",
        "game_ui/src/hud_role_index.rs",
        "game_ui/src/major_views.rs",
        "game_ui/src/markup.rs",
        "game_ui/src/mod_ui_discovery.rs",
        "game_ui/src/occupation_binding.rs",
        "game_ui/src/peace_conference_ui.rs",
        "game_ui/src/power_balance_compositor.rs",
        "game_ui/src/probe.rs",
        "game_ui/src/production_lines.rs",
        "game_ui/src/production_renderer.rs",
        "game_ui/src/research_compositor.rs",
        "game_ui/src/tooltip_compositor.rs",
        "game_ui/src/topbar.rs",
        "game_ui/src/trade_compositor.rs",
        "game_ui/src/trade_layout.rs",
        "map_viewer/src/combat_indicator_assets.rs",
        "map_viewer/src/map_furniture.rs",
        "map_viewer/src/map_icon_runtime.rs",
        "map_viewer/src/map_mode_terrain_resource.rs",
        "map_viewer/src/production_features/xmap14.rs",
        "map_viewer/src/production_features/xmap15.rs",
        "map_viewer/src/production_features/xmap17.rs",
        "map_viewer/src/rail_files.rs",
        "map_viewer/src/unit_counter_assets.rs",
        "replay_app/src/font.rs",
        "replay_app/src/gui_definition_admission.rs",
        ],
        logical_path_examples: &["interface/core.gfx", "interface/topbar.gui"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: INTERFACE_TEXTURE,
        subject: "gfx/interface/** and gfx/texticons/** sprite atlases named by an admitted .gfx \
                  sprite definition, plus gfx/particles/** - both its atlases and the .asset \
                  particle definitions that name them",
        route: "AssetResolver::decode_texture / GameFileResolver, restricted to admitted content",
        read_sites: &[
        "map_viewer/src/map_furniture.rs",
        "map_viewer/src/map_mode_terrain_resource.rs",
        "map_viewer/src/production_features/xmap19.rs",
        "map_viewer/src/rail_files.rs",
        "map_viewer/src/unit_counter_assets.rs",
        "map_viewer/src/weather_law.rs",
        ],
        logical_path_examples: &[
            "gfx/interface/resources_strip.dds",
            "gfx/interface/mapicons/mapicon_building_airbase.dds",
        ],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: COUNTRY_FLAG,
        subject: "gfx/flags/** country flags, including the ideology/cosmetic-tag and size-folder \
                  candidate ladder",
        route: "AssetResolver candidate ladder restricted to admitted content: a candidate the \
                capture never read is not visible, so the ladder cannot fall through to a flag \
                the engine would not have drawn",
        read_sites: &[
        "game_ui/src/probe.rs",
        "game_ui/src/topbar.rs",
        ],
        logical_path_examples: &["gfx/flags/medium/GER_democratic.tga", "gfx/flags/small/GER.tga"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: PORTRAIT,
        subject: "gfx/leaders/** and gfx/portraits/** character portraits named by admitted \
                  common/characters and common/units/leaders definitions",
        route: "AssetResolver::decode_texture restricted to admitted content",
        read_sites: &["game_ui/src/character_ui.rs"],
        logical_path_examples: &["gfx/leaders/GER/portrait_ger_army.dds"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: FONT,
        subject: "gfx/fonts/*.fnt bitmap-font descriptors, their .dds/.tga atlases, and the \
                  per-language override families named by interface/core.gfx",
        route: "GameFileResolver / AssetResolver under the recording's ValidatedContentAdmission; \
                the language that selects the override family is the recording's pinned \
                localisation_language",
        read_sites: &[
        "game_ui/src/font.rs",
        "map_viewer/src/map_furniture.rs",
        "map_viewer/src/production_features/xmap14.rs",
        "replay_app/src/font.rs",
        ],
        logical_path_examples: &["gfx/fonts/hoi_20bs.fnt", "gfx/fonts/hoi_20bs.dds"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: LOCALISATION,
        subject: "localisation/<language>/*.yml: the display names every recorded key resolves to",
        route: "Localisation::load_admitted_language over an admitted AssetResolver; the \
                directory listing is the admitted set, not the local directory, and every source \
                file a key came from must itself be admitted",
        read_sites: &[
        "game_ui/src/authored_role_index.rs",
        "game_ui/src/hud_role_index.rs",
        "game_ui/src/loc.rs",
        "game_ui/src/mod_ui_discovery.rs",
        "game_ui/src/production_lines.rs",
        "map_viewer/src/map_icon_runtime.rs",
        "map_viewer/src/map_text.rs",
        "map_viewer/src/production_features/xmap14.rs",
        "map_viewer/src/production_features/xmap17.rs",
        "replay_app/src/gui_definition_admission.rs",
        ],
        logical_path_examples: &["localisation/english/focus_l_english.yml"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: SCRIPT_DEFINITION,
        subject: "common/** Clausewitz definitions parsed at replay time: defines (.lua), \
                  buildings, terrain, resources, ideologies, countries, country_tags, characters, \
                  unit_leader, technologies, occupation_laws, scripted_guis, \
                  scripted_localisation, weather",
        route: "GameFileResolver / ProductionContentInterpreter under the recording's \
                ValidatedContentAdmission",
        read_sites: &[
        "game_ui/src/active_abilities.rs",
        "game_ui/src/character_ui.rs",
        "game_ui/src/content_interpreter.rs",
        "game_ui/src/decision_chrome.rs",
        "game_ui/src/font.rs",
        "game_ui/src/mod_ui_discovery.rs",
        "game_ui/src/occupation_binding.rs",
        "game_ui/src/power_balance.rs",
        "game_ui/src/scripted_gui_runtime.rs",
        "game_ui/src/tooltip_compositor.rs",
        "map_viewer/src/bloom.rs",
        "map_viewer/src/bundle_gen.rs",
        "map_viewer/src/game_data.rs",
        "map_viewer/src/grading_law.rs",
        "map_viewer/src/map_furniture.rs",
        "map_viewer/src/map_icon_runtime.rs",
        "map_viewer/src/map_mode_admin.rs",
        "map_viewer/src/map_mode_terrain_resource.rs",
        "map_viewer/src/map_static_data.rs",
        "map_viewer/src/mapmode_air.rs",
        "map_viewer/src/mapmode_factions.rs",
        "map_viewer/src/mapmode_infrastructure.rs",
        "map_viewer/src/naval_supremacy_tint.rs",
        "map_viewer/src/o05_fixed_object_law.rs",
        "map_viewer/src/occupation_mapmode.rs",
        "map_viewer/src/political_color_synthesis.rs",
        "map_viewer/src/production_features/xmap04.rs",
        "map_viewer/src/production_features/xmap05.rs",
        "map_viewer/src/production_features/xmap06.rs",
        "map_viewer/src/production_features/xmap07.rs",
        "map_viewer/src/production_features/xmap08.rs",
        "map_viewer/src/production_features/xmap10.rs",
        "map_viewer/src/production_features/xmap13.rs",
        "map_viewer/src/production_features/xmap14.rs",
        "map_viewer/src/production_features/xmap17.rs",
        "map_viewer/src/production_features/xmap18.rs",
        "map_viewer/src/rail_files.rs",
        "map_viewer/src/river_mesh_synthesis.rs",
        "map_viewer/src/shadow_law.rs",
        "map_viewer/src/terrain_id_map_synthesis.rs",
        "map_viewer/src/tree_runtime_selection.rs",
        "map_viewer/src/unit_counter_assets.rs",
        "map_viewer/src/weather_law.rs",
        "map_viewer/src/xmap12_population.rs",
        "replay_app/src/font.rs",
        "replay_app/src/gui_definition_admission.rs",
        "replay_app/src/political_scene.rs",
        "replay_app/src/world_audio_law.rs",
        ],
        logical_path_examples: &["common/defines/00_graphics.lua", "common/weather.txt"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: EVENT_DEFINITION,
        subject: "events/*.txt: the authored event definitions the event window binds a fired \
          event id to",
        route: "AssetResolver::list_files_recursive over the admitted content set, then \
          ProductionContentInterpreter for the payload",
        read_sites: &[
        "game_ui/src/event_chrome.rs",
        ],
        logical_path_examples: &["events"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: WORLD_GFX_TABLE,
        subject: "gfx-root tables outside gfx/FX: gfx/posteffect_volumes.txt, \
                  gfx/maparrows/maparrows.txt, gfx/world/**/*colorcorrection*.tga",
        route: "GameFileResolver under the recording's ValidatedContentAdmission",
        read_sites: &[
        "map_viewer/src/bloom.rs",
        "map_viewer/src/color_lut.rs",
        "map_viewer/src/grading_law.rs",
        "map_viewer/src/production_features/xmap10.rs",
        "map_viewer/src/production_features/xmap15.rs",
        ],
        logical_path_examples: &["gfx/posteffect_volumes.txt", "gfx/maparrows/maparrows.txt"],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: OTHER_ADMITTED,
        subject: "any admitted path the classification above does not name; the open-time \
                  statement counts it separately so an unclassified family cannot hide inside a \
                  named one",
        route: "GameFileResolver under the recording's ValidatedContentAdmission",
        read_sites: &[
        "game_ui/src/authored_role_index.rs",
        "game_ui/src/content_interpreter.rs",
        "game_ui/src/font.rs",
        "game_ui/src/hud_role_index.rs",
        "game_ui/src/mod_ui_discovery.rs",
        "game_ui/src/probe.rs",
        "game_ui/src/production_renderer.rs",
        "map_viewer/src/building_files.rs",
        "map_viewer/src/combat_indicator_assets.rs",
        "map_viewer/src/map_icon_runtime.rs",
        "map_viewer/src/production_features/xmap12.rs",
        "map_viewer/src/production_features/xmap14.rs",
        "map_viewer/src/production_features/xmap19.rs",
        "map_viewer/src/tree_runtime_selection.rs",
        "map_viewer/src/xmap12_population.rs",
        "replay_app/src/font.rs",
        "replay_app/src/gui_definition_admission.rs",
        ],
        logical_path_examples: &[],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
    LedgerRow {
        class: PROFILE_IDENTITY,
        subject: "install/profile identity files read OUTSIDE the effective VFS: \
                  launcher-settings.json, the executable it names, settings.txt, pdx_settings.txt, \
                  dlc_load.json, and the mod/*.mod and dlc/*.dlc descriptors (plus mod .zip \
                  containers) the launcher selection names",
        route: "install_paths::playback_environment_from_paths and \
                GameFileResolver::from_recorded_content_layers",
        read_sites: &[
            "replay_app/src/install_paths.rs",
            "map_viewer/src/mod_resolver.rs",
        ],
        logical_path_examples: &["launcher-settings.json", "settings.txt", "dlc_load.json"],
        validation: Validation::InstallReadPinned,
        mismatch:
            "each file is pinned by the material value derived from it, not by its own bytes: the \
             executable by sha256, the launcher metadata by version+build, settings.txt by every \
             required setting key (never by its own digest - settings_file_sha256 is \
             recorder-local and refused from required_settings), pdx_settings.txt by the \
             localisation language, dlc_load.json and the descriptors by \
             the exact layer roster, ordinals, descriptor_sha256, source_container_sha256 and \
             replace_path outcomes. Any divergence refuses at open with that issue class named",
    },
    LedgerRow {
        class: RENDER_SETTINGS,
        subject: "settings.txt mapRenderingOptions draw_hires_terrain, high_gfx_shaders and \
          texture_quality",
        route: "install_paths::playback_render_settings, read from the selected profile's \
          settings.txt at open",
        read_sites: &[
            "replay_app/src/install_paths.rs",
            "map_viewer/src/terrain_lod_law.rs",
            "map_viewer/src/production_features/xmap04.rs",
            "map_viewer/src/production_features/xmap12.rs",
        ],
        logical_path_examples: &["settings.txt"],
        validation: Validation::InstallReadUnpinnedNamed,
        mismatch:
            "these three change what is drawn, not which file is read: draw_hires_terrain drops \
              the near stride-one terrain tiles, high_gfx_shaders swaps the terrain effect for \
              terrain_low_gfx, and texture_quality selects a different fow_noise texture. A \
              recording that pins all three (required_settings map_draw_hires_terrain, \
              map_high_gfx_shaders, map_texture_quality) refuses at open on any disagreement. A \
              recording that pins none - every capture made before those keys existed - is NAMED \
              at open with the three values this machine supplied, so the reader knows the image \
              is drawn at the local quality tier and not the capture's. Pinning them at capture is \
              the follow-up: the recorder already parses settings.txt for display_variant, and the \
              three keys join required_settings there",
    },
    LedgerRow {
        class: PACKAGED_STATIC_ARTIFACT,
        subject: "the eight catalogs the recorder packages into the recording: \
                  templates_equipment_catalog, localisation_join, decisions_static_definitions, \
                  idea_tech_loc_xref_static, ship_definitions_static, \
                  gfx_icon_asset_manifest_static, technology_sharing_static, focus_defs_static. \
                  Every one is a join over authored files the admitted install already carries, \
                  and every generator's own scope block excludes its runtime state, so all eight \
                  belong install-read and pinned under the capture law; what packages them is \
                  the replay's missing parsers, not the content \
                  (PACKAGED_STATIC_ARTIFACT_RATIONALE names the blocker per row)",
        route: "carried in the .battleplan container; the roster, the index and the container \
                role must agree, the payload is re-hashed on open, and the recording's content \
                epoch must authorize that exact digest",
        read_sites: &[
            "replay_core/src/source.rs",
            "replay_app/src/app.rs",
        ],
        logical_path_examples: &[
            "static/focus_defs_static/focus_defs_static.json",
            "static/gfx_icon_asset_manifest_static/gfx_icon_manifest_catalog.json",
        ],
        validation: Validation::PackagedDigested,
        mismatch:
            "an artifact whose digest the content epoch does not authorize is never consumed: it \
             is listed as not-content-bound, and one the capture could not package at all is \
             listed as missing. Both lists are named at open with what the viewer loses; \
             focus_defs_static additionally refuses playback because the focus tree has no \
             degraded form. Beyond a digest mismatch there is a standing substitution: every one \
             of the eight is joined against the English-only localisation_join, so on a \
             non-English recording every display name a packaged catalog supplies - national \
             focus, technology and idea labels - is English. That is named at open, per \
             recording, not left to the reader",
    },
    LedgerRow {
        class: DEBUG_EVIDENCE_INPUT,
        subject: "map-frame capture bundles (--bundle), captured pass sessions (--capture), the \
                  ProvinceSecondaryColorMap and TerrainIDMap capture directories, the Static1936 \
                  baseline, and the map_viewer/game_ui dev binaries (dump_*, ui_probe, \
                  acceptance)",
        route: "raw filesystem reads and a GameFileResolver with no admission attached",
        read_sites: &[
            "game_ui/src/bin/acceptance.rs",
            "game_ui/src/probe.rs",
            "map_viewer/src/bin/dump_bloom_validation.rs",
            "map_viewer/src/bin/dump_map_labels.rs",
            "map_viewer/src/cap4_sea_referee.rs",
            "map_viewer/src/cli.rs",
            "map_viewer/src/cpu_terrain.rs",
            "map_viewer/src/d3d11_device.rs",
            "map_viewer/src/fow_law.rs",
            "map_viewer/src/frame_bundle.rs",
            "map_viewer/src/json_store.rs",
            "map_viewer/src/political_color_synthesis.rs",
            "map_viewer/src/restore_evidence.rs",
            "map_viewer/src/terrain_chunk_library.rs",
            "map_viewer/src/terrain_id_map_synthesis.rs",
            "map_viewer/src/tree_second_subdraw.rs",
            "replay_app/src/bundle.rs",
            "replay_app/src/headless.rs",
            "replay_app/src/performance_bench.rs",
            "replay_app/src/recording_reach.rs",
            "replay_app/src/render.rs",
            "replay_core/src/bin/make_demo_recording.rs",
        ],
        logical_path_examples: &[],
        validation: Validation::InstallReadUnpinnedDebugOnly,
        mismatch:
            "unreachable while a production recording is open: --bundle, --capture and every \
             other debug surface is refused by the blocker registry under \
             Scope::ProductionRecording, and the Static1936 baseline carries no recording to \
             mismatch against",
    },
    LedgerRow {
        class: EVIDENCE_USER_DATA_STAGING,
        subject: "user-data files read outside the effective VFS by the evidence entry points: \
                  static_mod_staging_manifest.json, ui14_engine_vfs.json, and each staged \
                  mod/DLC descriptor the staging manifest names",
        route: "AssetResolver::from_runtime_userdir reads the two receipts directly and mounts \
                the staged content roots they name; push_staged_layer re-hashes each descriptor \
                against the staging manifest's descriptor_sha256 and refuses on disagreement",
        read_sites: &[
            "game_ui/src/assets.rs",
            "game_ui/src/mod_ui_discovery.rs",
            "game_ui/src/scripted_gui_runtime.rs",
        ],
        logical_path_examples: &[
            "static_mod_staging_manifest.json",
            "ui14_engine_vfs.json",
        ],
        validation: Validation::InstallReadUnpinnedDebugOnly,
        mismatch:
            "no recording pins these: the staging manifest binds the UI14 receipt by digest and \
             each descriptor by digest, but nothing ties either to a recording, so a stale \
             receipt describes a mod set the capture never mounted. Unreachable from playback: \
             the only callers are discover_runtime_mod_ui_from_userdir_for_evidence and \
             discover_effective_scripted_guis, and every product entry point \
             (discover_admitted_runtime_mod_ui, ProductionContentInterpreter) refuses a resolver \
             without a portable content admission",
    },
    LedgerRow {
        class: RESOLVED_PATH_DECODER,
        subject: "the bytes of an install file, at a path a resolver already resolved: texture \
                  decoders, the image loader, the Clausewitz text reader and the definition \
                  loader",
        route: "a raw filesystem read of a &Path parameter; these modules never resolve, so they \
                cannot choose which install file they open",
        read_sites: &[
            "game_ui/src/dds.rs",
            "game_ui/src/paradox_text.rs",
            "game_ui/src/png.rs",
            "game_ui/src/tga.rs",
            "map_viewer/src/image_load.rs",
            "map_viewer/src/util.rs",
        ],
        logical_path_examples: &[],
        validation: Validation::InstallReadPinned,
        mismatch:
            "the second hop of an already-pinned read: the admission verified the payload sha256 \
             and byte length of the file at that exact path before the decoder was handed it, so \
             a decoder cannot open a file the recording did not pin",
    },
    LedgerRow {
        class: FROZEN_CENSUS_ROUTE,
        subject: "interface/**.gui addressed from a RAW game root, not through a resolver: the \
                  XUI01 frozen playset",
        route: "frozen_playset_files joins the census rows onto a game_root argument and \
                load_frozen_definition_catalog reads them",
        read_sites: &["game_ui/src/definition_admission.rs"],
        logical_path_examples: &[],
        validation: Validation::InstallReadUnpinnedDebugOnly,
        mismatch:
            "not silent, but NOT pinned to the recording: the frozen census compares byte length \
             and sha256 per file against a table compiled for hoi4.exe 1.19.2.0.a729, so another \
             build is refused by name while a recording made on that same build is never \
             consulted. No product route reaches it - the only callers in either workspace are \
             game_ui's own integration tests",
    },
    LedgerRow {
        class: REPLAY_CONTAINER,
        subject: "not a game file: the .battleplan container, its spool and the rollover package \
                  set. Listed so the raw-read gate can account for every module that reads bytes",
        route: "raw filesystem reads of the recording's own storage",
        read_sites: &[
            "replay_app/src/main.rs",
            "replay_app/src/prefold_worker.rs",
            "replay_app/src/recording_open.rs",
            "replay_app/src/windows_spool_storage.rs",
            "replay_core/src/rollover_package_set.rs",
            "replay_core/src/source.rs",
        ],
        logical_path_examples: &[],
        validation: Validation::InstallReadPinned,
        mismatch:
            "no install mismatch is possible: the SINGLE-FILE LAW makes the container the only \
             extractor-to-replay connection, and it carries its own seal",
    },
    LedgerRow {
        class: PASS_THROUGH_READER,
        subject: "no class of its own: these modules read an already-resolved UsedContentKey, a \
                  path their caller chose, or - for the resolver itself - every path the \
                  recording's required_content names, of every class at once",
        route: "read_admitted_used_content, a resolver call whose path is a parameter, or \
                playback_inventory / with_content_admission walking the recorded required set",
        read_sites: &[
            "game_ui/src/assets.rs",
            "game_ui/src/gui_scene_util.rs",
            "game_ui/src/occupation_compositor.rs",
            "map_viewer/src/border_texture_law.rs",
            "map_viewer/src/camera.rs",
            "map_viewer/src/counter_render.rs",
            "map_viewer/src/d3d11_replay.rs",
            "map_viewer/src/production_d3d.rs",
            "map_viewer/src/production_d3d/native.rs",
            "map_viewer/src/production_d3d/native_draw.rs",
            "map_viewer/src/mod_resolver.rs",
            "map_viewer/src/production_features/xmap09.rs",
            "map_viewer/src/production_features/xmap20.rs",
            "map_viewer/src/production_map_renderer.rs",
            "map_viewer/src/terrain_lod_law.rs",
            "map_viewer/src/tree_instance_synthesis.rs",
            "map_viewer/src/xmap06_d3d_handoff.rs",
            "map_viewer/src/xmap08_d3d_handoff.rs",
            "map_viewer/src/xmap11_d3d_handoff.rs",
            "map_viewer/src/xmap12_d3d_handoff.rs",
            "map_viewer/src/xmap12_pdxmesh.rs",
            "map_viewer/src/xmap15_d3d_handoff.rs",
            "map_viewer/src/xmap16_d3d_handoff.rs",
            "map_viewer/src/xmap16_hub_handoff.rs",
            "replay_app/src/combat_gpu.rs",
            "replay_app/src/counter_gpu.rs",
            "replay_app/src/furniture_render.rs",
            "replay_app/src/map_labels.rs",
            "replay_app/src/submitted_draw.rs",
        ],
        logical_path_examples: &[],
        validation: Validation::InstallReadPinned,
        mismatch:
            "a pass-through cannot mismatch on its own: read_admitted_used_content refuses a key \
             the admission did not record, and a path parameter has already been classified by \
             the caller that chose it",
    },
    LedgerRow {
        class: CONTENT_ROOT,
        subject: "the directory roots themselves, not files under them: game directory, \
                  the game user-data directory, and the three layer roots the launcher selection \
                  names (integrated DLC content, descriptor content, mod content)",
        route: "require_dir admits a root; every path in the admitted-content rows above is \
                resolved through the layer stack these roots build",
        read_sites: &[
            "map_viewer/src/mod_resolver.rs",
            "map_viewer/src/bundle_gen.rs",
            "map_viewer/src/util.rs",
            "replay_app/src/product_entry.rs",
        ],
        logical_path_examples: &[],
        validation: Validation::InstallReadPinned,
        mismatch:
            "a root that holds different content cannot pass unnoticed: the admission compares \
             the winning layer ordinal, kind, id and source-container sha256 for every used \
             path, so a root swapped for another install refuses at open by layer identity \
             rather than by the files it happens to hold",
    },
    LedgerRow {
        class: SHADER_CACHE,
        subject: "the *.scache files the game writes when it compiles its shaders, read for \
                  MAP_SIZE, MAP_POW2, FOW_POW2 and the TerrainIDMap sampler slot",
        route: "map_viewer's dev CLI only: scan-shader-cache and the --shader-cache arm of the \
                runtime pow2 source, both taking an operator-supplied directory, both reading \
                by path outside any resolver",
        read_sites: &["map_viewer/src/shader_meta.rs", "map_viewer/src/cli.rs"],
        logical_path_examples: &[],
        validation: Validation::InstallReadUnpinnedDebugOnly,
        mismatch:
            "no recording pins a shader cache and no product route opens one. Constants \
             harvested from a cache another install compiled would describe a different map \
             size; what carries them into the tree is a committed runtime-import artifact whose \
             extraction string names the cache file and sampler slot it came from \
             (shader_cache_define:<file>:<symbol>), so the origin is stated rather than assumed",
    },
    LedgerRow {
        class: AUDIO,
        subject: "the sound definition namespace and the wav payloads it names: \
                  sound/**/*.asset from the base tree, from integrated_dlc/*/sound and from the \
                  dlc/ overlay, and every wav a bound soundeffect resolves to",
        route: "GameFileResolver under the recording's ValidatedContentAdmission. \
                SoundBank::from_resolver requires the WHOLE sound/**/*.asset namespace to be \
                admitted before it binds anything, so a partial admission refuses rather than \
                binding a base definition a mod had overridden; each wav is then resolved by the \
                logical path its own defining .asset carries",
        read_sites: &["replay_app/src/world_audio.rs", "replay_app/src/world_audio_law.rs"],
        logical_path_examples: &[
            "sound/soundeffects.asset",
            "sound/category.asset",
            "sound/falloff.asset",
            "sound/animations/amb_battle_distant_01.wav",
            "sound/animations/amb_battle_distant_02.wav",
        ],
        validation: Validation::InstallReadPinned,
        mismatch: PINNED_MISMATCH,
    },
];

/// Which ledger class an admitted logical path belongs to.
pub fn classify_admitted_path(logical_path: &str) -> &'static str {
    let path = logical_path.replace('\\', "/").to_ascii_lowercase();
    // A bare directory root classifies with the files under it: several read
    // sites name the directory and enumerate it.
    let starts = |prefix: &str| path.starts_with(prefix) || path == prefix.trim_end_matches('/');
    if starts("gfx/flags/") {
        return COUNTRY_FLAG;
    }
    if starts("gfx/leaders/") || starts("gfx/portraits/") || starts("gfx/models/portraits/") {
        return PORTRAIT;
    }
    if starts("gfx/fonts/") {
        return FONT;
    }
    if starts("gfx/fx/") {
        return SHADER;
    }
    if starts("gfx/models/") || starts("gfx/entities/") {
        return if path.ends_with(".shader") || path.ends_with(".fxh") {
            SHADER
        } else {
            MODEL_MESH
        };
    }
    if starts("gfx/interface/") || starts("gfx/texticons/") || starts("gfx/particles/") {
        return INTERFACE_TEXTURE;
    }
    if starts("gfx/world/") || starts("gfx/maparrows/") || path == "gfx/posteffect_volumes.txt" {
        return WORLD_GFX_TABLE;
    }
    if starts("map/terrain/") {
        return MAP_TERRAIN_TEXTURE;
    }
    if starts("map/") || starts("history/") {
        return MAP_GEOMETRY;
    }
    if starts("interface/") {
        return INTERFACE_DEFINITION;
    }
    if starts("localisation/") {
        return LOCALISATION;
    }
    if starts("common/") {
        return SCRIPT_DEFINITION;
    }
    if starts("sound/") || starts("music/") {
        return AUDIO;
    }
    if starts("events/") {
        return EVENT_DEFINITION;
    }
    OTHER_ADMITTED
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn row(class: &str) -> Option<&'static LedgerRow> {
    GAME_FILE_LEDGER.iter().find(|entry| entry.class == class)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackagingVerdict {
    /// At least one input existed only inside the running engine, so no install
    /// read can reproduce it.
    CaptureOnlyInput,
    /// The payload is a join over authored installed content the admitted install
    /// already carries. The capture law says capture records changing semantic
    /// state, "not authored content already available in the certified
    /// effective installed content" (PROJECT.md), so this belongs install-read and
    /// pinned. The row states what blocks moving it.
    ShouldBeInstallReadPinned,
}

/// What each packaged artifact is DERIVED FROM, the verdict on whether
/// packaging it is right, and - for the ones that should move - what blocks it.
/// Checked against each generator, not asserted from the shape of the roster:
/// every one of the eight builds from mounted content layers plus the
/// English-only localisation join, and each generator's own scope block
/// EXCLUDES its runtime state.
/// The generators named below are the acquisition side's game_defs gates,
/// one .py per catalog: extractor/game_defs/<name>.py.
pub const PACKAGED_STATIC_ARTIFACT_RATIONALE: &[(&str, PackagingVerdict, &str)] = &[
    (
        "templates_equipment_catalog",
        PackagingVerdict::ShouldBeInstallReadPinned,
        "parsed from mounted content layers by the domain-59 lane gate, which states that live \
         template mutation still needs a DLL registration plan it does not have. What is \
         packaged is authored. Blocked on: the replay has no Clausewitz parser for division \
         templates and equipment, and the recording's own division rosters key into it",
    ),
    (
        "localisation_join",
        PackagingVerdict::ShouldBeInstallReadPinned,
        "authored localisation strings the admitted install already carries - name, raw, \
         source_file and source_layer per key - joined in ENGLISH ONLY \
         (localisation_gate.py states english-only scope). Every other \
         packaged catalog is joined against this one, so English is what the whole packaged set \
         displays. Blocked on: game_ui::static_research's focus, idea and technology label \
         lookups would have to read the admitted localisation instead. Until then the open \
         statement names the substitution to any non-English viewer",
    ),
    (
        "decisions_static_definitions",
        PackagingVerdict::ShouldBeInstallReadPinned,
        "built from mounted content layers plus the localisation join; the generator's own scope \
         block EXCLUDES runtime decision state and trigger execution \
         (decisions_static_gate.py). Blocked on: the replay has no decisions \
         parser, and the recording's decision frames key into this catalog",
    ),
    (
        "idea_tech_loc_xref_static",
        PackagingVerdict::ShouldBeInstallReadPinned,
        "built from mounted content layers plus the localisation join; the generator EXCLUDES \
         runtime ownership and research state (idea_tech_static_gate.py). \
         Blocked on: the replay has no idea/technology definition parser",
    ),
    (
        "ship_definitions_static",
        PackagingVerdict::ShouldBeInstallReadPinned,
        "parses authored hull and module files; the generator EXCLUDES runtime module \
         composition (ship_definitions_gate.py). Blocked on: the replay has \
         no ship-definition parser",
    ),
    (
        "gfx_icon_asset_manifest_static",
        PackagingVerdict::ShouldBeInstallReadPinned,
        "scans interface and flag files across the mounted layer stack \
         (gfx_icon_manifest_gate.py); it carries no live-engine payload. Its \
         one capture-shaped input is the DLC roster, which the replay reconstructs anyway. \
         Blocked on: nothing in the replay reads it yet, so moving it has no consumer to serve",
    ),
    (
        "technology_sharing_static",
        PackagingVerdict::ShouldBeInstallReadPinned,
        "definition id, picture, bonus and category from mounted layers, plus localisation. The \
         generator's scope block puts the runtime intern-ordinal mapping in not_closed_rows \
         (technology_sharing_static_gate.py), so the artifact does NOT carry \
         it. Blocked on: the replay has no technology-sharing parser",
    ),
    (
        "focus_defs_static",
        PackagingVerdict::ShouldBeInstallReadPinned,
        "parses the mounted focus files; the generator EXCLUDES runtime participant selection \
         (focus_defs_static_gate.py). Blocked on: the replay has no focus-tree \
         parser, and focus_defs_static is the one artifact whose absence refuses playback outright",
    ),
];

/// Every packaged static artifact, and what a viewer loses without it.
pub const PACKAGED_STATIC_ARTIFACT_LOSS: &[(&str, &str)] = &[
    (
        "templates_equipment_catalog",
        "division-template and equipment display names in the unit UI",
    ),
    (
        "localisation_join",
        "the static localisation join behind research and idea display names",
    ),
    (
        "decisions_static_definitions",
        "decision and decision-category definitions in the decision panel",
    ),
    (
        "idea_tech_loc_xref_static",
        "idea and technology display names",
    ),
    ("ship_definitions_static", "ship hull and module names"),
    (
        "gfx_icon_asset_manifest_static",
        "the sprite-to-texture icon manifest; no replay consumer reads it yet, so today the loss \
         is provenance only",
    ),
    (
        "technology_sharing_static",
        "technology-sharing group display",
    ),
    ("focus_defs_static", "the national focus tree"),
];

fn artifact_loss(name: &str) -> &'static str {
    PACKAGED_STATIC_ARTIFACT_LOSS
        .iter()
        .find(|(artifact, _)| *artifact == name)
        .map(|(_, loss)| *loss)
        .unwrap_or("an unnamed packaged catalog")
}

/// The open-time statement: what THIS recording pinned, per ledger class, and
/// every named degradation. Printed next to the content-compatibility summary
/// so a matching install is never indistinguishable from a partial one.
pub fn open_statement(
    admission: &ValidatedContentAdmission,
    product: Option<&RecorderProductAdmission>,
    render_settings: &RenderSettingsPin,
    localisation_language: &str,
) -> String {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut total = 0usize;
    for path in admission.required_paths() {
        *counts.entry(classify_admitted_path(path)).or_insert(0) += 1;
        total += 1;
    }
    render_statement(&counts, total, product, render_settings, localisation_language)
}

/// True when the statement names something the viewer loses, so a route with no
/// console can raise it instead of writing it into a void.
pub fn statement_names_a_degradation(statement: &str) -> bool {
    statement.lines().any(|line| line.starts_with("DEGRADED"))
}

pub fn degradation_lines(statement: &str) -> String {
    statement
        .lines()
        .filter(|line| line.starts_with("DEGRADED"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// Every call shape that can reach a game-install file.
    const INSTALL_READ_PRIMITIVES: &[&str] = &[
        "resolve_required(",
        "resolve_required_with_provenance(",
        "resolve_with_provenance(",
        "read_admitted_used_content(",
        "resolve_optional_file(",
        "resolve_dir_files(",
        "resolve_dir_files_recursive(",
        "resolve_foundation_content(",
        "resolve_used_content(",
        "list_files_recursive(",
        "decode_texture(",
        "decode_sprite(",
        "resolve_texture_file(",
        "resolve_file(",
    ];

    /// `resolver.resolve(path)` is the generic shape. Unrelated types in this
    /// tree also carry a zero-argument `.resolve()`; only a call WITH an
    /// argument can name a game file.
    fn reads_install_content(code: &str) -> bool {
        if INSTALL_READ_PRIMITIVES
            .iter()
            .any(|primitive| code.contains(primitive))
        {
            return true;
        }
        let mut rest = code;
        while let Some((_, tail)) = rest.split_once(".resolve(") {
            rest = tail;
            if !tail.trim_start().starts_with(')') {
                return true;
            }
        }
        false
    }

    /// THE READ-SITE CENSUS: every production file that reads game-install
    /// content through the resolver. The sweep below must equal it exactly, so a
    /// module that starts reading install content cannot appear without a
    /// deliberate edit here and a look at the ledger row that covers it.
    const READ_SITE_CENSUS: &[&str] = &[
        "game_ui/src/active_abilities.rs",
        "game_ui/src/assets.rs",
        "game_ui/src/authored_role_index.rs",
        "game_ui/src/character_ui.rs",
        "game_ui/src/combat_detail_canvas.rs",
        "game_ui/src/compose.rs",
        "game_ui/src/content_interpreter.rs",
        "game_ui/src/decision_chrome.rs",
        "game_ui/src/designer_observer.rs",
        "game_ui/src/event_chrome.rs",
        "game_ui/src/focus_observer.rs",
        "game_ui/src/font.rs",
        "game_ui/src/gui_scene_util.rs",
        "game_ui/src/hud.rs",
        "game_ui/src/hud_role_index.rs",
        "game_ui/src/loc.rs",
        "game_ui/src/major_views.rs",
        "game_ui/src/markup.rs",
        "game_ui/src/mod_ui_discovery.rs",
        "game_ui/src/occupation_binding.rs",
        "game_ui/src/occupation_compositor.rs",
        "game_ui/src/peace_conference_ui.rs",
        "game_ui/src/power_balance.rs",
        "game_ui/src/power_balance_compositor.rs",
        "game_ui/src/probe.rs",
        "game_ui/src/production_lines.rs",
        "game_ui/src/production_renderer.rs",
        "game_ui/src/research_compositor.rs",
        "game_ui/src/runtime_graphics.rs",
        "game_ui/src/scripted_gui_runtime.rs",
        "game_ui/src/tooltip_compositor.rs",
        "game_ui/src/topbar.rs",
        "game_ui/src/trade_compositor.rs",
        "game_ui/src/trade_layout.rs",
        "map_viewer/src/bloom.rs",
        "map_viewer/src/border_law.rs",
        "map_viewer/src/border_pass.rs",
        "map_viewer/src/border_texture_law.rs",
        "map_viewer/src/border_world_synthesis.rs",
        "map_viewer/src/building_files.rs",
        "map_viewer/src/bundle_gen.rs",
        "map_viewer/src/camera.rs",
        "map_viewer/src/color_lut.rs",
        "map_viewer/src/combat_indicator_assets.rs",
        "map_viewer/src/counter_render.rs",
        "map_viewer/src/d3d11_replay.rs",
        "map_viewer/src/frame_bundle.rs",
        "map_viewer/src/game_data.rs",
        "map_viewer/src/grading_law.rs",
        "map_viewer/src/map_furniture.rs",
        "map_viewer/src/map_icon_runtime.rs",
        "map_viewer/src/map_mode_admin.rs",
        "map_viewer/src/map_mode_terrain_resource.rs",
        "map_viewer/src/map_static_data.rs",
        "map_viewer/src/map_text.rs",
        "map_viewer/src/mapmode_air.rs",
        "map_viewer/src/mapmode_factions.rs",
        "map_viewer/src/mapmode_infrastructure.rs",
        "map_viewer/src/mod_resolver.rs",
        "map_viewer/src/naval_supremacy_tint.rs",
        "map_viewer/src/o05_fixed_object_law.rs",
        "map_viewer/src/occupation_mapmode.rs",
        "map_viewer/src/pdx_mesh.rs",
        "map_viewer/src/political_color_synthesis.rs",
        "map_viewer/src/posteffect_region_weights.rs",
        "map_viewer/src/production_d3d.rs",
        "map_viewer/src/production_d3d/native.rs",
        "map_viewer/src/production_d3d/native_draw.rs",
        "map_viewer/src/production_features/xmap04.rs",
        "map_viewer/src/production_features/xmap05.rs",
        "map_viewer/src/production_features/xmap06.rs",
        "map_viewer/src/production_features/xmap07.rs",
        "map_viewer/src/production_features/xmap08.rs",
        "map_viewer/src/production_features/xmap09.rs",
        "map_viewer/src/production_features/xmap10.rs",
        "map_viewer/src/production_features/xmap12.rs",
        "map_viewer/src/production_features/xmap13.rs",
        "map_viewer/src/production_features/xmap14.rs",
        "map_viewer/src/production_features/xmap15.rs",
        "map_viewer/src/production_features/xmap17.rs",
        "map_viewer/src/production_features/xmap18.rs",
        "map_viewer/src/production_features/xmap19.rs",
        "map_viewer/src/production_features/xmap20.rs",
        "map_viewer/src/production_foundation.rs",
        "map_viewer/src/production_map_renderer.rs",
        "map_viewer/src/province_positions.rs",
        "map_viewer/src/rail_files.rs",
        "map_viewer/src/reflection_cube_law.rs",
        "map_viewer/src/river_mesh_synthesis.rs",
        "map_viewer/src/river_world_synthesis.rs",
        "map_viewer/src/seasons.rs",
        "map_viewer/src/shader_compile.rs",
        "map_viewer/src/shadow_law.rs",
        "map_viewer/src/snow_law.rs",
        "map_viewer/src/terrain_id_map_synthesis.rs",
        "map_viewer/src/terrain_lod_law.rs",
        "map_viewer/src/terrain_mesh_law.rs",
        "map_viewer/src/tree_files.rs",
        "map_viewer/src/tree_instance_synthesis.rs",
        "map_viewer/src/tree_runtime_selection.rs",
        "map_viewer/src/underwater_pass.rs",
        "map_viewer/src/unit_counter_assets.rs",
        "map_viewer/src/weather_law.rs",
        "map_viewer/src/xmap04_d3d_handoff.rs",
        "map_viewer/src/xmap06_d3d_handoff.rs",
        "map_viewer/src/xmap07_d3d_handoff.rs",
        "map_viewer/src/xmap08_d3d_handoff.rs",
        "map_viewer/src/xmap10_d3d_handoff.rs",
        "map_viewer/src/xmap11_d3d_handoff.rs",
        "map_viewer/src/xmap12_d3d_handoff.rs",
        "map_viewer/src/xmap12_pdxmesh.rs",
        "map_viewer/src/xmap12_population.rs",
        "map_viewer/src/xmap15_d3d_handoff.rs",
        "map_viewer/src/xmap16_d3d_handoff.rs",
        "map_viewer/src/xmap16_hub_handoff.rs",
        "map_viewer/src/xmap16_supply_arrow_handoff.rs",
        "replay_app/src/app.rs",
        "replay_app/src/combat_gpu.rs",
        "replay_app/src/content_compatibility.rs",
        "replay_app/src/counter_gpu.rs",
        "replay_app/src/font.rs",
        "replay_app/src/furniture_render.rs",
        "replay_app/src/gui_definition_admission.rs",
        "replay_app/src/map_labels.rs",
        "replay_app/src/political_scene.rs",
        "replay_app/src/submitted_draw.rs",
        "replay_app/src/world_audio.rs",
        "replay_app/src/world_audio_law.rs",
    ];

    /// Top-level directories of a the game install. A literal starting with one of
    /// these is a game-relative path and must classify into a named ledger row.
    const INSTALL_ROOT_DIRECTORIES: [&str; 8] = [
        "gfx",
        "map",
        "common",
        "interface",
        "localisation",
        "history",
        "events",
        "music",
    ];

    /// Constructing a resolver that is NOT bound to a recording's admission.
    /// Every `GameFileResolver` constructor is here, including the two that
    /// take a recorded content ORDER: those restrict the layer stack but set
    /// `admission: None`, so they verify no digest. The `Self::` forms cover
    /// the resolvers' own definition modules, where the constructors delegate.
    const UNPINNED_RESOLVER_CONSTRUCTORS: &[&str] = &[
        "GameFileResolver::vanilla(",
        "GameFileResolver::from_options(",
        "GameFileResolver::from_dlc_load(",
        "GameFileResolver::with_mod_paths(",
        "GameFileResolver::from_recorded_content_layers(",
        "GameFileResolver::from_recorded_content_order(",
        "AssetResolver::new(",
        "AssetResolver::from_runtime_userdir(",
        "Self::vanilla(",
        "Self::from_options(",
        "Self::from_dlc_load(",
        "Self::with_mod_paths(",
        "Self::from_recorded_content_layers(",
        "Self::from_recorded_content_order(",
        "Self::from_runtime_userdir(",
    ];

    /// Files allowed to construct an unpinned resolver, HOW MANY times, and
    /// why. The count is what makes the exemption per-occurrence: a new
    /// construction inside an allowlisted file changes it and fails the gate.
    const UNPINNED_RESOLVER_ALLOWLIST: &[(&str, usize, &str)] = &[
        (
            "map_viewer/src/mod_resolver.rs",
            4,
            "the resolver's own definition module: vanilla, with_mod_paths and from_dlc_load \
             delegate to from_options, and from_recorded_content_order to \
             from_recorded_content_layers",
        ),
        (
            "map_viewer/src/cli.rs",
            1,
            "map_viewer dev CLI; carries no recording",
        ),
        (
            "map_viewer/src/bin/dump_bloom_validation.rs",
            2,
            "dev binary; carries no recording. Two: a vanilla arm and a --mod arm",
        ),
        (
            "map_viewer/src/bin/dump_border_synthesis.rs",
            2,
            "dev binary; carries no recording. Two: a vanilla arm and a --mod arm",
        ),
        (
            "map_viewer/src/bin/dump_map_labels.rs",
            2,
            "dev binary; carries no recording. Two: a vanilla arm and a --mod arm",
        ),
        (
            "replay_app/src/content_compatibility.rs",
            1,
            "the pinned route's own first step: resolver_for_manifest builds the recording's \
             layer order and admit_content binds the admission to it before any read",
        ),
        (
            "game_ui/src/probe.rs",
            1,
            "game_ui evidence probe binary; carries no recording",
        ),
        (
            "game_ui/src/mod_ui_discovery.rs",
            1,
            "the *_for_evidence entry point; the product entry point requires admission",
        ),
        (
            "game_ui/src/scripted_gui_runtime.rs",
            1,
            "the *_effective evidence entry point; the product entry point takes an admitted \
             AssetResolver",
        ),
        (
            "replay_app/src/install_paths.rs",
            1,
            "profile validation: resolves the launcher selection before any recording is open",
        ),
        (
            "replay_app/src/main.rs",
            1,
            "builds the resolver for the debug baselines; every recording route takes the \
             admission-bound one from bind_provider_content, and the blocker registry refuses \
             those surfaces while a production recording is open",
        ),
        (
            "replay_app/src/performance_bench.rs",
            1,
            "attaches the admission on the same expression",
        ),
    ];

    /// Every directory ROOT production code admits through `require_dir`, and
    /// the ledger class that accounts for what is read under it.
    const DIRECTORY_ROOTS: &[(&str, &str)] = &[
        ("game directory", CONTENT_ROOT),
        ("the game user-data directory", CONTENT_ROOT),
        ("integrated DLC content directory", CONTENT_ROOT),
        ("descriptor content directory", CONTENT_ROOT),
        ("mod content directory", CONTENT_ROOT),
        ("shader cache directory", SHADER_CACHE),
        ("ProvinceSecondaryColorMap capture directory", DEBUG_EVIDENCE_INPUT),
        ("TerrainIDMap capture directory", DEBUG_EVIDENCE_INPUT),
    ];

    /// Turning an install ROOT into a path: the game directory, and the
    /// user-data directory the launcher selection, the profile and the staged
    /// layers live in. Both are roots a raw `fs::read` could descend from
    /// without touching a resolver.
    const GAME_ROOT_USES: &[&str] = &[
        "game_dir()",
        ".game_install",
        ".user_data",
        "user_dir()",
        // Joining onto a root passed in as a bare path, which is how a module
        // addresses install files without ever holding a resolver.
        "game_root.join(",
        "vanilla_root.join(",
        "user_dir.join(",
    ];

    /// Files allowed to turn an install root into a path, and how many times.
    const GAME_ROOT_USE_ALLOWLIST: &[(&str, usize)] = &[
        ("game_ui/src/definition_admission.rs", 1),
        ("game_ui/src/mod_ui_discovery.rs", 2),
        ("map_viewer/src/bin/dump_bloom_validation.rs", 1),
        ("replay_app/src/font.rs", 2),
        ("replay_app/src/install_paths.rs", 13),
        ("replay_app/src/content_compatibility.rs", 2),
        ("replay_app/src/main.rs", 4),
        ("replay_app/src/product_entry.rs", 2),
    ];

    fn workspace() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("replay workspace")
            .to_path_buf()
    }

    /// A file is test-only when its own name says so, when it sits under a
    /// `tests` directory, or when the module that pulls it in is `#[cfg(test)]`:
    /// an out-of-line `#[cfg(test)] mod foo;` makes the whole of `foo.rs` test
    /// code, and a name-based rule alone cannot see that.
    fn is_test_module(path: &Path, test_only_module_files: &BTreeSet<PathBuf>) -> bool {
        matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("tests.rs") | Some("test_support.rs")
        ) || path.components().any(|part| part.as_os_str() == "tests")
            || test_only_module_files.contains(path)
    }

    /// Every source file pulled in by a `#[cfg(test)] mod <name>;` declaration,
    /// resolved to `<name>.rs` and `<name>/mod.rs` beside the declaring file.
    fn test_only_module_files() -> BTreeSet<PathBuf> {
        let mut out = BTreeSet::new();
        for path in source_files() {
            let source = std::fs::read_to_string(&path).expect("read replay crate source");
            let mut state = LiteralScan::default();
            let mut pending = false;
            for line in source.lines() {
                let code = code_only(line, &mut state);
                if test_only_cfg(line).is_some() {
                    pending = true;
                    continue;
                }
                let trimmed = code.trim();
                if trimmed.is_empty() || trimmed.starts_with("#[") {
                    continue;
                }
                if pending {
                    if let Some(name) = trimmed
                        .strip_prefix("mod ")
                        .or_else(|| trimmed.strip_prefix("pub mod "))
                        .and_then(|rest| rest.strip_suffix(';'))
                    {
                        let dir = path.parent().expect("module directory");
                        out.insert(dir.join(format!("{}.rs", name.trim())));
                        out.insert(dir.join(name.trim()).join("mod.rs"));
                    }
                }
                pending = false;
            }
        }
        out
    }

    /// Every `.rs` file in the four replay crates, before any test filtering.
    fn source_files() -> Vec<PathBuf> {
        let root = workspace();
        let mut out = Vec::new();
        for crate_dir in ["map_viewer", "game_ui", "replay_app", "replay_core"] {
            let mut stack = vec![root.join(crate_dir).join("src")];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).expect("read replay crate source directory") {
                    let path = entry.expect("source directory entry").path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                        out.push(path);
                    }
                }
            }
        }
        out.sort();
        out
    }

    /// The predicate with every `not(...)` group removed. `not(test)` is the
    /// negation of the token, and a scan that reads it as the token itself
    /// deletes production code from the sweep.
    fn without_negations(predicate: &str) -> String {
        let chars = predicate.chars().collect::<Vec<_>>();
        let mut out = String::with_capacity(predicate.len());
        let mut index = 0usize;
        while index < chars.len() {
            if chars[index..].starts_with(&['n', 'o', 't', '(']) {
                let mut depth = 0i64;
                let mut probe = index + 3;
                while probe < chars.len() {
                    match chars[probe] {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    probe += 1;
                }
                index = probe + 1;
                continue;
            }
            out.push(chars[index]);
            index += 1;
        }
        out
    }

    /// `#[cfg(test)]` and `#[cfg(all(test, ...))]` are test-only.
    /// `#[cfg(any(windows, test))]` and `#[cfg(all(windows, not(test)))]` are
    /// production and stay in the sweep.
    fn test_only_cfg(line: &str) -> Option<&str> {
        let indent_len = line.len() - line.trim_start().len();
        let (indent, rest) = line.split_at(indent_len);
        let predicate = without_negations(rest.strip_prefix("#[cfg(")?.strip_suffix(")]")?);
        let test_only = predicate == "test"
            || (predicate.starts_with("all(")
                && predicate
                    .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                    .any(|token| token == "test"));
        test_only.then_some(indent)
    }

    #[derive(Default)]
    struct LiteralScan {
        block_comment: bool,
        string: bool,
        raw_string: Option<usize>,
    }

    /// The line with every comment, string and char literal blanked, so brace
    /// counting is exact. Clausewitz fixtures in this tree routinely hold a `}`
    /// in column 0 inside a raw string, and an indentation- or truncation-based
    /// sweep reads those as the end of a block.
    fn code_only(line: &str, state: &mut LiteralScan) -> String {
        let chars = line.chars().collect::<Vec<_>>();
        let mut out = String::with_capacity(line.len());
        let mut index = 0usize;
        while index < chars.len() {
            let ch = chars[index];
            if state.block_comment {
                if ch == '*' && chars.get(index + 1) == Some(&'/') {
                    state.block_comment = false;
                    index += 2;
                } else {
                    index += 1;
                }
                out.push(' ');
                continue;
            }
            if let Some(hashes) = state.raw_string {
                if ch == '"'
                    && chars[index + 1..]
                        .iter()
                        .take(hashes)
                        .filter(|value| **value == '#')
                        .count()
                        == hashes
                {
                    state.raw_string = None;
                    index += 1 + hashes;
                } else {
                    index += 1;
                }
                out.push(' ');
                continue;
            }
            if state.string {
                if ch == '\\' {
                    index += 2;
                } else if ch == '"' {
                    state.string = false;
                    index += 1;
                } else {
                    index += 1;
                }
                out.push(' ');
                continue;
            }
            if ch == '/' && chars.get(index + 1) == Some(&'/') {
                break;
            }
            if ch == '/' && chars.get(index + 1) == Some(&'*') {
                state.block_comment = true;
                index += 2;
                out.push_str("  ");
                continue;
            }
            if ch == 'r' {
                let hashes = chars[index + 1..]
                    .iter()
                    .take_while(|value| **value == '#')
                    .count();
                if chars.get(index + 1 + hashes) == Some(&'"') {
                    state.raw_string = Some(hashes);
                    index += 2 + hashes;
                    out.push(' ');
                    continue;
                }
            }
            if ch == '"' {
                state.string = true;
                index += 1;
                out.push(' ');
                continue;
            }
            if ch == '\'' {
                // A char literal, not a lifetime, only when a closing quote
                // follows one (possibly escaped) character.
                let escaped = chars.get(index + 1) == Some(&'\\');
                let close = if escaped { index + 3 } else { index + 2 };
                if chars.get(close) == Some(&'\'') {
                    out.push_str(&" ".repeat(close + 1 - index));
                    index = close + 1;
                    continue;
                }
            }
            out.push(ch);
            index += 1;
        }
        out
    }

    /// Production source only, as `(code, raw)` pairs: `code` has every literal
    /// blanked, `raw` is the original line. Worktrees see LF and the gate sees
    /// CRLF, so this walks lines, never raw byte windows.
    ///
    /// Test-only items are dropped by brace balance over `code`, at any nesting
    /// depth, so a `#[cfg(test)] mod ...` in the middle of a file does not hide
    /// the production items that follow it.
    fn production_lines(source: &str) -> Vec<(String, String)> {
        #[derive(PartialEq, Eq)]
        enum Phase {
            Normal,
            /// A test-only attribute was seen; the item it annotates has not
            /// started yet.
            PendingItem,
            /// Inside the item's header, which may span lines before its `{` or
            /// its terminating `;`.
            ItemHeader,
            Body,
        }
        let mut state = LiteralScan::default();
        let mut out = Vec::new();
        let mut depth = 0i64;
        let mut group = 0i64;
        let mut base = 0i64;
        let mut phase = Phase::Normal;
        for line in source.lines() {
            let code = code_only(line, &mut state);
            let delta = code.matches('{').count() as i64 - code.matches('}').count() as i64;
            group += code.matches('(').count() as i64 - code.matches(')').count() as i64
                + code.matches('[').count() as i64
                - code.matches(']').count() as i64;
            match phase {
                Phase::Body => {
                    depth += delta;
                    if depth <= base {
                        phase = Phase::Normal;
                    }
                    continue;
                }
                Phase::PendingItem => {
                    let trimmed = line.trim();
                    if trimmed.is_empty()
                        || trimmed.starts_with("#[")
                        || trimmed.starts_with("//")
                    {
                        depth += delta;
                        continue;
                    }
                    base = depth;
                    phase = Phase::ItemHeader;
                }
                Phase::Normal => {
                    if test_only_cfg(line).is_some() {
                        phase = Phase::PendingItem;
                        depth += delta;
                        continue;
                    }
                    out.push((code, line.to_string()));
                    depth += delta;
                    continue;
                }
                Phase::ItemHeader => {}
            }
            // The annotated item, header line by header line. It ends at the
            // brace that opens its body, or - for a bodiless item - at the `;`,
            // `,` or closing `}` that ends it. A field, an enum variant and a
            // match arm all end in a comma; a multi-line signature's commas sit
            // inside parentheses or brackets, so a terminator only counts at
            // group depth zero.
            depth += delta;
            let trimmed = code.trim_end();
            if depth > base {
                phase = Phase::Body;
            } else if group <= 0
                && (trimmed.ends_with(';') || trimmed.ends_with(',') || trimmed.ends_with('}'))
            {
                phase = Phase::Normal;
            }
        }
        out
    }

    fn production_sources() -> Vec<(String, String)> {
        let root = workspace();
        let test_only = test_only_module_files();
        let mut out = Vec::new();
        for path in source_files() {
            if is_test_module(&path, &test_only) {
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .expect("source under the replay workspace")
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path).expect("read replay crate source");
            out.push((relative, text));
        }
        out.sort();
        assert!(out.len() > 100, "the source sweep found {} files", out.len());
        out
    }

    /// kills: dropping a row's subject/route/mismatch to an empty string, or
    /// leaving a class in the UNACCOUNTED state.
    #[test]
    fn every_ledger_row_is_stated_and_accounted() {
        let mut classes = BTreeSet::new();
        for entry in GAME_FILE_LEDGER {
            assert!(
                classes.insert(entry.class),
                "ledger repeats class {}",
                entry.class
            );
            for field in [entry.class, entry.subject, entry.route, entry.mismatch] {
                assert!(!field.trim().is_empty(), "{} has a blank field", entry.class);
            }
            assert_ne!(
                entry.validation,
                Validation::Unaccounted,
                "{} is UNACCOUNTED: every game-file read must be packaged, pinned, or reachable \
                 only without a recording",
                entry.class
            );
        }
    }

    /// kills: a ledger row naming a module that no longer exists, so the row
    /// silently stops describing anything.
    #[test]
    fn every_ledger_read_site_exists() {
        let root = workspace();
        for entry in GAME_FILE_LEDGER {
            for site in entry.read_sites {
                assert!(
                    root.join(site).is_file(),
                    "{} names a read site that does not exist: {site}",
                    entry.class
                );
            }
        }
    }

    /// THE CLOSURE: every production source file that can reach a game-install
    /// file does it through a resolver, and only the named files may build one
    /// that no recording pins.
    ///
    /// kills: adding `GameFileResolver::vanilla(...)` (or an AssetResolver over
    /// raw roots) to a module a production recording can reach, which would
    /// read whatever the local install happens to hold.
    #[test]
    fn no_unlisted_module_builds_an_unpinned_resolver() {
        let allowed = UNPINNED_RESOLVER_ALLOWLIST
            .iter()
            .map(|(file, count, _)| (*file, *count))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            allowed.len(),
            UNPINNED_RESOLVER_ALLOWLIST.len(),
            "the allowlist repeats a file"
        );
        let mut offenders = Vec::new();
        let mut counted: BTreeMap<&str, usize> = BTreeMap::new();
        for (relative, text) in production_sources() {
            for (code, raw) in production_lines(&text) {
                for constructor in UNPINNED_RESOLVER_CONSTRUCTORS {
                    let uses = code.matches(constructor).count();
                    if uses == 0 {
                        continue;
                    }
                    match allowed.get(relative.as_str()) {
                        Some(_) => {
                            *counted.entry(
                                UNPINNED_RESOLVER_ALLOWLIST
                                    .iter()
                                    .find(|(file, _, _)| *file == relative)
                                    .expect("allowlisted")
                                    .0,
                            )
                            .or_insert(0) += uses;
                        }
                        None => offenders.push(format!("{relative}: {}", raw.trim())),
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these modules build a resolver no recording pins: {offenders:#?}"
        );
        for (file, expected, reason) in UNPINNED_RESOLVER_ALLOWLIST {
            let actual = counted.get(file).copied().unwrap_or(0);
            assert_eq!(
                actual, *expected,
                "{file} is allowed {expected} unpinned construction(s) ({reason}) but has \
                 {actual}"
            );
        }
    }

    /// kills: turning `resolver.game_dir()` into a read path in a new module,
    /// which walks around the admission snapshot entirely.
    #[test]
    fn no_unlisted_module_reads_through_the_game_root() {
        let allowed = GAME_ROOT_USE_ALLOWLIST
            .iter()
            .copied()
            .collect::<BTreeMap<_, _>>();
        let mut offenders = Vec::new();
        let mut counted: BTreeMap<String, usize> = BTreeMap::new();
        for (relative, text) in production_sources() {
            for (code, raw) in production_lines(&text) {
                let uses = GAME_ROOT_USES
                    .iter()
                    .map(|use_site| code.matches(use_site).count())
                    .sum::<usize>();
                if uses == 0 {
                    continue;
                }
                if allowed.contains_key(relative.as_str()) {
                    *counted.entry(relative.clone()).or_insert(0) += uses;
                } else {
                    offenders.push(format!("{relative}: {}", raw.trim()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these modules build a path from an install root, not the resolver: {offenders:#?}"
        );
        for (file, expected) in GAME_ROOT_USE_ALLOWLIST {
            let actual = counted.get(*file).copied().unwrap_or(0);
            assert_eq!(
                actual, *expected,
                "{file} is allowed {expected} install-root use(s) but has {actual}"
            );
        }
    }

    /// THE COMPLETENESS STATEMENT, and how a reviewer refutes it: the swept set
    /// of install readers must EQUAL the census. A module that starts reading
    /// install content fails here until it is added and its ledger row checked.
    ///
    /// kills: adding a read route anywhere in the three crates without touching
    /// the ledger.
    #[test]
    fn the_install_read_census_is_the_swept_closure() {
        let mut readers = BTreeSet::new();
        for (relative, text) in production_sources() {
            for (code, _raw) in production_lines(&text) {
                if reads_install_content(&code) {
                    readers.insert(relative.clone());
                    break;
                }
            }
        }
        let census = READ_SITE_CENSUS
            .iter()
            .map(|file| (*file).to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(census.len(), READ_SITE_CENSUS.len(), "the census repeats a file");
        let unlisted = readers.difference(&census).cloned().collect::<Vec<_>>();
        let departed = census.difference(&readers).cloned().collect::<Vec<_>>();
        assert!(
            unlisted.is_empty(),
            "these files read install content and the ledger census does not list them: \
             {unlisted:#?}"
        );
        assert!(
            departed.is_empty(),
            "the census lists files that no longer read install content: {departed:#?}"
        );
    }

    /// kills: the ledger naming a read site the census does not carry, which
    /// would mean the row points at a module the sweep never saw reading.
    /// Every game-relative path literal on a line: the head directory is a
    /// the game top-level directory, and the literal is either a path under it or
    /// that directory itself.
    fn game_relative_literals(raw: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut rest = raw;
        while let Some((_, tail)) = rest.split_once('"') {
            let Some((value, after)) = tail.split_once('"') else {
                break;
            };
            rest = after;
            if value.contains('\\') {
                continue;
            }
            let head = value.split('/').next().unwrap_or("").to_ascii_lowercase();
            if INSTALL_ROOT_DIRECTORIES.contains(&head.as_str()) {
                out.push(value);
            }
        }
        out
    }

    /// Reading bytes off the filesystem, whatever produced the path. The
    /// resolver census sees the FIRST hop; a decoder handed an already-resolved
    /// path is a second hop no resolver predicate can see.
    const BYTE_READ_PRIMITIVES: &[&str] = &[
        "fs::read(",
        "fs::read_to_string(",
        "fs::read_dir(",
        "fs::metadata(",
        "File::open(",
        "read_latin1(",
        "collect_files_with_extension(",
        "OpenOptions",
        "symlink_metadata(",
        "canonicalize(",
        // Asking the filesystem ABOUT a path is a read of it: an install root
        // probed for existence chooses whether the rest of a route runs.
        ".metadata()",
        ".is_dir()",
        ".is_file()",
        ".exists()",
    ];

    /// kills: sweeping an out-of-line `#[cfg(test)] mod foo;` file as
    /// production, which forces a false ledger row for a read that only ever
    /// happens under `cargo test`.
    #[test]
    fn an_out_of_line_test_module_is_not_production() {
        let test_only = test_only_module_files();
        let declared = workspace().join("replay_core/src/world_xmap19_tests.rs");
        assert!(
            test_only.contains(&declared),
            "replay_core/src/world.rs declares world_xmap19_tests behind #[cfg(test)]"
        );
        assert!(
            !production_sources()
                .iter()
                .any(|(relative, _)| relative.ends_with("world_xmap19_tests.rs")),
            "an out-of-line test module must not be swept as production"
        );
        assert!(
            production_sources()
                .iter()
                .any(|(relative, _)| relative == "replay_core/src/world.rs"),
            "the module that declares it is still production"
        );
    }

    /// The readers whose EVERY read call names a string literal. Every other
    /// reader addresses at least one path it did not spell - a parameter, a
    /// resolved key, or the recording's own required_content - and the class of
    /// those reads is decided at OPEN TIME by `classify_admitted_path` over the
    /// admission, not by anything static.
    const LITERAL_ONLY_READERS: &[&str] = &[
        "game_ui/src/markup.rs",
        "game_ui/src/power_balance.rs",
        "game_ui/src/production_lines.rs",
        "game_ui/src/scripted_gui_runtime.rs",
        "map_viewer/src/bloom.rs",
        "map_viewer/src/political_color_synthesis.rs",
    ];

    /// True when some read call in `raw` is handed something other than a
    /// string literal.
    fn addresses_a_path_it_did_not_spell(raw: &str) -> bool {
        let mut shapes = INSTALL_READ_PRIMITIVES.to_vec();
        shapes.push(".resolve(");
        for shape in shapes {
            let mut rest = raw;
            while let Some((_, tail)) = rest.split_once(shape) {
                rest = tail;
                let argument = tail.trim_start();
                if !argument.starts_with('"') && !argument.starts_with(')') {
                    return true;
                }
            }
        }
        false
    }

    /// kills: a reader that quietly starts addressing paths it does not spell -
    /// or stops - without the ledger saying which reads a static table can
    /// classify and which only the open-time admission can.
    #[test]
    fn the_readers_that_spell_every_path_they_read_are_the_named_six() {
        let mut literal_only = BTreeSet::new();
        for (relative, source) in production_sources() {
            let rows = production_lines(&source);
            if !rows.iter().any(|(code, _)| reads_install_content(code)) {
                continue;
            }
            if !rows
                .iter()
                .any(|(_, raw)| addresses_a_path_it_did_not_spell(raw))
            {
                literal_only.insert(relative);
            }
        }
        let named = LITERAL_ONLY_READERS
            .iter()
            .map(|site| (*site).to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            literal_only, named,
            "the readers whose every read names a literal must be exactly the named set"
        );
    }

    /// THE SECOND-HOP CLOSURE: every module that reads bytes off the filesystem
    /// is named by a row, whether it resolved the path itself or was handed
    /// one. The resolver census cannot see the second hop - `game_ui/src/dds.rs`
    /// decodes an admitted texture without ever calling a resolver - so without
    /// this gate a decoder could read a game file with no row describing it.
    ///
    /// kills: a new module that reads an install file at a path someone else
    /// resolved, invisible to every resolver-shaped predicate.
    #[test]
    fn every_module_that_reads_bytes_is_named_by_a_row() {
        let named = GAME_FILE_LEDGER
            .iter()
            .flat_map(|entry| entry.read_sites.iter().copied())
            .collect::<BTreeSet<_>>();
        let mut unnamed = Vec::new();
        for (relative, source) in production_sources() {
            let reads_bytes = production_lines(&source).iter().any(|(code, _)| {
                BYTE_READ_PRIMITIVES
                    .iter()
                    .any(|primitive| code.contains(primitive))
            });
            if reads_bytes && !named.contains(relative.as_str()) {
                unnamed.push(relative);
            }
        }
        assert!(
            unnamed.is_empty(),
            "these modules read bytes off the filesystem and no ledger row names them: \
             {unnamed:#?}"
        );
    }

    /// THE PER-ROW CLOSURE: every file in the census is named by at least one
    /// row, so the ledger is the enumeration the task asked for and not a
    /// sample. The reverse does not hold - rows outside the admitted classes
    /// name modules that read by path, which the resolver census does not see.
    ///
    /// kills: a module that reads install content and is counted by the census
    /// but described by no row, so nothing states what a mismatch does to it.
    #[test]
    fn every_census_file_is_named_by_a_row() {
        let named = GAME_FILE_LEDGER
            .iter()
            .flat_map(|entry| entry.read_sites.iter().copied())
            .collect::<BTreeSet<_>>();
        let unnamed = READ_SITE_CENSUS
            .iter()
            .filter(|site| !named.contains(*site))
            .collect::<Vec<_>>();
        assert!(
            unnamed.is_empty(),
            "these files read install content and no ledger row names them: {unnamed:#?}"
        );
    }

    /// kills: a row that stops naming a module whose OWN game-relative literals
    /// put it in that class - the shape that let a class drift away from the
    /// code that reads it.
    #[test]
    fn every_class_a_modules_own_paths_name_lists_that_module() {
        let mut missing = Vec::new();
        for (relative, source) in production_sources() {
            let rows = production_lines(&source);
            if !rows.iter().any(|(code, _)| reads_install_content(code)) {
                continue;
            }
            let mut classes = BTreeSet::new();
            for (_code, raw) in &rows {
                for value in game_relative_literals(raw) {
                    classes.insert(classify_admitted_path(value));
                }
            }
            for class in classes {
                let entry = row(class).unwrap_or_else(|| panic!("{class} has no ledger row"));
                if !entry.read_sites.contains(&relative.as_str()) {
                    missing.push(format!("{relative} names a {class} path"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "these modules name a class the row does not list them under: {missing:#?}"
        );
    }

    #[test]
    fn every_ledger_read_site_is_in_the_census() {
        let census = READ_SITE_CENSUS.iter().copied().collect::<BTreeSet<_>>();
        let install_read_classes = [
            MAP_GEOMETRY,
            MAP_TERRAIN_TEXTURE,
            SHADER,
            MODEL_MESH,
            INTERFACE_DEFINITION,
            INTERFACE_TEXTURE,
            COUNTRY_FLAG,
            PORTRAIT,
            FONT,
            LOCALISATION,
            SCRIPT_DEFINITION,
            EVENT_DEFINITION,
            WORLD_GFX_TABLE,
            AUDIO,
            OTHER_ADMITTED,
            PASS_THROUGH_READER,
        ];
        for class in install_read_classes {
            let entry = row(class).unwrap_or_else(|| panic!("{class} has no ledger row"));
            for site in entry.read_sites {
                assert!(
                    census.contains(site),
                    "{class} names read site {site}, which the census does not list"
                );
            }
        }
    }

    /// kills: a sweep that reads a `}` in column 0 inside a Clausewitz fixture
    /// as the end of the test module, or that treats `#[cfg(any(windows, test))]`
    /// as test-only and drops the production item it annotates.
    #[test]
    fn the_production_sweep_skips_test_items_without_losing_the_code_around_them() {
        let source = concat!(
            "pub const BEFORE: &str = \"map/before.txt\";
",
            "#[cfg(any(windows, test))]
",
            "pub fn windows_production() {
",
            "    let _ = GameFileResolver::vanilla(root);
",
            "}
",
            "#[cfg(test)]
",
            "mod tests {
",
            "    const FIXTURE: &str = r#\"outer = {
",
            "}
",
            "\"#;
",
            "    fn helper() {
",
            "        let _ = AssetResolver::new(vec![root]);
",
            "    }
",
            "}
",
            "pub const AFTER: &str = \"map/after.txt\";
",
        );
        let kept = production_lines(source)
            .into_iter()
            .map(|(_code, raw)| raw)
            .collect::<Vec<_>>()
            .join("
");
        assert!(kept.contains("BEFORE"), "{kept}");
        assert!(
            kept.contains("AFTER"),
            "a production item after the test module must survive the sweep: {kept}"
        );
        assert!(
            kept.contains("GameFileResolver::vanilla"),
            "#[cfg(any(windows, test))] is production: {kept}"
        );
        assert!(
            !kept.contains("AssetResolver::new"),
            "the test module must be dropped whole: {kept}"
        );
        assert!(!kept.contains("FIXTURE"), "{kept}");
    }

    /// A `#[cfg(test)]` item whose signature spans lines opens no brace on its
    /// first line. Skipping only that line would leave the body swept as
    /// production - `open_statement_for_tests` in this very file is that shape.
    ///
    /// kills: ending the test-item skip at the attribute's first line.
    #[test]
    fn a_multiline_test_signature_is_skipped_to_its_closing_brace() {
        let source = concat!(
            "#[cfg(test)]\n",
            "fn only_for_tests(\n",
            "    paths: &[&str],\n",
            ") -> String {\n",
            "    let _ = GameFileResolver::vanilla(root);\n",
            "    String::new()\n",
            "}\n",
            "pub fn production() {\n",
            "    let _ = AssetResolver::new(vec![root]);\n",
            "}\n",
        );
        let kept = production_lines(source)
            .into_iter()
            .map(|(_code, raw)| raw)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !kept.contains("GameFileResolver::vanilla"),
            "the multiline test item's body must be skipped: {kept}"
        );
        assert!(!kept.contains("only_for_tests"), "{kept}");
        assert!(
            kept.contains("AssetResolver::new"),
            "the production item after it must survive: {kept}"
        );

        let bodiless = concat!(
            "#[cfg(test)]\n",
            "use crate::thing::GameFileResolver;\n",
            "pub fn production() {\n",
            "    let _ = AssetResolver::new(vec![root]);\n",
            "}\n",
        );
        let kept = production_lines(bodiless)
            .into_iter()
            .map(|(_code, raw)| raw)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!kept.contains("use crate::thing"), "{kept}");
        assert!(kept.contains("AssetResolver::new"), "{kept}");
    }

    /// kills: a bodiless test-only item that ends in a COMMA - a struct field,
    /// an enum variant, a match arm - swallowing the production code after it,
    /// so an install read inside that region is invisible to every gate above.
    #[test]
    fn a_comma_terminated_test_item_ends_at_its_comma() {
        let field = concat!(
            "struct Thing {\n",
            "    #[cfg(test)]\n",
            "    definitions: Option<Vec<Def>>,\n",
            "    catalog: Catalog,\n",
            "}\n",
            "impl Thing {\n",
            "    pub fn load(&self) {\n",
            "        let _ = AssetResolver::new(vec![root]);\n",
            "    }\n",
            "}\n",
        );
        let kept = production_lines(field)
            .into_iter()
            .map(|(_code, raw)| raw)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!kept.contains("definitions:"), "{kept}");
        assert!(kept.contains("catalog: Catalog"), "{kept}");
        assert!(
            kept.contains("AssetResolver::new"),
            "the production code after a comma-terminated test field must survive: {kept}"
        );

        let variant = concat!(
            "enum Mode {\n",
            "    Live,\n",
            "    #[cfg(test)]\n",
            "    Fixture,\n",
            "}\n",
            "impl Mode {\n",
            "    pub fn open(&self) {\n",
            "        let _ = GameFileResolver::vanilla(root);\n",
            "    }\n",
            "}\n",
        );
        let kept = production_lines(variant)
            .into_iter()
            .map(|(_code, raw)| raw)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!kept.contains("Fixture,"), "{kept}");
        assert!(
            kept.contains("GameFileResolver::vanilla"),
            "the production method after a comma-terminated test variant must survive: {kept}"
        );

        // The counterweight: a multi-line signature's commas sit inside the
        // parameter list, and must NOT end the item early.
        let signature = concat!(
            "#[cfg(test)]\n",
            "fn only_for_tests(\n",
            "    first: &str,\n",
            "    second: &str,\n",
            ") -> String {\n",
            "    let _ = GameFileResolver::vanilla(root);\n",
            "    String::new()\n",
            "}\n",
            "pub fn production() {\n",
            "    let _ = AssetResolver::new(vec![root]);\n",
            "}\n",
        );
        let kept = production_lines(signature)
            .into_iter()
            .map(|(_code, raw)| raw)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!kept.contains("second: &str"), "{kept}");
        assert!(!kept.contains("GameFileResolver::vanilla"), "{kept}");
        assert!(kept.contains("AssetResolver::new"), "{kept}");
    }

    /// THE ROOT CLOSURE: a game-derived directory can also enter the replay as
    /// a ROOT that no resolver owns - the shader cache did, read straight by
    /// path from a CLI argument, which is why no reader gate above saw it.
    /// `require_dir` is the one primitive that admits a root, so every label it
    /// is called with must map to a ledger class.
    ///
    /// kills: a new operator-supplied game-derived directory reaching
    /// production with no row describing what a mismatch does.
    #[test]
    fn every_directory_root_the_replay_admits_is_named_in_the_ledger() {
        let classes = GAME_FILE_LEDGER
            .iter()
            .map(|entry| entry.class)
            .collect::<BTreeSet<_>>();
        for (_label, class) in DIRECTORY_ROOTS {
            assert!(
                classes.contains(class),
                "root class {class} is not a ledger row"
            );
        }
        let named = DIRECTORY_ROOTS
            .iter()
            .map(|(label, _)| *label)
            .collect::<BTreeSet<_>>();

        let mut swept = BTreeSet::new();
        for (relative, source) in production_sources() {
            for (code, raw) in production_lines(&source) {
                if !code.contains("require_dir(") || code.contains("fn require_dir(") {
                    continue;
                }
                let label = raw
                    .split_once("require_dir(")
                    .and_then(|(_, tail)| tail.split_once(','))
                    .and_then(|(_, tail)| tail.split('"').nth(1))
                    .unwrap_or_else(|| {
                        panic!("{relative} admits a root with no stated label: {}", raw.trim())
                    });
                swept.insert(label.to_string());
            }
        }
        let swept = swept.iter().map(String::as_str).collect::<BTreeSet<_>>();
        assert_eq!(
            swept, named,
            "the directory roots the replay admits and the roots the ledger names must be the \
             same set"
        );
    }

    /// kills: matching a token inside a string literal or a comment as though it
    /// were a call, which would force a false entry into an allowlist.
    #[test]
    fn the_sweep_matches_code_and_not_literals() {
        let source = concat!(
            "// GameFileResolver::vanilla(root)
",
            "const NOTE: &str = \"GameFileResolver::vanilla(root)\";
",
            "fn real() { let _ = GameFileResolver::vanilla(root); }
",
        );
        let rows = production_lines(source);
        let code_hits = rows
            .iter()
            .filter(|(code, _)| code.contains("GameFileResolver::vanilla("))
            .count();
        assert_eq!(code_hits, 1, "{rows:#?}");
        let raw_hits = rows
            .iter()
            .filter(|(_, raw)| raw.contains("GameFileResolver::vanilla("))
            .count();
        assert_eq!(raw_hits, 3, "the raw lines are kept for the literal sweep");
    }

    /// kills: adding a game-relative path constant in a family the ledger does
    /// not describe, which would fall into the catch-all at open instead of
    /// being named.
    #[test]
    fn every_game_relative_path_constant_classifies_to_a_named_row() {
        let mut unclassified = Vec::new();
        let mut seen = 0usize;
        for (relative, text) in production_sources() {
            if !(relative.starts_with("map_viewer/src/")
                || relative.starts_with("game_ui/src/")
                || relative.starts_with("replay_app/src/"))
            {
                continue;
            }
            for (_code, line) in production_lines(&text) {
                let Some(rest) = line.split_once(": &str = \"") else {
                    continue;
                };
                if !rest.0.trim_start().starts_with("const ")
                    && !rest.0.trim_start().starts_with("pub const ")
                    && !rest.0.trim_start().starts_with("pub(crate) const ")
                {
                    continue;
                }
                let Some((value, _)) = rest.1.split_once('"') else {
                    continue;
                };
                let head = value.split('/').next().unwrap_or("").to_ascii_lowercase();
                if !INSTALL_ROOT_DIRECTORIES.contains(&head.as_str()) || !value.contains('/') {
                    continue;
                }
                seen += 1;
                let class = classify_admitted_path(value);
                if class == OTHER_ADMITTED {
                    unclassified.push(format!("{relative}: {value}"));
                }
            }
        }
        assert!(
            seen >= 100,
            "the path-constant sweep found only {seen}; it is broken"
        );
        assert!(
            unclassified.is_empty(),
            "these game-relative path constants fall into the catch-all class, so the open-time \
             statement cannot name them: {unclassified:#?}"
        );
    }

    /// A path handed to a resolver as a bare literal never becomes a `const`, so
    /// the constant sweep cannot see it. `events` reached production this way.
    ///
    /// kills: enumerating a new install directory through the resolver without
    /// giving its family a ledger row.
    #[test]
    fn every_literal_path_handed_to_the_resolver_classifies_to_a_named_row() {
        const LITERAL_ARGUMENT_CALLS: [&str; 11] = [
            ".resolve(",
            "resolve_required(",
            "resolve_required_with_provenance(",
            "resolve_with_provenance(",
            "resolve_optional_file(",
            "resolve_file(",
            "resolve_dir_files(",
            "resolve_dir_files_recursive(",
            "list_files_recursive(",
            "resolve_texture_file(",
            "decode_texture(",
        ];
        let mut unclassified = Vec::new();
        let mut seen = 0usize;
        for (relative, text) in production_sources() {
            for (_code, raw) in production_lines(&text) {
                for call in LITERAL_ARGUMENT_CALLS {
                    let mut rest = raw.as_str();
                    while let Some((_, tail)) = rest.split_once(call) {
                        rest = tail;
                        let argument = tail.trim_start().trim_start_matches('&');
                        let Some(literal) = argument.strip_prefix('"') else {
                            continue;
                        };
                        let Some((value, _)) = literal.split_once('"') else {
                            continue;
                        };
                        let head = value
                            .split(['/', '\\'])
                            .next()
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        if !INSTALL_ROOT_DIRECTORIES.contains(&head.as_str()) {
                            continue;
                        }
                        seen += 1;
                        if classify_admitted_path(value) == OTHER_ADMITTED {
                            unclassified.push(format!("{relative}: {value}"));
                        }
                    }
                }
            }
        }
        assert!(
            seen >= 20,
            "the literal-argument sweep found only {seen}; it is broken"
        );
        assert!(
            unclassified.is_empty(),
            "these literal paths handed to the resolver fall into the catch-all class: \
             {unclassified:#?}"
        );
    }

    /// kills: classifying a suspect family (flags, portraits, fonts,
    /// localisation, shaders, terrain textures) into the catch-all, which would
    /// hide it from the open-time statement.
    #[test]
    fn the_suspect_families_each_classify_to_their_own_row() {
        for (path, class) in [
            ("gfx/flags/medium/GER_democratic.tga", COUNTRY_FLAG),
            ("gfx/leaders/GER/portrait.dds", PORTRAIT),
            ("gfx/fonts/hoi_20bs.fnt", FONT),
            ("gfx/FX/pdxmap.shader", SHADER),
            ("gfx/fx/border.shader", SHADER),
            ("gfx/models/mapitems/trees/forest.mesh", MODEL_MESH),
            ("gfx/models/supply/railroad.shader", SHADER),
            ("gfx/entities/buildings.asset", MODEL_MESH),
            ("gfx/interface/resources_strip.dds", INTERFACE_TEXTURE),
            ("gfx/texticons/nato_unit_a.dds", INTERFACE_TEXTURE),
            ("gfx/posteffect_volumes.txt", WORLD_GFX_TABLE),
            ("gfx/world/colorcorrection_day.tga", WORLD_GFX_TABLE),
            ("map/terrain/atlas_normal0.dds", MAP_TERRAIN_TEXTURE),
            ("map/provinces.bmp", MAP_GEOMETRY),
            ("history/states/1.txt", MAP_GEOMETRY),
            ("interface/core.gfx", INTERFACE_DEFINITION),
            ("localisation/english/focus_l_english.yml", LOCALISATION),
            ("common/defines/00_graphics.lua", SCRIPT_DEFINITION),
            ("gfx/exact_target_initial.bin", OTHER_ADMITTED),
        ] {
            assert_eq!(
                classify_admitted_path(path),
                class,
                "{path} classified wrong"
            );
            assert!(row(class).is_some(), "{class} has no ledger row");
        }
        assert_eq!(
            classify_admitted_path("GFX/Flags/Medium/GER.TGA"),
            COUNTRY_FLAG,
            "Clausewitz paths are case-insensitive"
        );
    }

    /// kills: a packaged artifact that the roster requires but the ledger never
    /// tells the viewer they lost.
    #[test]
    fn every_packaged_static_artifact_names_what_it_loses() {
        let named = PACKAGED_STATIC_ARTIFACT_LOSS
            .iter()
            .map(|(name, _)| *name)
            .collect::<BTreeSet<_>>();
        assert_eq!(named.len(), PACKAGED_STATIC_ARTIFACT_LOSS.len());
        for (_, loss) in PACKAGED_STATIC_ARTIFACT_LOSS {
            assert!(!loss.trim().is_empty());
        }
        // Against the AUTHORITATIVE roster, not against itself: a ninth
        // required artifact must fail here rather than go unnamed at open.
        let roster = replay_core::REQUIRED_PACKAGED_STATIC_ARTIFACTS
            .iter()
            .map(|(name, _)| *name)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            named, roster,
            "the ledger's packaged-artifact list and the recording roster disagree"
        );
        let subject = row(PACKAGED_STATIC_ARTIFACT).expect("packaged row").subject;
        for name in &roster {
            assert!(
                subject.contains(name),
                "the packaged row does not name {name}"
            );
        }
    }

    /// kills: reporting "verified" for a recording whose capture refused a
    /// catalog, so the viewer never learns what is missing.
    /// kills: a recording whose recorder never stated a production profile
    /// reading as CLEAN, because the unstated admission arrives as
    /// `Some(product)` with empty artifact lists and only its unheld
    /// guarantees to show for it.
    #[test]
    fn an_unstated_production_profile_is_named_and_not_reported_clean() {
        let unstated = RecorderProductAdmission {
            authority: replay_core::ProductionProfileAuthority::Unstated,
            guarantees_not_held: vec![
                replay_core::ProductionProfileGuarantee::ProductionProfileEnabled,
                replay_core::ProductionProfileGuarantee::FullProfileDomainSchedule,
            ],
            profile_domains_not_scheduled: Vec::new(),
            missing_required_static_artifacts: Vec::new(),
            static_artifacts_not_content_bound: Vec::new(),
            attach_time_refusals: Vec::new(),
        };
        let text = super::open_statement_for_tests(
            &[],
            Some(&unstated),
            &RenderSettingsPin::Pinned,
            "l_english",
        );
        assert!(
            statement_names_a_degradation(&text),
            "an unstated profile must reach the viewer: {text}"
        );
        assert!(text.contains("production_profile_enabled"), "{text}");
        assert!(text.contains("full_profile_domain_schedule"), "{text}");
        assert!(
            !text.contains(&format!(
                "all {} are carried",
                replay_core::REQUIRED_PACKAGED_STATIC_ARTIFACTS.len()
            )),
            "empty artifact lists must not read as a clean bill: {text}"
        );

        let absent = super::open_statement_for_tests(
            &[],
            None,
            &RenderSettingsPin::Pinned,
            "l_english",
        );
        assert!(
            statement_names_a_degradation(&absent),
            "no admission at all must reach the viewer too: {absent}"
        );
    }

    /// kills: printing a generic cause over the attach-time proof's own, which
    /// is what dropped `captures/rc_vanilla7`'s "digest unrecorded (pre-bump
    /// capture)" on the floor. The role there is MISSING, not carried, so the
    /// cause has to reach the missing-artifact line and not only the
    /// content-bound one (lane UH, Codex round 5).
    #[test]
    fn a_missing_catalog_names_the_attach_time_proofs_own_cause() {
        use portable_content_manifest::attach_content_epoch::{
            RefusedArtifactDigest, DIGEST_UNRECORDED_PRE_BUMP,
        };
        let raw_reason = "33 of 30002 claimed paths did not match";
        let admission = |digest: RefusedArtifactDigest, reason: &str| RecorderProductAdmission {
            authority: replay_core::ProductionProfileAuthority::Disabled,
            guarantees_not_held: vec![
                replay_core::ProductionProfileGuarantee::RequiredStaticArtifacts,
            ],
            profile_domains_not_scheduled: Vec::new(),
            missing_required_static_artifacts: vec!["gfx_icon_asset_manifest_static".into()],
            static_artifacts_not_content_bound: Vec::new(),
            attach_time_refusals: vec![replay_core::StaticArtifactNotContentBound {
                role: "gfx_icon_asset_manifest_static".into(),
                reason: reason.into(),
                digest,
            }],
        };
        let statement = |product: &RecorderProductAdmission| {
            super::open_statement_for_tests(&[], Some(product), &RenderSettingsPin::Pinned, "l_english")
        };

        let product = admission(RefusedArtifactDigest::UnrecordedPreBump, raw_reason);
        let text = statement(&product);
        assert!(
            text.contains(&format!(
                "Cause: {raw_reason}; {DIGEST_UNRECORDED_PRE_BUMP}"
            )),
            "the pre-bump cause must reach the viewer: {text}"
        );

        // The badge comes from the TYPED disposition, not from the reason text.
        // A producer that spells the phrase into its own reason under an explicit
        // null does not get the pre-bump badge appended (Codex round 6).
        let forged = admission(
            RefusedArtifactDigest::RecorderUnreadable,
            &format!("{raw_reason}; {DIGEST_UNRECORDED_PRE_BUMP}"),
        );
        let forged_text = statement(&forged);
        assert_eq!(
            forged_text.matches(DIGEST_UNRECORDED_PRE_BUMP).count(),
            1,
            "the note must be the producer's own text once, not badged again: {forged_text}"
        );
        assert_eq!(
            text.matches(DIGEST_UNRECORDED_PRE_BUMP).count(),
            1,
            "{text}"
        );

        // A role the proof says nothing about keeps the generic cause rather
        // than borrowing another role's.
        let unnamed = RecorderProductAdmission {
            attach_time_refusals: Vec::new(),
            ..product
        };
        let text = super::open_statement_for_tests(
            &[],
            Some(&unnamed),
            &RenderSettingsPin::Pinned,
            "l_english",
        );
        assert!(
            text.contains("did not match the content that capture mounted")
                && !text.contains("pre-bump"),
            "{text}"
        );
    }

    #[test]
    fn a_refused_catalog_is_named_in_the_open_statement() {
        let product = RecorderProductAdmission {
            authority: replay_core::ProductionProfileAuthority::Enabled,
            guarantees_not_held: vec![replay_core::ProductionProfileGuarantee::RequiredStaticArtifacts],
            profile_domains_not_scheduled: Vec::new(),
            missing_required_static_artifacts: vec!["gfx_icon_asset_manifest_static".into()],
            static_artifacts_not_content_bound: vec![
                replay_core::StaticArtifactNotContentBound {
                    role: "localisation_join".into(),
                    reason: "this recording states no content epoch".into(),
                    digest: portable_content_manifest::attach_content_epoch::RefusedArtifactDigest::Recorded,
                },
            ],
            attach_time_refusals: Vec::new(),
        };
        let text = super::open_statement_for_tests(
            &[],
            Some(&product),
            &RenderSettingsPin::Pinned,
            "l_english",
        );
        assert!(text.contains("DEGRADED"), "{text}");
        assert!(text.contains("gfx_icon_asset_manifest_static"), "{text}");
        assert!(text.contains("localisation_join"), "{text}");
        assert!(
            text.contains("static localisation join"),
            "the loss must be named, not just the artifact: {text}"
        );

        let clean = RecorderProductAdmission {
            authority: replay_core::ProductionProfileAuthority::Enabled,
            guarantees_not_held: Vec::new(),
            profile_domains_not_scheduled: Vec::new(),
            missing_required_static_artifacts: Vec::new(),
            static_artifacts_not_content_bound: Vec::new(),
            attach_time_refusals: Vec::new(),
        };
        let text = super::open_statement_for_tests(
            &[],
            Some(&clean),
            &RenderSettingsPin::Pinned,
            "l_english",
        );
        assert!(
            !text.contains("DEGRADED"),
            "a complete recording must claim no degradation: {text}"
        );
        assert!(
            text.contains(&format!(
                "all {} are carried",
                replay_core::REQUIRED_PACKAGED_STATIC_ARTIFACTS.len()
            )),
            "{text}"
        );
    }

    /// kills: packaging a catalog on a blanket "the replay cannot re-derive it"
    /// claim. localisation_join carries authored strings the admitted install
    /// already holds, in English only, and the rationale must say so.
    #[test]
    fn every_packaged_artifact_states_its_own_reason_for_being_packaged() {
        let named = PACKAGED_STATIC_ARTIFACT_RATIONALE
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<BTreeSet<_>>();
        let roster = replay_core::REQUIRED_PACKAGED_STATIC_ARTIFACTS
            .iter()
            .map(|(name, _)| *name)
            .collect::<BTreeSet<_>>();
        assert_eq!(named, roster, "every packaged artifact needs its own reason");
        for (name, verdict, reason) in PACKAGED_STATIC_ARTIFACT_RATIONALE {
            assert!(!reason.trim().is_empty(), "{name} states no reason");
            // A verdict is not prose: one that says the payload belongs
            // install-read must also say what stops it moving, and one that
            // claims a capture-only input must name that input.
            match verdict {
                PackagingVerdict::ShouldBeInstallReadPinned => assert!(
                    reason.contains("Blocked on:"),
                    "{name} should move but names no blocker"
                ),
                PackagingVerdict::CaptureOnlyInput => assert!(
                    reason.contains("engine"),
                    "{name} claims a capture-only input but does not name it"
                ),
            }
        }
        let localisation = PACKAGED_STATIC_ARTIFACT_RATIONALE
            .iter()
            .find(|(name, _, _)| *name == "localisation_join")
            .expect("localisation_join is on the roster");
        assert_eq!(localisation.1, PackagingVerdict::ShouldBeInstallReadPinned);
        assert!(localisation.2.contains("ENGLISH ONLY"), "{}", localisation.2);
    }

    /// kills: showing English research and idea names over a non-English replay
    /// without saying so, because the packaged localisation_join is built in
    /// English only.
    #[test]
    fn a_non_english_recording_is_told_its_research_names_are_english() {
        let clean = RecorderProductAdmission {
            authority: replay_core::ProductionProfileAuthority::Enabled,
            guarantees_not_held: Vec::new(),
            profile_domains_not_scheduled: Vec::new(),
            missing_required_static_artifacts: Vec::new(),
            static_artifacts_not_content_bound: Vec::new(),
            attach_time_refusals: Vec::new(),
        };
        let french = super::open_statement_for_tests(
            &[],
            Some(&clean),
            &RenderSettingsPin::Pinned,
            "l_french",
        );
        assert!(french.contains("DEGRADED"), "{french}");
        assert!(french.contains("l_french"), "{french}");
        for label in ENGLISH_ONLY_PACKAGED_LABELS {
            assert!(
                french.contains(label),
                "the notice must name {label}, not only some of what is substituted: {french}"
            );
        }
        assert!(
            !french.contains("Every other string"),
            "the notice must not claim more than it can: {french}"
        );

        let english = super::open_statement_for_tests(
            &[],
            Some(&clean),
            &RenderSettingsPin::Pinned,
            "l_english",
        );
        assert!(
            !english.contains("national focus labels"),
            "an English recording loses nothing: {english}"
        );

        let unbound = RecorderProductAdmission {
            static_artifacts_not_content_bound: vec![
                replay_core::StaticArtifactNotContentBound {
                    role: "localisation_join".into(),
                    reason: "this recording states no content epoch".into(),
                    digest: portable_content_manifest::attach_content_epoch::RefusedArtifactDigest::Recorded,
                },
            ],
            attach_time_refusals: Vec::new(),
            ..clean
        };
        let text = super::open_statement_for_tests(
            &[],
            Some(&unbound),
            &RenderSettingsPin::Pinned,
            "l_french",
        );
        assert!(
            !text.contains("national focus labels"),
            "an artifact that is never consumed cannot substitute anything: {text}"
        );
    }

    /// kills: naming the map render settings as pinned when the recording
    /// carries none of them, which is the silent local-quality substitution.
    #[test]
    fn an_unpinned_render_profile_is_named_with_the_values_this_machine_supplied() {
        let text = super::open_statement_for_tests(
            &[],
            None,
            &RenderSettingsPin::LocalUnpinned {
                draw_hires_terrain: true,
                high_gfx_shaders: false,
                texture_quality: 2,
            },
            "l_english",
        );
        assert!(text.contains("DEGRADED"), "{text}");
        assert!(text.contains("draw_hires_terrain=true"), "{text}");
        assert!(text.contains("high_gfx_shaders=false"), "{text}");
        assert!(text.contains("texture_quality=2"), "{text}");

        let complete = RecorderProductAdmission {
            authority: replay_core::ProductionProfileAuthority::Enabled,
            guarantees_not_held: Vec::new(),
            profile_domains_not_scheduled: Vec::new(),
            missing_required_static_artifacts: Vec::new(),
            static_artifacts_not_content_bound: Vec::new(),
            attach_time_refusals: Vec::new(),
        };
        let pinned = super::open_statement_for_tests(
            &[],
            Some(&complete),
            &RenderSettingsPin::Pinned,
            "l_english",
        );
        assert!(
            pinned.contains("render_settings [INSTALL-READ-PINNED]"),
            "{pinned}"
        );
        assert!(!pinned.contains("DEGRADED"), "{pinned}");
    }

    /// kills: counting every admitted path into one bucket, so a viewer cannot
    /// see which class of asset the recording actually pinned.
    #[test]
    fn the_open_statement_counts_each_class_it_saw() {
        let text = super::open_statement_for_tests(
            &[
                "map/provinces.bmp",
                "map/definition.csv",
                "gfx/flags/medium/GER.tga",
                "localisation/english/a_l_english.yml",
            ],
            None,
            &RenderSettingsPin::Pinned,
            "l_english",
        );
        assert!(text.contains("4 install file(s) verified"), "{text}");
        assert!(text.contains("map_geometry [INSTALL-READ-PINNED]: 2"), "{text}");
        assert!(text.contains("country_flag [INSTALL-READ-PINNED]: 1"), "{text}");
        assert!(text.contains("localisation [INSTALL-READ-PINNED]: 1"), "{text}");
        assert!(
            text.contains("font [INSTALL-READ-PINNED]: 0 (this recording read no file"),
            "an unread class must still be named: {text}"
        );
        assert!(
            text.contains("carries no recorder product admission"),
            "{text}"
        );
    }
}

/// Path-only rendering of [`open_statement`] for the tests above: the counting
/// and the degradation naming are the same code, without a live admission.
#[cfg(test)]
fn open_statement_for_tests(
    admitted_paths: &[&str],
    product: Option<&RecorderProductAdmission>,
    render_settings: &RenderSettingsPin,
    localisation_language: &str,
) -> String {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for path in admitted_paths {
        *counts.entry(classify_admitted_path(path)).or_insert(0) += 1;
    }
    render_statement(
        &counts,
        admitted_paths.len(),
        product,
        render_settings,
        localisation_language,
    )
}

fn render_statement(
    counts: &BTreeMap<&'static str, usize>,
    total: usize,
    product: Option<&RecorderProductAdmission>,
    render_settings: &RenderSettingsPin,
    localisation_language: &str,
) -> String {
    let mut text = format!(
        "Installed content: {total} install file(s) verified against this recording's pins, by class:"
    );
    for entry in GAME_FILE_LEDGER {
        if let Some(count) = counts.get(entry.class) {
            text.push_str(&format!(
                "\n  {} [{}]: {count}",
                entry.class,
                entry.validation.token()
            ));
        }
    }
    for entry in GAME_FILE_LEDGER {
        if entry.class == PROFILE_IDENTITY
            || entry.class == CONTENT_ROOT
            || entry.class == PASS_THROUGH_READER
            || entry.class == RESOLVED_PATH_DECODER
            || entry.class == REPLAY_CONTAINER
            || entry.class == FROZEN_CENSUS_ROUTE
            || entry.class == OTHER_ADMITTED
            || counts.contains_key(entry.class)
            || entry.validation != Validation::InstallReadPinned
        {
            continue;
        }
        text.push_str(&format!(
            "\n  {} [{}]: 0 (this recording read no file of this class)",
            entry.class,
            entry.validation.token()
        ));
    }
    text.push_str(
        "\n  profile_identity [INSTALL-READ-PINNED]: game version, executable, launcher \
         selection, settings and language all compared",
    );
    match render_settings {
        RenderSettingsPin::Pinned => text.push_str(
            "\n  render_settings [INSTALL-READ-PINNED]: terrain detail, shader tier and texture \
             quality match the recording",
        ),
        RenderSettingsPin::LocalUnpinned {
            draw_hires_terrain,
            high_gfx_shaders,
            texture_quality,
        } => text.push_str(&format!(
            "\nDEGRADED: this recording pins no map render settings, so the image is drawn at \
             THIS machine's quality tier, not the capture's: \
             draw_hires_terrain={draw_hires_terrain}, high_gfx_shaders={high_gfx_shaders}, \
             texture_quality={texture_quality}. Terrain detail, the terrain shader variant and \
             the fog-of-war noise texture follow those values; every other asset in this replay \
             is still the capture's own content."
        )),
    }
    match product {
        None => text.push_str(
            "\nDEGRADED: this recording carries no recorder product admission at all, so it \
             names neither the catalogs it packaged nor the ones it could not, and nothing here \
             can say what the viewer is missing.",
        ),
        Some(product) => {
            if !product.guarantees_not_held.is_empty() {
                text.push_str(&format!(
                    "\nDEGRADED: this recording's production profile is {}, so these guarantees \
                     are NOT held: {}. Whatever the catalog lines below say, they describe a \
                     capture that did not run the full production schedule.",
                    match product.authority {
                        replay_core::ProductionProfileAuthority::Unstated =>
                            "not stated by the recorder",
                        replay_core::ProductionProfileAuthority::Disabled => "disabled",
                        replay_core::ProductionProfileAuthority::Enabled => "enabled",
                    },
                    product
                        .guarantees_not_held
                        .iter()
                        .map(|guarantee| guarantee.token())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            for domain in &product.profile_domains_not_scheduled {
                text.push_str(&format!(
                    "\nDEGRADED: the capture did not schedule domain {domain}, so this replay \
                     shows no state that domain owns."
                ));
            }
            if product.guarantees_not_held.is_empty()
                && product.missing_required_static_artifacts.is_empty()
                && product.static_artifacts_not_content_bound.is_empty()
            {
                text.push_str(&format!(
                    "\nPACKAGED CATALOGS: all {} are carried and bound to this recording's \
                     content epoch.",
                    replay_core::REQUIRED_PACKAGED_STATIC_ARTIFACTS.len()
                ));
            }
            for name in &product.missing_required_static_artifacts {
                // The attach-time proof's own reason when it has one for this
                // role, which is the only place a pre-bump recording's
                // unrecorded digest can be read: a role the package does not
                // carry never reaches the content-bound check below.
                let cause = product
                    .attach_time_refusals
                    .iter()
                    .find(|row| row.role == *name)
                    .map(replay_core::StaticArtifactNotContentBound::rendered)
                    .unwrap_or_else(|| {
                        "the artifact on the capture machine did not match the content that \
                         capture mounted"
                            .to_string()
                    });
                text.push_str(&format!(
                    "\nDEGRADED: the capture could not package {name}, so this replay cannot show \
                     {}. Cause: {cause}.",
                    artifact_loss(name)
                ));
            }
            for row in &product.static_artifacts_not_content_bound {
                // The proof's own refusal reason, verbatim. A row whose
                // `artifact_sha256` key was never written says so here, instead
                // of reading like an ordinary unauthorized digest.
                text.push_str(&format!(
                    "\nDEGRADED: {} is carried but this recording's content epoch does not \
                     authorize its digest, so it is not consumed and this replay cannot show {}. \
                     Cause: {}.",
                    row.role,
                    artifact_loss(&row.role),
                    row.rendered()
                ));
            }
            let english_join_consumed = !product
                .missing_required_static_artifacts
                .iter()
                .map(String::as_str)
                .chain(
                    product
                        .static_artifacts_not_content_bound
                        .iter()
                        .map(|row| row.role.as_str()),
                )
                .any(|name| name == "localisation_join");
            if english_join_consumed && !is_english(localisation_language) {
                text.push_str(&format!(
                    "\nDEGRADED: this recording was made in {localisation_language}, but every \
                     packaged catalog is joined against an English-only localisation build, so \
                     the display names they supply are English: {}. Strings the replay reads from \
                     the admitted install - the interface itself and its localisation - are in \
                     {localisation_language}.",
                    ENGLISH_ONLY_PACKAGED_LABELS.join(", ")
                ));
            }
        }
    }
    text
}

/// The display names the packaged catalogs supply, and therefore the strings an
/// English-only localisation join substitutes. One entry per label lookup in
/// `game_ui::static_research`, the only consumer that reads display text out of
/// a packaged catalog.
pub const ENGLISH_ONLY_PACKAGED_LABELS: &[&str] = &[
    "national focus labels",
    "technology names",
    "idea names",
];

fn is_english(language: &str) -> bool {
    let trimmed = language.trim();
    trimmed.eq_ignore_ascii_case("l_english") || trimmed.eq_ignore_ascii_case("english")
}
