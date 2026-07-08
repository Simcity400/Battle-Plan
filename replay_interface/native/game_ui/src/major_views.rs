use crate::assets::AssetResolver;
use crate::compose::{draw_scaled, ComposeOptions, ComposeOutput, Compositor};
use crate::dds::RgbaImage;
use crate::font::blank_image;
use crate::gfx::SpriteRegistry;
use crate::gui::{parse_measure, parse_measure_pair, GuiElement, LayoutSet, MeasurePair};
use crate::loc::Localisation;
use crate::paradox_text::{parse_file, Value};
use crate::png::write_png;
use crate::{GameUiError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const MAJOR_VIEW_WIDTH: usize = 1920;
pub const MAJOR_VIEW_HEIGHT: usize = 1080;
pub const EXPECTED_MAJOR_VIEW_COUNT: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MajorViewReport {
    pub key: String,
    pub display_name: String,
    pub source_files: Vec<String>,
    pub root_windows: Vec<String>,
    pub sample_drawn_elements: usize,
    pub sample_missing_sprites: usize,
    pub sample_missing_sprite_names: Vec<String>,
    pub sample_undecodable_sprites: usize,
    pub sample_undecodable_sprite_errors: Vec<String>,
    pub sample_missing_fonts: usize,
    pub sample_missing_font_errors: Vec<String>,
    pub sample_missing_glyphs: BTreeMap<String, String>,
    pub sample_unresolved_loc_keys: Vec<String>,
    pub documented_skips: Vec<String>,
    pub proof_png: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MajorViewComposeResult {
    pub report: MajorViewReport,
    pub output: ComposeOutput,
}

#[derive(Clone, Copy, Debug)]
struct RootSpec {
    file: &'static str,
    name: &'static str,
    position_override: Option<(i32, i32)>,
    use_show_position: bool,
}

#[derive(Clone, Copy, Debug)]
struct TemplateSpec {
    file: &'static str,
    name: &'static str,
    prefix: &'static str,
    x: i32,
    y: i32,
    text: &'static [(&'static str, &'static str)],
    sprite_overrides: &'static [(&'static str, &'static str)],
}

#[derive(Clone, Copy, Debug)]
struct ViewSpec {
    key: &'static str,
    display_name: &'static str,
    roots: &'static [RootSpec],
    templates: &'static [TemplateSpec],
    text: &'static [(&'static str, &'static str)],
    progress: &'static [(&'static str, f32)],
    hidden: &'static [&'static str],
    skip_notes: &'static [&'static str],
}

const PRODUCTION_ROOTS: &[RootSpec] = &[RootSpec {
    file: "interface/countryproductionlineview.gui",
    name: "countryproductionlineview",
    position_override: None,
    use_show_position: true,
}];

const PRODUCTION_TEMPLATES: &[TemplateSpec] = &[
    TemplateSpec {
        file: "interface/countryproductionlineview.gui",
        name: "production_military_line_entry",
        prefix: "prod_line_0_",
        x: 8,
        y: 324,
        text: &[
            ("name", "Infantry Equipment I"),
            ("general_name", "FAKE LINE"),
            ("speed", "+4.20/d"),
            ("total_need", "123"),
            ("factories_used_txt", "15/15"),
        ],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countryproductionlineview.gui",
        name: "production_military_line_entry_collapsed",
        prefix: "prod_line_1_",
        x: 8,
        y: 468,
        text: &[
            ("name", "Fighter I"),
            ("general_name", "TAG SAMPLE"),
            ("speed", "+2.10/d"),
            ("total_need", "42"),
        ],
        sprite_overrides: &[],
    },
];

const PRODUCTION_TEXT: &[(&str, &str)] = &[
    ("operational_factories_value", "123"),
    ("operational_naval_factories_value", "45"),
    ("operational_naval_factories_repair_value", "6"),
    ("production_output_value", "88%"),
    ("docyards_output_value", "77%"),
    ("efficiency_cap_value", "99%"),
    ("production_efficiency_loss_value", "-1%"),
    ("factory_bombing_defence_value", "12%"),
    ("efficiency_growth_value", "+3%"),
    ("factories_used_equipment", "15"),
    ("factories_used_armor", "6"),
    ("factories_used_air", "9"),
    ("factories_used_capital_ships", "3"),
    ("factories_used_screen_ships", "4"),
    ("factories_used_resources", "2"),
    ("factories_used_outdated", "1"),
    ("factories_used_stockpiling", "8"),
    ("factories_used_reinforcing", "5"),
    ("factories_used_upgrading", "7"),
    ("factories_used_refitting", "0"),
];

