use std::sync::OnceLock;

use engine_executable::executable_tables::{self, Tables};
use synthetic_game::Executable;

const INTERFACE_WORDS: &[&str] = &[
    "accelerated", "add", "add_horizontal", "allwaystransparent", "alwaystransparent", "amount",
    "anchor", "animatedmaptext", "animation", "animation_rate_fps", "animation_rate_spf",
    "animation_time", "animation_type", "animationblendmode", "animationdelay",
    "animationframes", "animationlooping", "animationmaskfile", "animationrotation",
    "animationrotationoffset", "animationtexturefile", "animationtexturescale", "animationtime",
    "animationtriggerframes", "animationtype", "arrowtype", "auto", "autohide_scrollbars",
    "background", "background_margin", "base", "below", "bitmapfont", "bitmapfont_override",
    "bitmapfonts", "border_color", "bordersize", "bottom", "bound_text", "bound_tooltip",
    "browsertype", "buttonfont", "buttonmesh", "buttontext", "buttontype", "callback_argument",
    "callback_type", "can_be_lowres", "center", "center_down", "center_left",
    "center_on_anchor", "center_right", "center_up", "centerposition", "centre", "char",
    "chars", "checkboxtype", "circularprogressbartype", "click_to_front", "clickonly",
    "clicksound", "clipping", "closed_position", "color", "color_override", "colorcodes",
    "colors", "colortwo", "containerwindowtype", "context_aware_text", "context_aware_tooltip",
    "contractonleave", "corneredtilespritetype", "cull_distance", "cursor", "cursor_offset",
    "curvegraphtype", "decelerated", "decreasebutton", "default_clicksound",
    "default_downsound", "default_oversound", "default_upsound", "defaultanimationtime",
    "delayedtooltiptext", "dissablesprite", "dontrender", "down", "downsound", "downsprite",
    "drag", "drag_scroll", "dropdown", "dropdownboxtype", "dropdownmenutype", "dx_offset",
    "editboxtype", "effect", "effectfile", "enabled", "endat", "expandbutton", "expandedontop",
    "expandedwindow", "extendedscrollbartype", "fade_delay", "fade_time", "fade_type", "file",
    "first_on_top", "fixedsize", "font", "fontfiles", "fontname", "format", "frame",
    "frameanimatedspritetype", "framedirectiony", "fullscreen", "generate_mip_maps",
    "gridboxtype", "guibuttontype", "guitypes", "heading", "height", "hide",
    "hide_animation_time", "hide_animation_type", "hide_position", "hide_sound",
    "hideheaderonexpand", "hint_tag", "horizontal", "horizontal_scroll_step",
    "horizontalborder", "horizontalscrollbar", "hsv", "icon_scale", "icons_add_height",
    "icontype", "id", "ignore_enter", "ignore_special_chars", "ignore_tab_navigation",
    "increasebutton", "index", "info", "instanttextboxtype", "is_multiline", "kerning",
    "languages", "layout", "left", "leftbutton", "legacy_lazy_load", "linear", "linecharttype",
    "lineheight", "linewidth", "listboxtype", "loadtype", "localization_key", "lockable",
    "looping", "lower_left", "lower_right", "luaclick", "margin", "maskedshieldtype",
    "masking_texture", "max", "max_resolution", "max_slots", "max_slots_horizontal",
    "max_slots_vertical", "maxheight", "maxvalue", "maxwidth", "membername", "meshsettings",
    "middle", "min", "minvalue", "moveable", "multiline", "multiply", "multispritebuttontype",
    "name", "no_clicksound", "none", "noofframes", "norefcount", "normal", "null_font",
    "objecttypes", "offset", "only_numbers", "open_browser", "open_position", "orientation",
    "origo", "overlappingelementsboxtype", "overlay", "oversound", "oversprite", "padding",
    "page", "parent", "path", "pause_on_loop", "pdx_disabled_tooltip",
    "pdx_disabled_tooltip_delayed", "pdx_tooltip", "pdx_tooltip_anchor", "pdx_tooltip_delayed",
    "pdxmesh", "pdxparticle", "peek_check_size", "peek_check_top_left", "peek_position",
    "piecharttype", "play_on_show", "position", "positiontype", "preload_textures",
    "preserve_aspect_ratio", "priority", "progressbartype", "pulsing", "quadtexturesprite",
    "radiobuttongrouptype", "rangelimitmax", "rangelimitmaxicon", "rangelimitmin",
    "rangelimitminicon", "relative", "resizeablespritetype", "rgb", "right", "rightbutton",
    "rotating", "rotating_ccw", "rotating_cw", "rotation", "scale", "scroll_speed",
    "scroll_wheel_factor", "scrollbar_side", "scrollbargrouptype", "scrollbartype",
    "scrolling", "selection_offset", "settrackframeonchange", "shader", "shader_file",
    "shortcut", "show_animation_time", "show_animation_type", "show_position", "show_sound",
    "size", "slider", "slider_pos_shift", "slotsize", "small", "smooth_scrolling",
    "smoothlistboxtype", "smoothstep", "spacing", "specular", "speed", "spritetype",
    "spritetypes", "startvalue", "step", "steps", "stepsize", "switch_frame_on_expand", "text",
    "text_color_code", "textblock", "textboxtype", "textcolors", "textspritetype", "texture",
    "texturefile", "texturefile1", "texturefile2", "tilesize", "tilespritetype",
    "tilingcenter", "tooltip", "tooltiptext", "top", "track", "transparencecheck", "truncate",
    "type", "up", "upper_left", "upper_right", "upsound", "upsprite", "use_special_chars",
    "userangelimit", "variant", "vertical_alignment", "vertical_scroll_step",
    "verticalborder", "verticalscrollbar", "w", "web_link", "weight", "width", "windowtype",
    "x", "y", "z",
];

