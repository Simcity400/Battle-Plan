//! The division designer of a recorded template: [`ROOT`] with the template's composition in
//! its three grids ([`slots`]), its name, symbol, priority and capability header, the country's
//! Army experience, and the designer's stat rows and summary values
//! ([`dashboard_army::products::designer_stats`]), drawn read-only.
//!
//! The window is the designer the template's roster owner sees: a cell is attached in the
//! state the designer gives it ([`slots::state`]), a filled cell shows its definition's unit
//! glyph, an addable one the new-slot sprite and a locked or blocked one its lock or block
//! sprite. Nothing in the window acts: the editor's buttons are muted as the app mutes what
//! cannot be acted on, the save button's price is cleared, and no picker or overlay opens.
//!
//! Root, entry and control names are the binding contract: a stack that renames them refuses
//! the window or the control by name. A fact the recording cannot give (the country's
//! brigade-height unlock, the effective special-forces count, the equipment warning's
//! candidates, a stat not yet proven) leaves its control hidden or its cells out, named in the
//! reply's slots, never filled with a guess.

pub mod slots;

use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

use dashboard_army::products::designer_stats::{self, rows};
use dashboard_base::{Source, catalogue::Catalogue, metadata::check_cancel};
use dashboard_text::{
    catalogue::{CachedResult, View},
    subunit_requirements,
    table_access::TokenNames,
};
use installed_content_localisation::localisation::{Argument, Localisation};
use replay::container::SeekResult;
use replay_data::rows::{TableId, TemplateSettings, tables};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::map::native_gui::{Assets, SpriteSetter, assets_in, lifecycle::Instance};

use crate::{
    compose::{self, BoundGrid, Hover, Row},
    products::game_window::{Bound, SlotState as Slot, WindowSlot},
    resources::WindowResources,
    session::Session,
};
use slots::{GridKind, Operands, Plan, SlotState};

/// The designer's root window.
pub const ROOT: &str = "countrydivisiondesignerview";
/// The entry the regiment grid's cells are made of.
pub const REGIMENT_ENTRY: &str = "designer_division_regiment_slot_entry";
/// The entry the support and regimental support grids' cells are made of.
pub const SUPPORT_ENTRY: &str = "designer_division_support_slot_entry";

/// The header of an ordinary template and of an army headquarters.
const ORDINARY_HEADER: &str = "non_hq_view";
const HQ_HEADER: &str = "hq_view";
/// Shown only for army headquarters templates.
const HQ_ONLY: [&str; 3] = ["subunits_hq", "hq_overlay_images", "hq_title"];
/// Windows the designer opens on a click. The read-only window opens none.
const POPUPS: [&str; 5] = [
    "designerregimentswindow",
    "division_equipment_view",
    "div_templ_custom_icon_view",
    "div_template_select_model",
    "niche_icon_selector",
];
/// The colonial force flag, whose binding is not established; the window draws no country
/// flags in any case.
const COLONIAL_FLAG: &str = "colonial_force_flag";
/// The editor's buttons, which act and so are muted.
const EDITOR_BUTTONS: [&str; 3] = ["duplicate_button", "reset_button", "save_button"];
/// Hidden by the designer's setup.
const SAVE_AND_DEPLOY: &str = "save_and_deploy_button";
/// The summary values' text boxes, in the order of [`Designer::summary`].
const SUMMARY_TEXTS: [&str; 4] = [
    "hardness_stat/stat_text",
    "org_stat/stat_text",
    "strength_stat/stat_text",
    "speed_stat/stat_text",
];
const WIDTH_TEXT: &str = "combat_width_view/combat_width_text";
/// The production footer's cost and resource texts, whose selector endpoints are not bound yet:
/// their authored text is a placeholder.
const PRODUCTION_FOOTER: [&str; 2] = ["production_value", "production_resources"];
/// The stat grids and their row entry.
const STAT_GRIDS: [(rows::Grid, &str); 3] = [
    (rows::Grid::Base, "info/base_stats/stats_grid"),
    (rows::Grid::Combat, "info/combat_stats/stats_grid"),
    (rows::Grid::Equipment, "info/equipment_stats/stats_grid"),
];
pub const STAT_ENTRY: &str = "designer_stat_entry";
const STAT_LABEL: &str = "label";
const STAT_COUNT: &str = "count";
const EXPERIENCE: &str = "experience";
/// The template name's edit box within the header's `divisions` drop-down.
const NAME_BOX: &str = "name";
const EXPERIENCE_KEY: &str = "DESIGNER_ARMY_EXPERIENCE";