const PRODUCTION_PROGRESS: &[(&str, f32)] = &[
    ("prod_line_0_efficiency_progressbar_to_max", 0.82),
    ("prod_line_0_efficiency_progressbar", 0.64),
    ("prod_line_0_progressbar", 0.37),
    ("prod_line_1_efficiency_progressbar_to_max", 0.55),
    ("prod_line_1_efficiency_progressbar", 0.42),
    ("prod_line_1_progressbar", 0.72),
];

const CONSTRUCTION_ROOTS: &[RootSpec] = &[RootSpec {
    file: "interface/countryconstructionsview.gui",
    name: "countryconstructionsview",
    position_override: None,
    use_show_position: true,
}];

const CONSTRUCTION_TEMPLATES: &[TemplateSpec] = &[
    TemplateSpec {
        file: "interface/countryconstructionsview.gui",
        name: "production_building_line_entry",
        prefix: "build_line_0_",
        x: 12,
        y: 426,
        text: &[
            ("location", "FAKE STATE"),
            ("queued", "3"),
            ("repair", "0"),
            ("factories", "15/15"),
        ],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countryconstructionsview.gui",
        name: "static_consumer_goods_entry",
        prefix: "build_static_0_",
        x: 12,
        y: 552,
        text: &[("name", "Consumer goods"), ("factories", "12")],
        sprite_overrides: &[],
    },
];

const CONSTRUCTION_TEXT: &[(&str, &str)] = &[
    ("factories_used_txt", "42/123"),
    ("available_factories_value", "81"),
    ("traded_factories_value", "12"),
    ("local_factories_value", "69"),
    ("cic_bank_value", "123"),
    ("construction_speed_value", "+25%"),
    ("factories_amount", "15"),
];

const CONSTRUCTION_PROGRESS: &[(&str, f32)] = &[("build_line_0_progress", 0.44)];

const RESEARCH_ROOTS: &[RootSpec] = &[
    RootSpec {
        file: "interface/countrytechtreeview.gui",
        name: "countrytechtreeview",
        position_override: None,
        use_show_position: false,
    },
    RootSpec {
        file: "interface/countrytechnologyview.gui",
        name: "countrytechnologyview",
        position_override: None,
        use_show_position: true,
    },
];

const RESEARCH_TEMPLATES: &[TemplateSpec] = &[
    TemplateSpec {
        file: "interface/countrytechnologyview.gui",
        name: "research_slot_entry",
        prefix: "research_slot_0_",
        x: 12,
        y: 306,
        text: &[
            ("title", "Research Slot 1"),
            ("year", "1936"),
            ("eta", "123d"),
        ],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countrytechnologyview.gui",
        name: "research_slot_entry",
        prefix: "research_slot_1_",
        x: 12,
        y: 414,
        text: &[
            ("title", "Research Slot 2"),
            ("year", "1937"),
            ("eta", "42d"),
        ],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countrytechtreeview.gui",
        name: "techtree_infantry_folder_item",
        prefix: "tech_item_0_",
        x: 126,
        y: 332,
        text: &[("Name", "Basic Infantry"), ("bonus", "1")],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countrytechtreeview.gui",
        name: "techtree_infantry_folder_small_item",
        prefix: "tech_item_1_",
        x: 306,
        y: 348,
        text: &[("bonus", "2")],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countrytechtreeview.gui",
        name: "techtree_infantry_folder_item",
        prefix: "tech_item_2_",
        x: 446,
        y: 478,
        text: &[("Name", "Motorized Sample"), ("bonus", "3")],
        sprite_overrides: &[],
    },
];

const RESEARCH_TEXT: &[(&str, &str)] = &[
    ("limited_research_bonus_value", "3"),
    ("research_speed_value", "+12%"),
];

const RESEARCH_PROGRESS: &[(&str, f32)] = &[
    ("research_slot_0_research_progressbar", 0.62),
    ("research_slot_1_research_progressbar", 0.31),
];