const SHADER_WORDS: &[&str] = &[
    "addressu", "addressv", "alphatest", "alphatocoverage", "blendenable", "blendop",
    "blendopalpha", "blendstate", "code", "constantbuffer", "cullmode", "define", "defines",
    "depthbias", "depthbiasclamp", "depthenable", "depthfunction", "depthstencilstate",
    "depthwritemask", "destalpha", "destblend", "elif", "else", "endif", "fillmode", "frontccw",
    "frontstencildepthfailop", "frontstencilfailop", "frontstencilfunc", "frontstencilpassop",
    "if", "ifdef", "ifndef", "includes", "magfilter", "maincode", "maxanisotropy", "minfilter",
    "mipfilter", "mipmaplodbias", "mipmapmaxlod", "mipmapminlod", "multisample", "pixelshader",
    "rasterizerstate", "samplers", "scissor", "slopescaledepthbias", "sourcealpha", "sourceblend",
    "stencilenable", "stencilreadmask", "stencilref", "stencilwritemask", "vertexshader",
    "vertexstruct", "writemask",
];

const EQUIPMENT_CATEGORIES: &[&str] = &[
    "infantry", "support", "armor", "convoy", "artillery", "anti_tank", "anti_air", "motorized",
    "mechanized", "fighter", "cas", "naval_bomber", "tactical_bomber", "strategic_bomber",
    "air_transport", "capital_ship", "screen_ship", "submarine", "carrier", "support_ship",
    "train", "scout_plane", "maritime_patrol_plane", "rocket", "flame", "land_cruiser",
];

const PINNED_CATEGORIES: &[(&str, u64)] = &[
    ("convoy", 0x1),
    ("armor", 0x4),
    ("capital_ship", 0x40),
    ("submarine", 0x80),
    ("screen_ship", 0x100),
    ("carrier", 0x40_0000),
    ("fighter", 0x400),
    ("tactical_bomber", 0x800),
    ("strategic_bomber", 0x2000),
    ("cas", 0x4000),
    ("naval_bomber", 0x8000),
    ("air_transport", 0x10_0000),
    ("scout_plane", 0x1_0000_0000),
    ("maritime_patrol_plane", 0x2_0000_0000),
];
const RESERVED_CATEGORY_BITS: u64 = 0x80_0040_03C1 | 0x1f_0037_fc00;
const LAND: &[&str] = &["armor", "motorized", "mechanized", "infantry", "train", "land_cruiser"];
const AIR: &[&str] = &[
    "fighter", "tactical_bomber", "strategic_bomber", "cas", "naval_bomber", "scout_plane",
    "air_transport", "maritime_patrol_plane",
];
const NAVAL: &[&str] = &["convoy", "capital_ship", "submarine", "screen_ship", "carrier", "support_ship"];
const SHIP_DESIGNER: &[&str] = &["capital_ship", "submarine", "screen_ship", "carrier", "support_ship"];
const TANK_DESIGNER: &[&str] = &["armor"];
const PLANE_DESIGNER: &[&str] = &[
    "fighter", "tactical_bomber", "strategic_bomber", "cas", "naval_bomber", "scout_plane",
    "maritime_patrol_plane",
];