/// A cell's controls.
const CELL_BACKGROUND: &str = "Background";
const CELL_UNIT: &str = "unit_icon";
const CELL_EMPTY: &str = "invisible_icon";
const CELL_NAME: &str = "name";
const CELL_GLOW: &str = "glowup";
const CELL_HQ_REMOVE: &str = "unit_icon_hq_remove";

const REGIMENT_BACKGROUND: &[u8] = b"GFX_div_unit_item_bg_wide";
const NEW_SLOT: &[u8] = b"GFX_new_regiment_slot";
const LOCKED_SLOT: &[u8] = b"GFX_locked_icon";
const BLOCKED_SLOT: &[u8] = b"GFX_blocked_icon";
const EQUIPMENT_BUTTON: &[u8] = b"GFX_button_123x34";
const EQUIPMENT_BUTTON_RESTRICTED: &[u8] = b"GFX_button_123x34_alt";

/// The tables one hour of the designer reads, besides the preview's
/// ([`dashboard_army::products::designer_stats::TABLES`]).
const TABLES: [TableId; 7] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryExperience,
    TableId::CountryTemplateState,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::LexicalTokens,
];

/// A coloured medium land-unit glyph of a definition.
fn unit_glyph(definition: &str) -> Vec<u8> {
    format!("GFX_unit_{definition}_icon_medium").into_bytes()
}

/// The glyph's frame: the NATO frame when the viewer's game shows NATO symbols.
fn glyph_frame(nato: bool) -> i32 {
    if nato { 2 } else { 1 }
}

/// A template's custom symbol.
fn custom_symbol(index: i32) -> Vec<u8> {
    format!("GFX_div_templ_{index}_large").into_bytes()
}

/// Each priority button's frame: the selected stage's button shows its frame past the stage
/// count. One-based.
pub fn priority_frames(priority: i32, stages: i32) -> Vec<i32> {
    (0..stages)
        .map(|button| {
            if priority == button {
                button + 1 + stages
            } else {
                button + 1
            }
        })
        .collect()
}

/// The equipment button's sprite: the plain one only when the template takes new and foreign
/// equipment and excludes no design or type.
pub fn equipment_button(
    settings: &TemplateSettings,
    excluded: &[u64],
) -> Result<&'static [u8], String> {
    let forbidden = settings
        .forbidden_equipment
        .as_ref()
        .ok_or("the template's forbidden equipment types were not read")?;
    Ok(
        if settings.allow_new_equipment != 0
            && settings.allow_foreign_equipment != 0
            && excluded.is_empty()
            && forbidden.is_empty()
        {
            EQUIPMENT_BUTTON
        } else {
            EQUIPMENT_BUTTON_RESTRICTED
        },
    )
}

/// One composition cell as it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawnCell {
    pub cell: slots::Cell,
    /// The real definition's name, or why it is unknown.
    pub definition: Option<Result<String, String>>,
    pub hover: Option<Hover>,
}

/// One grid's drawn cells and refused states.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawnGrid {
    pub kind: GridKind,
    pub cells: Vec<DrawnCell>,
    pub refused: Vec<String>,
}

/// What the symbol button shows: a custom symbol, frame one, or a unit glyph, with its frame;
/// `None` when the designer hides the button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    /// Candidates in the designer's order: the first declared sprite is shown.
    pub candidates: Vec<(Vec<u8>, i32)>,
}

/// Everything one hour gives the designer of one template.
#[derive(Clone, Debug)]
pub struct Designer {
    pub hq: bool,
    pub name: Result<String, String>,
    pub grids: Result<Vec<DrawnGrid>, String>,
    pub symbol: Result<Symbol, String>,
    /// The stored priority and the priority stage count.
    pub priority: Result<(i32, i32), String>,
    /// Whether the regiment grid holds a real definition: the capability icons show, and the
    /// equipment button acts, only then.
    pub regiments: bool,
    pub paradrop: Result<bool, String>,
    pub equipment_button: Result<&'static [u8], String>,
    pub experience: Result<String, String>,
    /// The stat grids' rows, in the game's fixed order.
    pub stats: Vec<rows::Row>,
    /// The summary values (hardness, organisation, strength, speed) and the width text.
    pub summary: ([Result<String, String>; 4], Result<String, String>),
    /// When the stats' daily inputs were read, for every stat row's hover.
    pub basis: Vec<String>,
    pub read_hour: Option<i32>,
}

/// The number the experience text shows: the balance truncated to whole points.
pub fn experience_points(raw: i64) -> i32 {
    (raw / 100_000) as i32
}