const RESEARCH_HIDDEN: &[&str] = &[
    "facilities",
    "support_folder",
    "armour_folder",
    "nsb_armour_folder",
    "artillery_folder",
    "naval_folder",
    "mtgnavalfolder",
    "mtgnavalsupportfolder",
    "air_techs_folder",
    "bba_air_techs_folder",
    "industry_folder",
    "electronics_folder",
];

const TRADE_ROOTS: &[RootSpec] = &[RootSpec {
    file: "interface/countrytradeview.gui",
    name: "countrytradeview",
    position_override: None,
    use_show_position: true,
}];

const TRADE_TEMPLATES: &[TemplateSpec] = &[
    TemplateSpec {
        file: "interface/countrytradeview.gui",
        name: "resources_filter_entry",
        prefix: "trade_resource_0_",
        x: 18,
        y: 258,
        text: &[],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countrytradeview.gui",
        name: "country_trade_entry",
        prefix: "trade_country_0_",
        x: 18,
        y: 392,
        text: &[
            ("country", "TAG"),
            ("exported", "123"),
            ("available_export", "45"),
            ("convoys", "6"),
        ],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countrytradeview.gui",
        name: "country_trade_entry",
        prefix: "trade_country_1_",
        x: 18,
        y: 436,
        text: &[
            ("country", "FAK"),
            ("exported", "80"),
            ("available_export", "20"),
            ("convoys", "3"),
        ],
        sprite_overrides: &[],
    },
];

const TRADE_TEXT: &[(&str, &str)] = &[
    ("civ_factories", "81"),
    ("factories_total", "123"),
    ("factories_used", "42"),
    ("factories_available", "81"),
    ("produced_title", "PROD"),
    ("imported_title", "IMP"),
    ("exported_title", "EXP"),
    ("needed_title", "NEED"),
];

const DIPLOMACY_ROOTS: &[RootSpec] = &[RootSpec {
    file: "interface/countrydiplomacyview.gui",
    name: "countrydiplomacyview",
    position_override: None,
    use_show_position: true,
}];

const DIPLOMACY_TEMPLATES: &[TemplateSpec] = &[
    TemplateSpec {
        file: "interface/countrydiplomacyview.gui",
        name: "diplomacy_action_entry",
        prefix: "diplo_action_0_",
        x: 24,
        y: 476,
        text: &[("name", "Improve relations"), ("cost", "10")],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countrydiplomacyview.gui",
        name: "diplomacy_action_entry",
        prefix: "diplo_action_1_",
        x: 24,
        y: 512,
        text: &[("name", "Send attache"), ("cost", "50")],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countrydiplomacyview.gui",
        name: "relation_strip_view",
        prefix: "diplo_relation_0_",
        x: 230,
        y: 420,
        text: &[],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countrydiplomacyview.gui",
        name: "diplomacy_trade_entry",
        prefix: "diplo_trade_0_",
        x: 252,
        y: 600,
        text: &[],
        sprite_overrides: &[],
    },
];

const DIPLOMACY_TEXT: &[(&str, &str)] = &[
    ("country_name", "Fake Major Country"),
    ("faction_name", "Faction TAG"),
    ("leader_name", "Leader Name"),
    ("our_opinion_value", "+123"),
    ("their_opinion_value", "-12"),
    ("stability_value", "74%"),
    ("war_support_value", "88%"),
    ("party_name", "Fake Party"),
    ("ideology_name", "Democratic"),
    ("elections", "1940"),
    ("national_focus_label", "Fake Focus"),
    ("estimated_military_forces", "123"),
    ("estimated_navy_forces", "45"),
    ("estimated_airforce", "67"),
    ("estimated_civil_industry_size", "81"),
    ("estimated_military_industry_size", "42"),
    ("estimated_naval_industry_size", "8"),
    ("estimated_convoy_size", "123"),
    ("estimated_manpower_size", "1.23M"),
    ("player_name", "Player TAG"),
];

const DIPLOMACY_HIDDEN: &[&str] = &[
    "chart",
    "intel_ledger_container",
    "standard_diplomacy_controller",
    "invite_faction_diplomacy_controller",
    "diplomacy_action_info_window",
];