const STATS: &[(&str, &str, bool)] = &[
    ("max_strength", "STAT_SYNTHETIC_MAX_STRENGTH", true),
    ("max_organisation", "STAT_SYNTHETIC_MAX_ORGANISATION", true),
    ("defense", "STAT_SYNTHETIC_DEFENSE", true),
    ("breakthrough", "STAT_SYNTHETIC_BREAKTHROUGH", true),
    ("soft_attack", "STAT_SYNTHETIC_SOFT_ATTACK", true),
    ("hard_attack", "STAT_SYNTHETIC_HARD_ATTACK", true),
    ("lg_attack", "STAT_SYNTHETIC_LG_ATTACK", true),
    ("air_attack", "STAT_SYNTHETIC_AIR_ATTACK", true),
    ("hardness", "STAT_SYNTHETIC_HARDNESS", true),
    ("armor_value", "STAT_SYNTHETIC_ARMOR", true),
    ("ap_attack", "STAT_SYNTHETIC_PIERCING", true),
    ("maximum_speed", "STAT_SYNTHETIC_SPEED", false),
    ("hg_attack", "STAT_SYNTHETIC_HG_ATTACK", true),
    ("torpedo_attack", "STAT_SYNTHETIC_TORPEDO_ATTACK", true),
    ("sub_attack", "STAT_SYNTHETIC_SUB_ATTACK", true),
    ("air_agility", "STAT_SYNTHETIC_AIR_AGILITY", true),
    ("air_defence", "STAT_SYNTHETIC_AIR_DEFENCE", true),
    ("supply_consumption", "STAT_SYNTHETIC_SUPPLY_CONSUMPTION", false),
];
const PINNED_STATS: &[(usize, (&str, &str, bool))] = &[
    (
        crate::equipment_stats::RELIABILITY_STAT,
        ("reliability", "STAT_SYNTHETIC_RELIABILITY", false),
    ),
    (
        crate::equipment_stats::RELIABILITY_FACTOR_STAT,
        (
            "reliability_factor",
            "STAT_SYNTHETIC_RELIABILITY_FACTOR",
            false,
        ),
    ),
    (
        crate::equipment_stats::CARRIER_SIZE_STAT,
        ("carrier_size", "STAT_SYNTHETIC_CARRIER_SIZE", true),
    ),
    (
        crate::equipment_stats::BUILD_COST_IC_STAT,
        ("build_cost_ic", "STAT_SYNTHETIC_COST", false),
    ),
];

const AI_STRATEGIES: &[(&str, u32)] = &[
    ("conquer", 0x1),
    ("scorched_earth_prio", 0x42),
    ("put_unit_buffers", 0x46),
    ("front_unit_request", 0x48),
    ("operative_mission", 0x4B),
    ("front_control", 0x4F),
    ("stockpile_fuel", 0x70),
];

const RULES: &[(&str, &str, bool, bool)] = &[
    ("can_create_factions", "synthetic_diplomacy", true, true),
    ("can_join_factions", "synthetic_diplomacy", true, true),
    ("can_puppet", "synthetic_diplomacy", true, true),
    ("can_send_volunteers", "synthetic_diplomacy", false, false),
    ("can_decline_call_to_war", "synthetic_diplomacy", true, true),
];

const MODIFIERS: &[&str] = &[
    "army_attack_factor",
    "army_defence_factor",
    "political_power_gain",
    "stability_factor",
    "war_support_factor",
    "soft_attack",
    "research_speed_factor",
];