/// Whether two raw country references are one country: equal, or both nonzero and linked to one
/// canonical country.
fn same_country(links: &BTreeMap<u32, u32>, a: u32, b: u32) -> bool {
    a == b
        || (a != 0
            && b != 0
            && links
                .get(&a)
                .is_some_and(|slot| links.get(&b) == Some(slot)))
}

/// The designer of `template` at the frame's hour: its facts and stats from the recorded
/// catalogue, its text (name, Army experience, stat labels and values, summaries) in `shown`,
/// the localisation the window is drawn in (`Catalogue::presentation_localisation`).
pub fn inputs(
    frame: &DecodedFrame,
    catalogue: &Catalogue,
    shown: &Localisation,
    template: u64,
    nato: bool,
) -> Result<Designer, String> {
    let roster = frame
        .complete::<tables::DivisionTemplates>()
        .map_err(|why| why.to_string())?;
    let owner = roster
        .iter()
        .find(|row| row.template == template)
        .ok_or("the template is not recorded at this hour")?
        .owner;
    let icons = frame
        .complete::<tables::DivisionTemplateIcons>()
        .map_err(|why| why.to_string())?;
    let icon = icons
        .binary_search_by_key(&template, |row| row.template)
        .map(|at| &icons[at])
        .map_err(|_| "the template's composition is not recorded at this hour")?;
    let settings = icon
        .settings
        .require("template settings")
        .and_then(|settings| {
            settings
                .clone()
                .ok_or_else(|| "the template's settings were refused at capture".to_owned())
        });
    let hq = settings.as_ref().map_err(Clone::clone)?.is_army_hq != 0;
    let links: BTreeMap<u32, u32> = frame
        .complete::<tables::CountryLinks>()
        .map(|rows| {
            rows.iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect()
        })
        .unwrap_or_default();
    let country = icon.country as u32;
    let tokens = TokenNames::new(frame).map_err(|why| why.to_string());
    let spell = |token: u32| -> Result<String, String> {
        let tokens = tokens.as_ref().map_err(Clone::clone)?;
        tokens
            .name(token)
            .map(str::to_owned)
            .map_err(|why| format!("definition token {token}: {why}"))
    };
    let localisation = shown;
    let name = dashboard_text::template_facts::display_name(frame, template, localisation);
    let defines = catalogue.defines();
    let layout = icon.layout.require("template layout").and_then(|layout| {
        layout
            .clone()
            .ok_or_else(|| "the template's layout was refused at capture".to_owned())
    });
    let regiments = layout.as_ref().is_ok_and(|layout| {
        slots::counts(&layout.regiments)
            .iter()
            .any(|&count| count > 0)
    });
    let editable = settings.as_ref().ok().and_then(|settings| {
        let lock_all = if (icon.country) > 0 {
            frame
                .complete::<tables::CountryTemplateState>()
                .ok()?
                .iter()
                .find(|row| same_country(&links, row.country, country))
                .map(|row| row.lock_all != 0)?
        } else {
            false
        };
        Some(!lock_all && settings.locked == 0 && settings.division_cap.is_none())
    });
    let grids = if hq {
        Err("an army headquarters' composition grids are not bound yet".to_owned())
    } else {
        layout.as_ref().map_err(Clone::clone).and_then(|layout| {
            let defines = defines.as_ref().map_err(Clone::clone)?;
            let int = |name: &str| {
                defines
                    .int32("NMilitary", name)
                    .map_err(|why| format!("NMilitary.{name}: {why}"))
            };
            let (low, high) = (
                int("MIN_DIVISION_BRIGADE_HEIGHT")?,
                int("MAX_DIVISION_BRIGADE_HEIGHT")?,
            );
            if low > high {
                return Err(format!(
                    "NMilitary.MIN_DIVISION_BRIGADE_HEIGHT {low} exceeds MAX_DIVISION_BRIGADE_HEIGHT {high}"
                ));
            }
            let thresholds = defines
                .numbers("NMilitary", "REGIMENTAL_SUPPORT_REQUIRED_BATTALIONS")
                .map_err(|why| format!("NMilitary.REGIMENTAL_SUPPORT_REQUIRED_BATTALIONS: {why}"))?
                .into_iter()
                .map(|value| {
                    (value.fract() == 0.0 && value.abs() <= f64::from(i32::MAX))
                        .then_some(value as i32)
                        .ok_or_else(|| {
                            format!("NMilitary.REGIMENTAL_SUPPORT_REQUIRED_BATTALIONS holds {value}, not a whole number")
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            // The window is the roster owner's designer.
            let viewing = same_country(&links, owner, country);
            let regiment_counts = slots::counts(&layout.regiments);
            Ok(GridKind::ALL
                .into_iter()
                .map(|kind| {
                    let grid = kind.grid(layout);
                    let limits = |column: usize| Operands {
                        limits: match kind {
                            GridKind::Regiments => (low..=high).collect(),
                            GridKind::Support => vec![grid.rows as i32],
                            GridKind::RegimentalSupport => vec![slots::regimental_support_rows(
                                &thresholds,
                                regiment_counts.get(column).copied().unwrap_or(0),
                            )],
                        },
                        editable,
                        hq,
                    };
                    let Plan { cells, refused } = slots::plan(
                        kind,
                        grid,
                        limits,
                        "the country's brigade height unlock is not recorded",
                    );
                    let cells = cells
                        .into_iter()
                        // Outside the composition country's own view an addable cell turns locked,
                        // and locked and blocked cells are not attached.
                        .filter(|cell| viewing || cell.state == SlotState::Filled)
                        .map(|cell| {
                            let definition = cell.definition.map(|(token, _)| spell(token));
                            let hover = definition.as_ref().map(|definition| Hover {
                                title: definition
                                    .as_ref()
                                    .ok()
                                    .and_then(|key| catalogue.localised(key, View::Plain))
                                    .and_then(|text| dashboard_base::catalogue::plain_text(&text).ok())
                                    .or_else(|| definition.clone().ok())
                                    .unwrap_or_else(|| "Unit name unavailable".to_owned()),
                                details: [
                                    definition.as_ref().ok().map(|key| format!("Key: {key}")),
                                    cell.definition
                                        .is_some_and(|(_, local)| local)
                                        .then(|| "The country's own copy of the unit".to_owned()),
                                ]
                                .into_iter()
                                .flatten()
                                .collect(),
                            });
                            DrawnCell {
                                cell,
                                definition,
                                hover,
                            }
                        })
                        .collect();
                    DrawnGrid {
                        kind,
                        cells,
                        refused,
                    }
                })
                .collect())
        })
    };
    // The symbol's unit: the stored priority definition, else the first regiment cell while the
    // grid holds a real definition.
    let symbol = layout.as_ref().map_err(Clone::clone).and_then(|layout| {
        let definition = if icon.priority_unit != 0 {
            Some(icon.priority_unit)
        } else if regiments {
            slots::real(&layout.regiments, 0, 0).map(|(token, _)| token)
        } else {
            None
        };
        let Some(definition) = definition else {
            return Ok(Symbol {
                candidates: Vec::new(),
            });
        };
        let mut candidates = Vec::new();
        if icon.custom_icon >= 0 && !(nato && icon.custom_icon_nato == 0) {
            candidates.push((custom_symbol(icon.custom_icon), 1));
        }
        candidates.push((unit_glyph(&spell(definition)?), glyph_frame(nato)));
        Ok(Symbol { candidates })
    });
    let priority = settings
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|settings| {
            let stages = defines
                .as_ref()
                .map_err(Clone::clone)?
                .int32("NCountry", "REINFORCEMENT_DIVISION_PRIORITY_COUNT")
                .map_err(|why| format!("NCountry.REINFORCEMENT_DIVISION_PRIORITY_COUNT: {why}"))?;
            Ok((settings.priority, stages))
        });
    let paradrop = paradrop(catalogue, icon, &spell);
    let equipment_button = settings
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|settings| equipment_button(settings, &icon.excluded_variants));
    let experience = frame
        .complete::<tables::CountryExperience>()
        .map_err(|why| why.to_string())
        .and_then(|rows| {
            rows.iter()
                .find(|row| same_country(&links, row.country, owner))
                .ok_or_else(|| "the country's Army experience is not recorded".to_owned())
        })
        .and_then(|row| experience_text(localisation, row.army_raw));
    // The designer's empty-template test: the leading line and support slots hold nothing.
    let empty = layout.as_ref().is_ok_and(|layout| {
        slots::real(&layout.regiments, 0, 0).is_none()
            && slots::real(&layout.support, 0, 0).is_none()
    });
    let preview = (|| {
        let calculator = designer_stats::Calculator::read(catalogue)?;
        let technologies = catalogue.scripted_content()?.effects()?.technologies()?;
        designer_stats::preview(&calculator, frame, technologies, icon, owner)
    })();
    // The designer is its country's own: its requirement rows use that country's names.
    let tag = dashboard_army::products::army_content::Countries::read(frame)
        .ok()
        .and_then(|countries| countries.tag(owner));
    let stats = rows::rows(localisation, &preview, empty, tag.as_deref());
    // The preview's daily inputs date its stats.
    let basis = [
        (TableId::CountryUnitBonuses, "unit bonuses"),
        (TableId::AvailableEquipmentVariants, "designs"),
    ]
    .into_iter()
    .map(
        |(table, what)| match frame.tables[table.index()].sampled_at() {
            Some(hour) => format!(
                "The country's {what} as read on {}",
                dashboard_text::division_history::date(hour)
            ),
            None => format!("The country's {what} are not read at this hour"),
        },
    )
    .collect();
    let summary = rows::summary(localisation, &preview);
    Ok(Designer {
        hq,
        name,
        grids,
        symbol,
        priority,
        regiments,
        paradrop,
        equipment_button,
        experience,
        stats,
        summary,
        basis,
        read_hour: frame.tables[TableId::DivisionTemplateIcons.index()].sampled_at(),
    })
}

/// The Army experience line for `raw` recorded experience, in `localisation`.
pub fn experience_text(localisation: &Localisation, raw: i64) -> Result<String, String> {
    localisation
        .localise_with(
            EXPERIENCE_KEY.as_bytes(),
            &[("VALUE", Argument::Int32(experience_points(raw)))],
        )
        .map_err(|why| format!("{EXPERIENCE_KEY}: {why:?}"))
}

/// Whether every grouped real definition of the composition may be parachuted.
fn paradrop(
    catalogue: &Catalogue,
    icon: &replay_data::rows::DivisionTemplateIconRow,
    spell: &dyn Fn(u32) -> Result<String, String>,
) -> Result<bool, String> {
    let content = catalogue
        .content
        .layers
        .derived::<CachedResult<subunit_requirements::Content>>()
        .get_or_init(|| subunit_requirements::Content::load(&catalogue.content.layers))
        .map_err(|why| format!("the sub-units' content does not read: {why}"))?;
    if icon.model_unreadable != 0 {
        return Err("the template's grouped sub-units were not readable".to_owned());
    }
    let mut every = true;
    for unit in icon.subunits.iter().filter(|unit| unit.token != 0) {
        let key = spell(unit.token)?;
        let subunit = content
            .subunit(&key)
            .ok_or_else(|| format!("sub-unit {key} is not in the content"))?;
        every &= subunit.can_be_parachuted.clone()?;
    }
    Ok(every)
}

/// The designer's hour of `template`, read for the hour asked.
fn hour(
    source: &Source,
    catalogue: &Catalogue,
    branch: u64,
    at: i32,
    template: u64,
    nato: bool,
    cancelled: &AtomicBool,
) -> io::Result<Result<Designer, String>> {
    let tables: Vec<TableId> = TABLES.into_iter().chain(designer_stats::TABLES).collect();
    let frame = match source
        .replay_ref()
        .seek_selected_with(branch, at, Some(&tables), || check_cancel(cancelled))?
    {
        SeekResult::Frame(frame) => frame,
        SeekResult::Gap { reason, .. } => {
            return Ok(Err(format!("the hour is not recorded: {reason}")));
        }
    };
    check_cancel(cancelled)?;
    // The window's text is the presentation's; its facts and stats are recorded.
    let shown = catalogue.presentation_localisation()?;
    Ok(inputs(&frame, catalogue, &shown, template, nato))
}

/// The designer of `template` at `at`.
#[allow(clippy::too_many_arguments)]
pub fn bind(
    source: &Source,
    catalogue: &Catalogue,
    session: &std::sync::Mutex<Session>,
    branch: u64,
    at: i32,
    template: u64,
    nato: bool,
    scroll: &BTreeMap<String, i32>,
    cancelled: &AtomicBool,
) -> io::Result<Bound> {
    let designer = match hour(source, catalogue, branch, at, template, nato, cancelled)? {
        Ok(designer) => designer,
        Err(why) => {
            return Ok(Bound {
                title: None,
                read_hour: None,
                scene: Err(vec![format!("{ROOT}: {why}")]),
                slots: Vec::new(),
                warnings: Vec::new(),
            });
        }
    };
    // The window shows the presentation's text; the stats above read the recorded facts.
    let localisation = catalogue.presentation_localisation()?;
    let mut session = session.lock().unwrap_or_else(|error| error.into_inner());
    check_cancel(cancelled)?;
    let mut bound = draw(
        &mut session,
        &catalogue.content.layers,
        &localisation,
        &designer,
        nato,
        scroll,
    )?;
    bound.read_hour = designer.read_hour;
    Ok(bound)
}

mod draw;
pub use draw::{check, draw};

#[cfg(test)]
mod tests;