const FOCUS_ROOTS: &[RootSpec] = &[RootSpec {
    file: "interface/nationalfocusview.gui",
    name: "nationalfocusview",
    position_override: None,
    use_show_position: false,
}];

const FOCUS_TEMPLATES: &[TemplateSpec] = &[
    TemplateSpec {
        file: "interface/nationalfocusview.gui",
        name: "national_focus_item",
        prefix: "focus_0_",
        x: 620,
        y: 230,
        text: &[("name", "Industry Plan")],
        sprite_overrides: &[("symbol", "GFX_focus_generic_industry_1")],
    },
    TemplateSpec {
        file: "interface/nationalfocusview.gui",
        name: "national_focus_item",
        prefix: "focus_1_",
        x: 790,
        y: 230,
        text: &[("name", "Research Slot")],
        sprite_overrides: &[("symbol", "GFX_focus_research")],
    },
    TemplateSpec {
        file: "interface/nationalfocusview.gui",
        name: "national_focus_item",
        prefix: "focus_2_",
        x: 705,
        y: 370,
        text: &[("name", "Army Sample")],
        sprite_overrides: &[("symbol", "GFX_focus_generic_army_tanks2")],
    },
    TemplateSpec {
        file: "interface/nationalfocusview.gui",
        name: "national_focus_link",
        prefix: "focus_link_0_",
        x: 760,
        y: 330,
        text: &[],
        sprite_overrides: &[],
    },
];

const FOCUS_TEXT: &[(&str, &str)] = &[
    ("national_focus_title", "National Focus"),
    ("to_find_text", "focus"),
    ("found_text", "3"),
    ("type_to_search_text", "Fake search"),
    ("continuous_focus_title", "Continuous Focus"),
    ("continuous_focus_desc", "Static sample"),
];

const FOCUS_HIDDEN: &[&str] = &[
    "national_focus_detail_view",
    "coninuous_focus_detail_view",
    "search_tooltip",
];

const ARMY_ROOTS: &[RootSpec] = &[RootSpec {
    file: "interface/countryarmyview.gui",
    name: "countryarmyview",
    position_override: None,
    use_show_position: true,
}];

const ARMY_TEMPLATES: &[TemplateSpec] = &[
    TemplateSpec {
        file: "interface/countryarmyview.gui",
        name: "armyview_army_entry",
        prefix: "army_row_0_",
        x: 16,
        y: 486,
        text: &[
            ("name", "1st Army"),
            ("theatre", "Theatre TAG"),
            ("group", "Group A"),
        ],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/countryarmyview.gui",
        name: "armyview_army_entry",
        prefix: "army_row_1_",
        x: 16,
        y: 526,
        text: &[
            ("name", "2nd Army"),
            ("theatre", "Front TAG"),
            ("group", "Group B"),
        ],
        sprite_overrides: &[],
    },
];

const ARMY_TEXT: &[(&str, &str)] = &[
    ("filter", "TAG"),
    ("unit_count", "24"),
    ("battalion_count", "216"),
    ("special_forces_cap", "12/24"),
    ("theatres_count", "3"),
];

const ARMY_HIDDEN: &[&str] = &[
    "navy_header",
    "navy_units",
    "air_header",
    "air_units",
    "foreign_template_flag",
];

const DIVISION_ROOTS: &[RootSpec] = &[RootSpec {
    file: "interface/divisiondesignerview.gui",
    name: "countrydivisiondesignerview",
    position_override: Some((360, 180)),
    use_show_position: false,
}];

const DIVISION_TEMPLATES: &[TemplateSpec] = &[
    TemplateSpec {
        file: "interface/divisiondesignerview.gui",
        name: "designer_division_regiment_slot_entry",
        prefix: "division_regiment_0_",
        x: 420,
        y: 454,
        text: &[("name", "INF")],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/divisiondesignerview.gui",
        name: "designer_division_regiment_slot_entry",
        prefix: "division_regiment_1_",
        x: 526,
        y: 454,
        text: &[("name", "ART")],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/divisiondesignerview.gui",
        name: "designer_division_support_slot_entry",
        prefix: "division_support_0_",
        x: 912,
        y: 480,
        text: &[("name", "ENG")],
        sprite_overrides: &[],
    },
    TemplateSpec {
        file: "interface/divisiondesignerview.gui",
        name: "designer_stat_entry",
        prefix: "division_stat_0_",
        x: 1032,
        y: 278,
        text: &[("count", "123"), ("label", "Fake Stat")],
        sprite_overrides: &[],
    },
];