pub fn executable() -> Executable {
    let mut game = crate::trigger_keys::synthetic_vocabulary(
        crate::effect_keys::synthetic_vocabulary(Executable::empty()),
    )
    .words(INTERFACE_WORDS.iter().copied())
        .words(SHADER_WORDS.iter().copied());
    game = game
        .words(EQUIPMENT_CATEGORIES.iter().copied())
        .words(AI_STRATEGIES.iter().map(|&(name, _)| name))
        .words(["build_intel_network", "balanced", "front"]);
    let pinned = PINNED_CATEGORIES.iter().fold(0, |bits, &(_, mask)| bits | mask);
    let mut free = (0..64)
        .map(|bit| 1_u64 << bit)
        .filter(|bit| bit & (RESERVED_CATEGORY_BITS | pinned) == 0);
    let categories: Vec<_> = EQUIPMENT_CATEGORIES
        .iter()
        .map(|&name| {
            let mask = PINNED_CATEGORIES
                .iter()
                .find(|&&(kept, _)| kept == name)
                .map_or_else(|| free.next().expect("a free category bit"), |&(_, mask)| mask);
            (game.token(name).expect("a category is a keyword"), mask, name)
        })
        .collect();
    let stats: Vec<_> = (0..crate::equipment_stats::STAT_COUNT)
        .map(|index| {
            if let Some(&(_, entry)) = PINNED_STATS.iter().find(|&&(at, _)| at == index) {
                entry
            } else if let Some(&entry) = STATS.get(index) {
                entry
            } else {
                let name: &'static str =
                    Box::leak(format!("synthetic_stat_{index}").into_boxed_str());
                (name, "STAT_SYNTHETIC", false)
            }
        })
        .collect();
    game.modifiers(MODIFIERS.iter().copied())
        .auxiliary(|tables| {
            tables.equipment_categories = Ok(categories
                .iter()
                .map(|&(token, mask, name)| (token, mask, name))
                .collect());
            tables.category_names = Ok(categories
                .iter()
                .map(|&(_, mask, name)| (name, mask))
                .collect());
            let bits = |members: &[&str]| {
                categories
                    .iter()
                    .filter(|(_, _, name)| members.contains(name))
                    .fold(0, |bits, &(_, mask, _)| bits | mask)
            };
            tables.category_families = Ok(engine_executable::executable_tables::auxiliary::CategoryFamilies {
                land: bits(LAND),
                air: bits(AIR),
                naval: bits(NAVAL),
                ship_designer: bits(SHIP_DESIGNER),
                tank_designer: bits(TANK_DESIGNER),
                plane_designer: bits(PLANE_DESIGNER),
            });
            tables.stat_metadata = Ok(stats);
            tables.ai_strategy_types = Ok(AI_STRATEGIES.to_vec());
            tables.missions = Ok(vec![("build_intel_network", 1), ("synthetic_mission", 2)]);
            tables.execution_types = Ok(vec![("balanced", 0), ("synthetic_rush", 1)]);
            tables.order_types = Ok(vec![("front", 0), ("synthetic_invasion", 1)]);
            tables.rules = Ok(RULES
                .iter()
                .copied()
                .chain((RULES.len()..16).map(|index| {
                    let name: &'static str =
                        Box::leak(format!("synthetic_rule_{index}").into_boxed_str());
                    (name, "synthetic_rules", false, false)
                }))
                .collect());
        })
}

pub fn tables() -> Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    *TABLES.get_or_init(|| executable().build())
}

pub fn enter() -> Tables {
    let tables = tables();
    executable_tables::enter(Some(tables));
    tables
}

pub fn enter_viewer() -> Tables {
    use crate::scope::ScopeKind::{Character, Combat, Country};
    use crate::trigger_keys::Reader;

    static TABLES: OnceLock<Tables> = OnceLock::new();
    let tables = *TABLES.get_or_init(|| {
        executable()
            .trigger(
                "tag",
                Combat.bit() | Country.bit() | Character.bit(),
                Reader::Assign,
                0,
            )
            .trigger("armor", Combat.bit(), Reader::Assign, 0)
            .trigger(
                "custom_override_tooltip",
                0,
                Reader::Own(0x141446060),
                0x141446C20,
            )
            .trigger("synthetic_unported_trigger", 0, Reader::Assign, 0)
            .build()
    });
    executable_tables::enter(Some(tables));
    tables
}

pub fn with(change: impl FnOnce(Executable) -> Executable) -> Tables {
    let tables = change(executable()).build();
    executable_tables::enter(Some(tables));
    tables
}

pub fn category(name: &str) -> u64 {
    if !executable_tables::is_initialized() {
        enter();
    }
    crate::equipment_stats::category_mask(name).expect("a synthetic equipment category")
}

pub fn read_effect_body(text: &[u8], names: &crate::key_names::KeyNames) -> crate::native::Read {
    use crate::native::{self, Shape};
    enter();
    native::read(
        text,
        native::Mode::KeepMark,
        native::Top::File(()),
        Some(names),
        &mut |(), field| {
            if field.keyword_is("body") {
                Shape::Effect
            } else {
                Shape::One
            }
        },
    )
}

pub mod effect_assertions {
use crate::native::Read;
use content_reader_parser::parser::Item;
pub fn scalar<'a>(items: &'a [Item], key: &str) -> &'a [u8] {
    items
        .iter()
        .find(|item| item.keyword_is(key))
        .unwrap_or_else(|| panic!("missing {key}: {items:?}"))
        .value
        .scalar()
        .unwrap()
}
pub fn body<'a>(items: &'a [Item], key: &str) -> &'a [Item] {
    items
        .iter()
        .find(|item| item.keyword_is(key))
        .unwrap()
        .value
        .items()
        .unwrap()
}
pub fn assert_complete_template_read(result: &Read) {
    assert!(result.unsettled.is_none(), "{result:?}");
    assert!(
        !result
            .misreads
            .iter()
            .any(|note| note.what.starts_with("Deferred effect")),
        "{result:?}"
    );
}
}