const DIVISION_TEXT: &[(&str, &str)] = &[
    ("division_title", "Fake Division"),
    ("experience", "123"),
    ("production_resources", "Steel 1"),
    ("production_value", "9.99"),
    ("combat_width_text", "18"),
    ("support_header_label", "Support"),
    ("regimental_support_header_label", "Regiments"),
];

const DIVISION_HIDDEN: &[&str] = &[
    "hq_overlay_images",
    "toggle_model_selector_button",
    "niche_icon_disabled",
    "subunits_hq",
    "designerregimentswindow",
    "division_equipment_view",
    "div_templ_custom_icon_view",
    "div_template_select_model",
    "division_regiment_0_Background",
    "division_regiment_1_Background",
];

const VIEW_SPECS: &[ViewSpec] = &[
    ViewSpec {
        key: "production",
        display_name: "production",
        roots: PRODUCTION_ROOTS,
        templates: PRODUCTION_TEMPLATES,
        text: PRODUCTION_TEXT,
        progress: PRODUCTION_PROGRESS,
        hidden: &[],
        skip_notes: &["production item grid is runtime-populated; two static line templates are sampled"],
    },
    ViewSpec {
        key: "construction",
        display_name: "construction",
        roots: CONSTRUCTION_ROOTS,
        templates: CONSTRUCTION_TEMPLATES,
        text: CONSTRUCTION_TEXT,
        progress: CONSTRUCTION_PROGRESS,
        hidden: &[],
        skip_notes: &["construction queue/building grids are runtime-populated; sample building/static entries are overlaid"],
    },
    ViewSpec {
        key: "research",
        display_name: "research/tech tree",
        roots: RESEARCH_ROOTS,
        templates: RESEARCH_TEMPLATES,
        text: RESEARCH_TEXT,
        progress: RESEARCH_PROGRESS,
        hidden: RESEARCH_HIDDEN,
        skip_notes: &["inactive tech folders and runtime tech grids are documented skips; infantry folder and sample tech templates are composed"],
    },
    ViewSpec {
        key: "trade",
        display_name: "trade",
        roots: TRADE_ROOTS,
        templates: TRADE_TEMPLATES,
        text: TRADE_TEXT,
        progress: &[],
        hidden: &[],
        skip_notes: &["resource, country, and flag grids are runtime-populated; sample trade rows are overlaid"],
    },
    ViewSpec {
        key: "diplomacy",
        display_name: "diplomacy major-country view",
        roots: DIPLOMACY_ROOTS,
        templates: DIPLOMACY_TEMPLATES,
        text: DIPLOMACY_TEXT,
        progress: &[],
        hidden: DIPLOMACY_HIDDEN,
        skip_notes: &["diplomatic action/list grids and inactive action controllers are runtime-populated"],
    },
    ViewSpec {
        key: "focus",
        display_name: "focus tree chrome",
        roots: FOCUS_ROOTS,
        templates: FOCUS_TEMPLATES,
        text: FOCUS_TEXT,
        progress: &[],
        hidden: FOCUS_HIDDEN,
        skip_notes: &["focus grid contents are runtime-populated; sample focus icons use GFX_focus_* sprite overrides"],
    },
    ViewSpec {
        key: "army_ledger",
        display_name: "army ledger",
        roots: ARMY_ROOTS,
        templates: ARMY_TEMPLATES,
        text: ARMY_TEXT,
        progress: &[],
        hidden: ARMY_HIDDEN,
        skip_notes: &["army, navy, and air unit grids are runtime-populated; army rows are sampled"],
    },
    ViewSpec {
        key: "division_designer",
        display_name: "division designer chrome",
        roots: DIVISION_ROOTS,
        templates: DIVISION_TEMPLATES,
        text: DIVISION_TEXT,
        progress: &[],
        hidden: DIVISION_HIDDEN,
        skip_notes: &["subunit/stat grids and modal selector surfaces are runtime-populated; sample regiment/support/stat entries are overlaid"],
    },
];

pub fn inspect_major_views(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
) -> Result<Vec<MajorViewReport>> {
    compose_major_views(resolver, registry)
        .map(|results| results.into_iter().map(|result| result.report).collect())
}

pub fn write_major_view_proofs(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    out_dir: &Path,
) -> Result<Vec<(String, PathBuf)>> {
    let mut paths = Vec::new();
    for result in compose_major_views(resolver, registry)? {
        let path = out_dir.join(major_view_png_name(&result.report.key));
        write_png(&path, &result.output.image)?;
        paths.push((result.report.key, path));
    }
    Ok(paths)
}

pub fn major_view_png_name(key: &str) -> String {
    format!("{key}_view_composited.png")
}

fn compose_major_views(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
) -> Result<Vec<MajorViewComposeResult>> {
    VIEW_SPECS
        .iter()
        .map(|spec| compose_view(resolver, registry, spec))
        .collect()
}

fn compose_view(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    spec: &ViewSpec,
) -> Result<MajorViewComposeResult> {
    let mut source_files = BTreeSet::new();
    let mut root_windows = Vec::new();
    let mut children = Vec::new();

    for root in spec.roots {
        let mut element = load_named_element(resolver, root.file, root.name)?;
        if root.use_show_position {
            promote_show_position(&mut element);
        }
        if let Some((x, y)) = root.position_override {
            set_position(&mut element, x, y);
            element.orientation = Some("UPPER_LEFT".to_string());
        }
        source_files.insert(root.file.to_string());
        root_windows.push(root.name.to_string());
        children.push(element);
    }

    let mut options = localized_options(resolver)?;
    for &(name, value) in spec.text {
        options
            .text_overrides
            .insert(name.to_string(), value.to_string());
    }
    for &(name, value) in spec.progress {
        options.progress_overrides.insert(name.to_string(), value);
    }
    for &name in spec.hidden {
        options.hidden_elements.insert(name.to_string());
    }

    for template in spec.templates {
        let mut element = load_named_element(resolver, template.file, template.name)?;
        set_position(&mut element, template.x, template.y);
        element.orientation = Some("UPPER_LEFT".to_string());
        prefix_element_names(&mut element, template.prefix);
        for &(name, value) in template.text {
            options
                .text_overrides
                .insert(format!("{}{}", template.prefix, name), value.to_string());
        }
        for &(name, sprite_name) in template.sprite_overrides {
            let image = sprite_override_image(resolver, registry, sprite_name)?;
            options
                .sprite_overrides
                .insert(format!("{}{}", template.prefix, name), image);
        }
        for &name in spec.hidden {
            options
                .hidden_elements
                .insert(format!("{}{}", template.prefix, name));
        }
        source_files.insert(template.file.to_string());
        root_windows.push(format!("{} as {}*", template.name, template.prefix));
        children.push(element);
    }

    let wrapper = wrapper_root(spec.key, children);
    let hidden = options.hidden_elements.clone();
    let mut documented_skips = Vec::new();
    for note in spec.skip_notes {
        documented_skips.push(format!("note: {note}"));
    }
    collect_documented_skips(&wrapper, &hidden, "", &mut documented_skips);

    let mut compositor = Compositor::new(resolver, registry);
    let output = compositor.compose(&wrapper, &options);
    let report = report_from_output(
        spec,
        source_files.into_iter().collect(),
        root_windows,
        documented_skips,
        &output,
    );
    Ok(MajorViewComposeResult { report, output })
}

fn report_from_output(
    spec: &ViewSpec,
    source_files: Vec<String>,
    root_windows: Vec<String>,
    documented_skips: Vec<String>,
    output: &ComposeOutput,
) -> MajorViewReport {
    MajorViewReport {
        key: spec.key.to_string(),
        display_name: spec.display_name.to_string(),
        source_files,
        root_windows,
        sample_drawn_elements: output.report.drawn_elements,
        sample_missing_sprites: output.report.missing_sprites.len(),
        sample_missing_sprite_names: output.report.missing_sprites.iter().cloned().collect(),
        sample_undecodable_sprites: output.report.undecodable_sprites.len(),
        sample_undecodable_sprite_errors: output.report.undecodable_sprites.clone(),
        sample_missing_fonts: output.report.missing_fonts.len(),
        sample_missing_font_errors: output.report.missing_fonts.iter().cloned().collect(),
        sample_missing_glyphs: stringify_missing_glyphs(&output.report.missing_glyphs),
        sample_unresolved_loc_keys: output.report.unresolved_loc_keys.iter().cloned().collect(),
        documented_skips,
        proof_png: None,
    }
}

fn localized_options(resolver: &AssetResolver) -> Result<ComposeOptions> {
    let localisation = Localisation::load_english(resolver)?;
    Ok(ComposeOptions::new(MAJOR_VIEW_WIDTH, MAJOR_VIEW_HEIGHT).with_localisation(&localisation))
}

fn load_named_element(resolver: &AssetResolver, file: &str, name: &str) -> Result<GuiElement> {
    let resolved = resolver.resolve_file(file)?;
    let doc = parse_file(&resolved.path)?;
    let mut layouts = LayoutSet::default();
    layouts.add_document(&resolved.rel_path, &doc);
    layouts
        .find_by_name(name)
        .into_iter()
        .next()
        .cloned()
        .ok_or_else(|| GameUiError::missing(format!("{file}: element {name} not found")))
}

fn wrapper_root(key: &str, children: Vec<GuiElement>) -> GuiElement {
    GuiElement {
        element_type: "containerWindowType".to_string(),
        name: format!("{key}_proof_root"),
        position: None,
        size: Some(pair(MAJOR_VIEW_WIDTH as i32, MAJOR_VIEW_HEIGHT as i32)),
        orientation: Some("UPPER_LEFT".to_string()),
        sprite_refs: Vec::new(),
        font: None,
        text: None,
        format: None,
        children,
        raw: Value::Object(Vec::new()),
    }
}

fn promote_show_position(element: &mut GuiElement) {
    if let Some(position) = element
        .raw
        .first_value_ci("show_position")
        .and_then(parse_measure_pair)
    {
        element.position = Some(position);
    }
}

fn set_position(element: &mut GuiElement, x: i32, y: i32) {
    element.position = Some(pair(x, y));
}

fn pair(x: i32, y: i32) -> MeasurePair {
    MeasurePair {
        x: parse_measure(&x.to_string()),
        y: parse_measure(&y.to_string()),
    }
}

fn prefix_element_names(element: &mut GuiElement, prefix: &str) {
    element.name = format!("{prefix}{}", element.name);
    for child in &mut element.children {
        prefix_element_names(child, prefix);
    }
}

fn sprite_override_image(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    sprite_name: &str,
) -> Result<RgbaImage> {
    let sprite = registry
        .get(sprite_name)
        .ok_or_else(|| GameUiError::missing(format!("sprite override missing: {sprite_name}")))?;
    let decoded = resolver.decode_sprite(sprite)?;
    let frame = decoded
        .frames
        .first()
        .ok_or_else(|| GameUiError::missing(format!("sprite {sprite_name} has no frames")))?;
    if frame.width <= 128 && frame.height <= 128 {
        return Ok(frame.clone());
    }
    let mut image = blank_image(96, 96);
    draw_scaled(&mut image, frame, 0, 0, 96, 96);
    Ok(image)
}

fn collect_documented_skips(
    element: &GuiElement,
    hidden: &BTreeSet<String>,
    path: &str,
    out: &mut Vec<String>,
) {
    let path = if path.is_empty() {
        element.name.clone()
    } else {
        format!("{path}/{}", element.name)
    };
    if hidden.contains(&element.name) {
        out.push(format!(
            "hidden runtime surface: {} ({})",
            path, element.element_type
        ));
        return;
    }
    if is_runtime_populated(&element.element_type) {
        out.push(format!(
            "runtime-populated element: {} ({})",
            path, element.element_type
        ));
    }
    for child in &element.children {
        collect_documented_skips(child, hidden, &path, out);
    }
}

fn is_runtime_populated(element_type: &str) -> bool {
    matches!(
        element_type.to_ascii_lowercase().as_str(),
        "gridboxtype" | "listboxtype" | "smoothlistboxtype" | "overlappingelementsboxtype"
    )
}

fn stringify_missing_glyphs(
    missing: &BTreeMap<String, BTreeSet<char>>,
) -> BTreeMap<String, String> {
    missing
        .iter()
        .map(|(key, chars)| (key.clone(), chars.iter().collect()))
        .collect()
}
