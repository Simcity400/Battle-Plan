//! Proof of the calculator against the game's own recorded caches:
//! - for every deployed division of a recording, the calculator over its template, its held
//!   equipment and its manpower, compared with its recorded panel cache (its anti-air and fuel
//!   capacity with its movement stats, its suppression with the stat block's word stored beside
//!   the panel's) stat by stat. A stat the deployed modifier pass changes by country or
//!   commander modifiers is compared after that pass ([`post`]), for a medal-free division;
//! - for every template, the calculator's width, manpower and training time over its effective
//!   definitions, compared with the template's own recorded base width, manpower and training
//!   time caches, which the game's template recalculation writes from the same definitions, at
//!   hours whose unit-bonus lists were read that hour (a template's caches follow a bonus the
//!   hour it is granted; a carried read of the list can be a day older).
//! A division or template whose inputs the comparison does not reproduce (an unread table,
//! equipment short of a requirement, a fuel-using design below full fuel) is counted by reason,
//! never compared.
//!
//! The proof is the owner's chosen standard: a shown row matches every comparison of a full run,
//! with at least [`MINIMUM_NONZERO`] nonzero values among them and every term of its pass
//! reproduced; one mismatch fails it. The run asserts that for every shown row and names each
//! unshown row's state. It does not attribute a deployed cache to the operands that built it (a
//! recording holds no evidence of them: a cache can predate a pool, bonus or commander change),
//! so these are full-run comparisons, not attributed ones.
//!
//! Run read-only on a recording (it writes nothing beside it):
//! `BATTLE_PLAN_GAME_DIR=<game> BATTLE_PLAN_DESIGNER_REPLAY=<recording> BATTLE_PLAN_DESIGNER_USER=<user folder>
//! cargo test -p dashboard-army designer_stats_match -- --ignored --nocapture`.
//! `BATTLE_PLAN_DESIGNER_HOURS` names hours (comma-separated; default every recorded hour), and
//! `BATTLE_PLAN_DESIGNER_TECHNOLOGY_ONLY=1` assumes no persistent unit bonuses, to show what
//! technologies alone explain. `BATTLE_PLAN_DESIGNER_EXPLAIN=<division>` prints that division's bonuses,
//! effective definitions, each group's contribution alone and its held designs.
//!
//! `designer_rows_of_named_countries` lists, for the countries `BATTLE_PLAN_DESIGNER_COUNTRIES` names
//! (comma-separated tags) at the first hour `BATTLE_PLAN_DESIGNER_HOURS` names (default the last
//! recorded hour), every template's name,
//! composition and designer preview rows, proven or not: what to read off the game's own
//! designer at that date.

use std::{collections::BTreeMap, path::PathBuf, sync::atomic::AtomicBool};

use dashboard_base::source::{Source, SourceKey};
use dashboard_core::config::ContentOptions;
use replay::container::SeekResult;
use replay_data::rows::{DivisionPanelStatsRow, TableId, tables};

use super::{Calculator, Persistent, definitions, design_stats, kernel};

mod post;

/// The stats an explanation lists.
const EXPLAINED: [&str; 7] = [
    "soft_attack",
    "hard_attack",
    "defense",
    "breakthrough",
    "ap_attack",
    "entrenchment",
    "reliability",
];

/// The panel slots the deployed modifier pass changes only by the division's medal modifiers:
/// compared for a division whose recorded history holds no awarded medal.
const MEDAL_ONLY: [&str; 4] = ["soft_attack", "hard_attack", "defense", "entrenchment"];

/// The panel slots the deployed modifier pass leaves as the calculator wrote them, times one
/// (suppression is the stat block's word stored beside the panel's).
const UNMODIFIED: [&str; 7] = [
    "reliability",
    "reliability_factor",
    "ap_attack",
    "initiative",
    "hardness",
    "suppression",
    "experience_loss_factor",
];

#[derive(Default)]
struct Report {
    compared: BTreeMap<&'static str, (u64, u64)>,
    /// Comparisons whose recorded value was nonzero, by stat: what the match exercised.
    nonzero: BTreeMap<&'static str, u64>,
    /// Comparisons whose deployed pass changed the calculator's value, by stat.
    passed: BTreeMap<&'static str, u64>,
    /// Researched technologies with a unit bonus at the first compared hour, by their level:
    /// what the level multiplier was exercised by.
    levels: BTreeMap<i32, u64>,
    mismatches: BTreeMap<&'static str, Vec<String>>,
    skipped: BTreeMap<String, u64>,
    /// Nonzero caches of unshown rows a refused division held, by stat and reason: where the
    /// witnesses a proof still needs were lost.
    lost: BTreeMap<(&'static str, String), u64>,
}

impl Report {
    fn skip(&mut self, why: impl Into<String>) {
        *self.skipped.entry(why.into()).or_default() += 1;
    }

    /// Skips a division whose cache holds nonzero `witnesses` of unshown rows, counting them.
    fn refuse(&mut self, witnesses: &[&'static str], why: impl Into<String>) {
        let why = why.into();
        for &stat in witnesses {
            *self.lost.entry((stat, why.clone())).or_default() += 1;
        }
        self.skip(why);
    }

    fn compare(
        &mut self,
        stat: &'static str,
        recorded: i64,
        equal: bool,
        detail: impl FnOnce() -> String,
    ) {
        if recorded != 0 {
            *self.nonzero.entry(stat).or_default() += 1;
        }
        let counts = self.compared.entry(stat).or_default();
        counts.0 += 1;
        if equal {
            counts.1 += 1;
        } else {
            let listed = self.mismatches.entry(stat).or_default();
            if listed.len() < 8 {
                listed.push(detail());
            }
        }
    }
}

/// The deployed pass's defines.
struct Defines {
    capture_base: i64,
    max_out_of_supply_days: i64,
    drop_penalty_duration: i64,
}

/// The template's priority sub-unit: its stored priority, else its first line cell's
/// definition, else none.
fn priority_subunit(
    template: &replay_data::rows::DivisionTemplateIconRow,
    tokens: &dashboard_text::table_access::TokenNames<'_>,
) -> Result<Option<String>, String> {
    if template.priority_unit != 0 {
        return Ok(Some(
            String::from_utf8_lossy(&template.priority_unit_name).into_owned(),
        ));
    }
    let layout = template
        .layout
        .require("layout")
        .map_err(|why| why.to_string())?
        .as_ref()
        .ok_or("the template's layout is not recorded")?;
    match layout.regiments.cells.first() {
        Some(cell) if cell.occupied && cell.token != 0 => tokens
            .name(cell.token)
            .map(|name| Some(name.to_owned()))
            .map_err(|why| why.to_string()),
        _ => Ok(None),
    }
}

/// What the deployed pass reads of a division besides its source trees.
fn context(
    calculator: &Calculator,
    defines: &Defines,
    status: &replay_data::rows::DivisionStatusRow,
    template: &replay_data::rows::DivisionTemplateIconRow,
    tokens: &dashboard_text::table_access::TokenNames<'_>,
) -> post::Context {
    let organisation = priority_subunit(template, tokens).and_then(|priority| {
        let Some(priority) = priority else {
            return Ok(Vec::new());
        };
        let subunit = calculator
            .content
            .subunit_named(&priority)
            .ok_or_else(|| format!("priority sub-unit {priority} has no definition"))?;
        Ok(std::iter::once(subunit.key.as_str())
            .chain(subunit.sheet.categories.iter().map(String::as_str))
            .map(post::organisation_key)
            .collect())
    });
    // The supply-deficit term is zero with no days out of supply (over a nonzero maximum), or
    // with positive maximum grace and a received ratio of at least one. The recent-drop term
    // compares the hour plus the duration with the last drop's hour, never later than the
    // hour: unreachable for a duration of zero or more.
    let days = status
        .out_of_supply_days
        .require("out of supply days")
        .ok()
        .copied()
        .flatten();
    let supplied = match days {
        Some(0) => defines.max_out_of_supply_days != 0,
        Some(_) => {
            status
                .supply_grace_maximum
                .require("supply grace")
                .is_ok_and(|&maximum| maximum > 0)
                && status.supply_exact().is_some_and(|[ratio, _]| ratio >= kernel::ONE)
        }
        None => false,
    };
    let morale = if defines.drop_penalty_duration < 0 {
        Err("the recent-drop penalty duration is negative".into())
    } else if !supplied {
        Err("the division's out-of-supply severity is not reproduced".into())
    } else {
        Ok(())
    };
    post::Context {
        entrenched: status.entrenchment > 0,
        organisation,
        capture_base: defines.capture_base,
        morale,
    }
}

/// Every template the hour records.
fn templates_of(
    frame: &replay_frame::frame::DecodedFrame,
) -> &[replay_data::rows::DivisionTemplateIconRow] {
    frame
        .complete::<tables::DivisionTemplateIcons>()
        .unwrap_or(&[])
}

/// The recording and content stack the environment names.
fn open() -> (Source, std::sync::Arc<dashboard_text::catalogue::Catalogue>) {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let path = PathBuf::from(std::env::var("BATTLE_PLAN_DESIGNER_REPLAY").or_else(|_| std::env::var("HOI4_DESIGNER_REPLAY")).expect("BATTLE_PLAN_DESIGNER_REPLAY"));
    let flag = AtomicBool::new(false);
    let source = Source::open(
        SourceKey {
            source_id: "designer-validation".into(),
            revision: dashboard_base::protocol::replay_revision(&path).unwrap(),
        },
        path,
        &flag,
    )
    .unwrap();
    let options = ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: PathBuf::from(std::env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| std::env::var("HOI4_GAME_DIR")).unwrap()),
            user_dir: PathBuf::from(
                std::env::var("BATTLE_PLAN_DESIGNER_USER").or_else(|_| std::env::var("HOI4_DESIGNER_USER")).expect("BATTLE_PLAN_DESIGNER_USER"),
            ),
            cache_dir: std::env::temp_dir().join("hoi4-designer-validation"),
        },
        allow_unverified: false,
        presentation: Default::default(),
    };
    let catalogue = dashboard_base::catalogue::catalogue(&source, &options).unwrap();
    (source, catalogue)
}

#[test]
#[ignore = "reads a recording and the installation named by the environment"]
fn designer_rows_of_named_countries() {
    let (source, catalogue) = open();
    let calculator = Calculator::read(&catalogue).unwrap();
    let technologies = catalogue
        .scripted_content()
        .and_then(|content| content.effects())
        .and_then(|effects| effects.technologies())
        .unwrap();
    let tags: Vec<String> = std::env::var("BATTLE_PLAN_DESIGNER_COUNTRIES").or_else(|_| std::env::var("HOI4_DESIGNER_COUNTRIES"))
        .expect("BATTLE_PLAN_DESIGNER_COUNTRIES")
        .split(',')
        .map(|tag| tag.trim().to_owned())
        .collect();
    let replay = source.replay_ref();
    let branch = replay.branches.first().unwrap().id;
    let hour: i32 = std::env::var("BATTLE_PLAN_DESIGNER_HOURS").or_else(|_| std::env::var("HOI4_DESIGNER_HOURS")).map_or_else(
        |_| {
            replay
                .index
                .iter()
                .filter(|block| block.branch == branch)
                .map(|block| block.last_hour)
                .max()
                .unwrap()
        },
        |hours| hours.split(',').next().unwrap().trim().parse().unwrap(),
    );
    let mut tables: Vec<TableId> = vec![
        TableId::Countries,
        TableId::DivisionTemplateIcons,
        TableId::DivisionTemplates,
    ];
    tables.extend(super::TABLES);
    let SeekResult::Frame(frame) = replay
        .seek_selected_with(branch, hour, Some(&tables), || Ok(()))
        .unwrap()
    else {
        panic!("hour {hour} is not recorded");
    };
    let countries = crate::products::army_content::Countries::read(&frame).unwrap();
    let tokens = dashboard_text::table_access::TokenNames::new(&frame).unwrap();
    let roster = frame.complete::<tables::DivisionTemplates>().unwrap();
    println!("date {}", dashboard_text::division_history::date(hour));
    for template in templates_of(&frame) {
        let Some(owner) = roster
            .iter()
            .find(|row| row.template == template.template)
            .map(|row| row.owner)
        else {
            continue;
        };
        let Some(tag) = countries.tag(owner) else {
            continue;
        };
        if !tags.contains(&tag) {
            continue;
        }
        let name = crate::products::army_content::template_name(
            &frame,
            template.template,
            Some(&catalogue),
        )
        .unwrap_or_else(|why| why);
        let composition = template
            .layout
            .require("layout")
            .ok()
            .and_then(|layout| layout.as_ref())
            .map(|layout| {
                [&layout.regiments, &layout.support, &layout.regimental_support]
                    .iter()
                    .map(|grid| {
                        grid.cells
                            .iter()
                            .filter(|cell| cell.occupied && cell.token != 0)
                            .map(|cell| tokens.name(cell.token).unwrap_or("?"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .unwrap_or_default();
        println!("{tag} template {} \"{name}\": {composition}", template.template);
        match super::preview(&calculator, &frame, technologies, template, owner) {
            Ok(preview) => {
                let block = match &preview.block {
                    Ok(block) => block.clone(),
                    Err(why) => {
                        println!("  equipment-dependent stats refused: {why}");
                        continue;
                    }
                };
                for (name, amount) in &preview.requirements {
                    println!(
                        "  requires {name} = {}",
                        super::rows::required_quantity(*amount)
                    );
                }
                for (_, operand, _) in super::rows::ROSTER {
                    let (key, value) = match operand {
                        super::rows::Operand::Stat(stat) => {
                            let index =
                                installed_content_core::equipment::stat_index(stat.as_bytes()).unwrap();
                            (stat, block.stats[index])
                        }
                        super::rows::Operand::CombatWidth => {
                            ("combat_width", Some(block.combat_width))
                        }
                        super::rows::Operand::Manpower => {
                            ("manpower", Some(i64::from(block.manpower) * kernel::ONE))
                        }
                        super::rows::Operand::TrainingTime => (
                            "training_time",
                            Some(i64::from(block.training_time) * kernel::ONE),
                        ),
                    };
                    let shown = if super::rows::PROVEN.contains(&key) {
                        "shown"
                    } else {
                        "unavailable"
                    };
                    let value = value.map_or("none".into(), |value| {
                        installed_content_core::fixed5::fixed5_to_string(value, 3)
                    });
                    println!("  {key} = {value} ({shown})");
                }
            }
            Err(why) => println!("  preview refused: {why}"),
        }
    }
}

#[test]
#[ignore = "reads a recording and the installation named by the environment"]
fn designer_stats_match_recorded_division_caches() {
    let (source, catalogue) = open();
    let calculator = Calculator::read(&catalogue).unwrap();
    let define = |name: &str| {
        catalogue
            .defines()
            .unwrap()
            .fixed5("NMilitary", name)
            .unwrap()
    };
    let defines = Defines {
        capture_base: define("BASE_CAPTURE_EQUIPMENT_RATIO"),
        max_out_of_supply_days: define("MAX_OUT_OF_SUPPLY_DAYS"),
        drop_penalty_duration: define("PARACHUTE_ORG_REGAIN_PENALTY_DURATION"),
    };
    let technologies = catalogue
        .scripted_content()
        .and_then(|content| content.effects())
        .and_then(|effects| effects.technologies())
        .unwrap();
    let explain: Option<u64> = std::env::var("BATTLE_PLAN_DESIGNER_EXPLAIN").or_else(|_| std::env::var("HOI4_DESIGNER_EXPLAIN"))
        .ok()
        .map(|division| division.parse().unwrap());
    let persistent = if std::env::var_os("BATTLE_PLAN_DESIGNER_TECHNOLOGY_ONLY").or_else(|| std::env::var_os("HOI4_DESIGNER_TECHNOLOGY_ONLY")).is_some() {
        Persistent::AssumedNone
    } else {
        Persistent::Recorded
    };
    let wanted: Option<Vec<i32>> = std::env::var("BATTLE_PLAN_DESIGNER_HOURS").or_else(|_| std::env::var("HOI4_DESIGNER_HOURS")).ok().map(|hours| {
        hours
            .split(',')
            .map(|hour| hour.trim().parse().unwrap())
            .collect()
    });
    let mut tables: Vec<TableId> = vec![
        TableId::Divisions,
        TableId::DivisionStatus,
        TableId::DivisionHistory,
        TableId::DivisionPanelStats,
        TableId::DivisionEquipment,
        TableId::NewDivisionEquipment,
        TableId::DivisionLandInputs,
        TableId::DivisionTemplateIcons,
        TableId::DivisionTemplates,
        TableId::DivisionPanelSources,
        TableId::DivisionMovement,
        TableId::ModifierDefinitions,
    ];
    tables.extend(super::TABLES);
    let slots = &calculator.slots;
    let replay = source.replay_ref();
    let mut report = Report::default();
    for branch in &replay.branches {
        let blocks = replay
            .index
            .iter()
            .filter(|block| block.branch == branch.id);
        let hours = wanted.clone().unwrap_or_else(|| {
            blocks
                .flat_map(|block| block.first_hour..=block.last_hour)
                .collect()
        });
        for hour in hours {
            let frame = match replay
                .seek_selected_with(branch.id, hour, Some(&tables), || Ok(()))
                .unwrap()
            {
                SeekResult::Frame(frame) => frame,
                SeekResult::Gap { reason, .. } => {
                    report.skip(format!("hour {hour} is not recorded: {reason}"));
                    continue;
                }
            };
            if report.levels.is_empty()
                && let (Ok(research), Ok(definitions)) = (
                    frame.complete::<tables::ResearchTechnologies>(),
                    frame.complete::<tables::TechnologyDefinitions>(),
                )
            {
                for row in research.iter().filter(|row| row.researched != 0) {
                    let bonus = definitions
                        .binary_search_by_key(&row.database_index, |definition| {
                            definition.database_index
                        })
                        .ok()
                        .and_then(|at| technologies.get(&definitions[at].id).ok().flatten())
                        .is_some_and(|template| !template.unit_bonus.is_empty());
                    if bonus {
                        *report.levels.entry(row.level).or_default() += 1;
                    }
                }
            }
            // Templates compare only where the countries' unit-bonus lists were read at this very
            // hour: a template's caches follow a bonus the hour it is granted, while a carried
            // read of the list can be a day older.
            let bonuses_read =
                frame.tables[TableId::CountryUnitBonuses.index()].sampled_at() == Some(hour);
            if !bonuses_read {
                report.skip("templates: unit bonuses not read at this hour");
            }
            for template in templates_of(&frame).iter().filter(|_| bonuses_read) {
                let Some(settings) = template
                    .settings
                    .require("settings")
                    .ok()
                    .cloned()
                    .flatten()
                else {
                    report.skip("a template's settings are not recorded");
                    continue;
                };
                let Some(base_width) = template
                    .base_width
                    .require("base width")
                    .ok()
                    .copied()
                    .flatten()
                else {
                    report.skip("a template's base width is not recorded");
                    continue;
                };
                let groups =
                    match definitions(&calculator, &frame, technologies, template, persistent) {
                        Ok(groups) => groups,
                        Err(why) => {
                            report.skip(format!("template: {why}"));
                            continue;
                        }
                    };
                let groups: Vec<kernel::Group<'_>> = groups
                    .iter()
                    .map(|(definition, count)| kernel::Group {
                        definition,
                        count: *count,
                        critical: kernel::ONE,
                    })
                    .collect();
                let block = match kernel::calculate(slots, &groups, &[], &calculator.operands) {
                    Ok(block) => block,
                    Err(why) => {
                        report.skip(format!("template: {why}"));
                        continue;
                    }
                };
                let detail = |what: &str, calculated: i64, recorded: i64| {
                    format!(
                        "template {} at hour {hour}: {what} calculated {calculated}, recorded {recorded}",
                        template.template
                    )
                };
                for (stat, calculated, recorded) in [
                    ("combat_width", block.combat_width, base_width),
                    (
                        "manpower",
                        i64::from(block.manpower),
                        i64::from(settings.total_manpower),
                    ),
                    (
                        "training_time",
                        i64::from(block.training_time),
                        i64::from(settings.training_time),
                    ),
                ] {
                    report.compare(stat, recorded, calculated == recorded, || {
                        detail(stat, calculated, recorded)
                    });
                }
            }
            // Deployed divisions compare only where their daily equipment pools and the
            // countries' unit-bonus lists were read at this very hour: a carried read can be
            // a day older than the cache it is compared with.
            let fresh = [TableId::DivisionEquipment, TableId::CountryUnitBonuses]
                .into_iter()
                .all(|table| frame.tables[table.index()].sampled_at() == Some(hour));
            if !fresh {
                report.skip("division pools or unit bonuses not read at this hour");
                continue;
            }
            let divisions = frame.complete::<tables::Divisions>().unwrap();
            let statuses = frame.complete::<tables::DivisionStatus>().unwrap();
            // Divisions whose recorded history holds no awarded medal: their medal modifiers are
            // none.
            let mut histories = BTreeMap::<u64, (bool, bool)>::new();
            if let Ok(rows) = frame.complete::<tables::DivisionHistory>() {
                for row in rows {
                    let entry = histories.entry(row.division).or_default();
                    entry.0 |= row.list == replay_data::rows::DivisionHistoryRow::SUMMARY
                        && row.available != 0;
                    entry.1 |= row.list == replay_data::rows::DivisionHistoryRow::AWARDS;
                }
            }
            let panels = frame.complete::<tables::DivisionPanelStats>().unwrap();
            let registry = replay_derive::modifier_registry::Registry::from_frame(&frame);
            let source_rows = frame.complete::<tables::DivisionPanelSources>();
            let movements = frame.complete::<tables::DivisionMovement>().unwrap_or(&[]);
            let templates = frame.complete::<tables::DivisionTemplateIcons>().unwrap();
            let manpower = frame.complete::<tables::DivisionLandInputs>().unwrap();
            let mut pools: BTreeMap<u64, Vec<(u32, u64, i64)>> = BTreeMap::new();
            for table in [
                frame.complete::<tables::DivisionEquipment>(),
                frame.complete::<tables::NewDivisionEquipment>(),
            ] {
                for row in table.unwrap() {
                    pools.entry(row.division).or_default().push((
                        row.ordinal,
                        row.variant_handle,
                        row.amount_raw,
                    ));
                }
            }
            let tokens = dashboard_text::table_access::TokenNames::new(&frame).unwrap();
            let variants = frame.complete::<tables::VariantDefinitions>().unwrap();
            let mut designs: BTreeMap<
                u64,
                Result<(usize, [i64; installed_content_core::equipment::STAT_COUNT]), String>,
            > = BTreeMap::new();
            for division in divisions
                .iter()
                .filter(|division| division.reveal_skip == 0)
            {
                let Ok(panel) = panels.binary_search_by_key(&division.id, |row| row.division)
                else {
                    report.skip("no recorded panel cache");
                    continue;
                };
                let panel = &panels[panel];
                // The unshown rows this cache holds a nonzero witness of.
                let witnesses: Vec<&'static str> = super::rows::UNPROVEN
                    .iter()
                    .map(|(stat, _)| *stat)
                    .filter(|stat| {
                        installed_content_core::equipment::stat_index(stat.as_bytes())
                            .and_then(DivisionPanelStatsRow::stored)
                            .and_then(|at| panel.panel_stats[at])
                            .is_some_and(|value| value != 0)
                    })
                    .collect();
                let Ok(template) =
                    templates.binary_search_by_key(&division.template, |row| row.template)
                else {
                    report.refuse(&witnesses, "its template's composition is not recorded");
                    continue;
                };
                let template = &templates[template];
                let groups =
                    match definitions(&calculator, &frame, technologies, template, persistent) {
                        Ok(groups) => groups,
                        Err(why) => {
                            report.refuse(&witnesses, why);
                            continue;
                        }
                    };
                // The designer's preview is at full strength: a division short of manpower or
                // equipment (its recorded strength factor below one) exercises fill operands the
                // preview never reaches, and its current manpower source is not the game's own.
                let status = statuses
                    .binary_search_by_key(&division.id, |row| row.division)
                    .ok()
                    .map(|at| &statuses[at]);
                match status.map(|status| status.strength_factor) {
                    Some(kernel::ONE) => {}
                    Some(_) => {
                        report.refuse(&witnesses, "below full strength");
                        continue;
                    }
                    None => {
                        report.refuse(&witnesses, "no recorded status");
                        continue;
                    }
                }
                let Some(pool) = pools.get_mut(&division.id) else {
                    report.refuse(&witnesses, "no recorded equipment pool");
                    continue;
                };
                pool.sort_by_key(|(ordinal, ..)| *ordinal);
                let mut held = Vec::with_capacity(pool.len());
                let mut refused = None;
                for &(_, variant, amount) in pool.iter() {
                    let design = designs.entry(variant).or_insert_with(|| {
                        let definition = variants
                            .binary_search_by_key(&variant, |row| row.id)
                            .map(|at| &variants[at])
                            .map_err(|_| format!("design {variant} has no recorded definition"))?;
                        let key = calculator.content.equipment_key(&tokens, definition)?;
                        let archetype = calculator
                            .content
                            .archetype(key)
                            .map_err(|why| why.to_string())?;
                        let archetype = calculator
                            .content
                            .equipment
                            .get(archetype)
                            .ok_or("an archetype is not equipment")?;
                        Ok((
                            archetype,
                            design_stats(&frame, &calculator.content, variant)?,
                        ))
                    });
                    match design {
                        Ok((archetype, stats)) => held.push(kernel::Held {
                            stats: *stats,
                            archetype: *archetype,
                            amount,
                        }),
                        Err(why) => refused = Some(why.clone()),
                    }
                }
                if let Some(why) = refused {
                    report.refuse(&witnesses, why);
                    continue;
                }
                let uses_fuel = held
                    .iter()
                    .any(|design| design.stats[slots.fuel_consumption] > 0);
                let sources = match (
                    &registry,
                    &source_rows,
                    status.and_then(|status| Some((status, status.panel_source()?))),
                ) {
                    (Ok(registry), Ok(rows), Some((status, selected))) => post::Sources::new(
                        registry,
                        rows,
                        selected,
                        context(&calculator, &defines, status, template, &tokens),
                    ),
                    (Err(why), ..) => Err(format!("modifier definitions: {why}")),
                    (_, Err(why), _) => Err(format!("modifier sources: {why}")),
                    (.., None) => Err("the division's modifier sources are not selected".into()),
                };
                // Equipment short of any requirement leaves the critical-equipment fill, which the
                // comparison does not reproduce.
                let mut required = BTreeMap::<usize, i64>::new();
                for (definition, count) in &groups {
                    for &(archetype, amount) in &definition.requirements {
                        *required.entry(archetype).or_default() += amount * i64::from(*count);
                    }
                }
                let short = required.iter().any(|(archetype, need)| {
                    held.iter()
                        .filter(|design| design.archetype == *archetype)
                        .map(|design| design.amount)
                        .sum::<i64>()
                        < *need
                });
                if short {
                    report.refuse(&witnesses, "equipment short of a requirement");
                    continue;
                }
                let current = manpower
                    .binary_search_by_key(&division.id, |row| row.division)
                    .ok()
                    .map(|at| {
                        manpower[at]
                            .manpower
                            .iter()
                            .map(|contributor| i64::from(contributor.amount))
                            .sum::<i64>()
                    });
                let Some(current) = current else {
                    report.refuse(&witnesses, "no recorded manpower");
                    continue;
                };
                let Some(settings) = template
                    .settings
                    .require("settings")
                    .ok()
                    .cloned()
                    .flatten()
                else {
                    report.refuse(&witnesses, "the template's settings are not recorded");
                    continue;
                };
                let required_manpower = i64::from(settings.total_manpower);
                let manpower_fill = if required_manpower <= 0 {
                    kernel::ONE
                } else {
                    match kernel::div(current * kernel::ONE, required_manpower * kernel::ONE) {
                        Ok(fill) => kernel::ONE.min(fill),
                        Err(why) => {
                            report.refuse(&witnesses, why);
                            continue;
                        }
                    }
                };
                let operands = kernel::Operands {
                    manpower_fill,
                    ..calculator.operands
                };
                let groups: Vec<kernel::Group<'_>> = groups
                    .iter()
                    .map(|(definition, count)| kernel::Group {
                        definition,
                        count: *count,
                        critical: kernel::ONE,
                    })
                    .collect();
                let block = match kernel::calculate(slots, &groups, &held, &operands) {
                    Ok(block) => block,
                    Err(why) => {
                        report.refuse(&witnesses, why);
                        continue;
                    }
                };
                let medal_free = histories
                    .get(&division.id)
                    .is_some_and(|&(read, awarded)| read && !awarded);
                // A fuel-using design's equipment factor reads the division's fuel over its prior
                // deployed capacity: compared only at full fuel, where the factor is one. The
                // capacity's pass reads the medal store too, so its medal terms must be none.
                if uses_fuel && !medal_free {
                    report.refuse(&witnesses, 
                        "a held design uses fuel, and the medal terms of its capacity are not known",
                    );
                    continue;
                }
                if uses_fuel {
                    let fuel = movements
                        .binary_search_by_key(&division.id, |row| row.division)
                        .ok()
                        .and_then(|at| movements[at].fuel.require("fuel").ok().copied().flatten());
                    let capacity = match (&sources, block.stats[slots.fuel_capacity]) {
                        (Ok(sources), Some(capacity)) => sources.fuel_capacity(capacity).ok(),
                        _ => None,
                    };
                    match (fuel, capacity) {
                        (Some(fuel), Some(capacity)) if fuel >= capacity => {}
                        (Some(_), Some(_)) => {
                            report.refuse(&witnesses, "a held design uses fuel, and the division is below full fuel");
                            continue;
                        }
                        _ => {
                            report.refuse(&witnesses, 
                                "a held design uses fuel, and the division's fuel or capacity is not recorded",
                            );
                            continue;
                        }
                    }
                }
                if explain == Some(division.id) {
                    let named = |stats: &[i64; installed_content_core::equipment::STAT_COUNT]| {
                        EXPLAINED
                            .iter()
                            .map(|stat| {
                                let index =
                                    installed_content_core::equipment::stat_index(stat.as_bytes()).unwrap();
                                format!("{stat}={}", stats[index])
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    };
                    println!(
                        "explain division {} manpower fill {manpower_fill}",
                        division.id
                    );
                    let slot = dashboard_text::metadata::link_slot(
                        frame.complete::<tables::CountryLinks>().unwrap(),
                        template.country as u32,
                    )
                    .unwrap();
                    let links: BTreeMap<u32, u32> = frame
                        .complete::<tables::CountryLinks>()
                        .unwrap()
                        .iter()
                        .map(|row| (row.id, row.canonical_slot))
                        .collect();
                    let research = frame.complete::<tables::ResearchTechnologies>().unwrap();
                    let own: Vec<_> = research
                        .iter()
                        .filter(|row| links.get(&row.country) == Some(&slot))
                        .collect();
                    let definitions = frame.complete::<tables::TechnologyDefinitions>().unwrap();
                    println!(
                        "  research rows {} researched {} applied {} definitions {}",
                        own.len(),
                        own.iter().filter(|row| row.researched != 0).count(),
                        own.iter().filter(|row| row.applied_bonus_count > 0).count(),
                        definitions.len()
                    );
                    for row in own.iter().filter(|row| row.researched != 0).take(5) {
                        let id = definitions
                            .iter()
                            .find(|definition| definition.database_index == row.database_index)
                            .map(|definition| String::from_utf8_lossy(&definition.id).into_owned());
                        let bonus = id
                            .as_ref()
                            .and_then(|id| technologies.get(id.as_bytes()).ok().flatten())
                            .map(|template| template.unit_bonus.len());
                        println!(
                            "  tech {:?} applied {} level {} unit bonus blocks {:?}",
                            id, row.applied_bonus_count, row.level, bonus
                        );
                    }
                    for bonus in super::technology_bonuses(
                        &frame,
                        &calculator.content,
                        technologies,
                        &calculator.terrains,
                        slot,
                        &links,
                    )
                    .unwrap()
                    .into_iter()
                    .chain(super::persistent_bonuses(&frame, slot).unwrap_or_default())
                    {
                        let blocks: Vec<String> = bonus
                            .blocks
                            .iter()
                            .map(|block| {
                                format!(
                                    "{}{}:{:?}",
                                    if block.category { "cat " } else { "" },
                                    block.target,
                                    block.stats
                                )
                            })
                            .collect();
                        println!(
                            "  bonus {} x{} {}",
                            bonus.source,
                            bonus.multiplier,
                            blocks.join(" | ")
                        );
                    }
                    for group in &groups {
                        let alone =
                            kernel::calculate(slots, std::slice::from_ref(group), &held, &operands)
                                .unwrap();
                        println!(
                            "  alone: {} reqs={:?}",
                            EXPLAINED
                                .iter()
                                .map(|stat| {
                                    let index =
                                        installed_content_core::equipment::stat_index(stat.as_bytes())
                                            .unwrap();
                                    format!("{stat}={:?}", alone.stats[index])
                                })
                                .collect::<Vec<_>>()
                                .join(" "),
                            group.definition.requirements
                        );
                        println!(
                            "  group x{} support_type={} support_group={} categories={:?} grants={:?} width={} {}",
                            group.count,
                            group.definition.support_type,
                            group.definition.support_group,
                            group.definition.categories,
                            group.definition.grants,
                            group.definition.combat_width,
                            named(&group.definition.stats)
                        );
                    }
                    for design in &held {
                        println!(
                            "  held archetype {} amount {} {}",
                            design.archetype,
                            design.amount,
                            named(&design.stats)
                        );
                    }
                }
                let anti_air = statuses
                    .binary_search_by_key(&division.id, |row| row.division)
                    .ok()
                    .and_then(|at| {
                        statuses[at]
                            .movement_stats
                            .require("movement")
                            .ok()
                            .cloned()
                            .flatten()
                    })
                    .map(|movement| movement.air_defense);
                match anti_air {
                    Some(recorded) => {
                        let calculated = block.stats[slots.air_attack];
                        report.compare(
                            "air_attack",
                            recorded,
                            calculated == Some(recorded),
                            || {
                                format!(
                                    "division {}: calculated {calculated:?}, recorded {recorded}",
                                    division.id
                                )
                            },
                        );
                    }
                    None => report.refuse(&witnesses, "air_attack cache unavailable"),
                }
                // The stat block's maximum speed, read for a division with a route: the
                // calculator's speed after the deployed pass, which multiplies it by one.
                let base_speed = movements
                    .binary_search_by_key(&division.id, |row| row.division)
                    .ok()
                    .and_then(|at| {
                        movements[at]
                            .base_speed
                            .require("base speed")
                            .ok()
                            .copied()
                            .flatten()
                    });
                if let Some(recorded) = base_speed {
                    let calculated = block.stats[slots.maximum_speed];
                    let deployed = calculated.map(|speed| kernel::mul(speed, kernel::ONE));
                    report.compare("maximum_speed (moving)", recorded, deployed == Some(recorded), || {
                        format!(
                            "division {} (template {}, owner {}): calculated {calculated:?}, recorded {recorded}",
                            division.id, division.template, division.roster_owner
                        )
                    });
                }
                if !medal_free {
                    report.refuse(&witnesses, "medal-dependent stats: the division's history holds medals or was not read");
                }
                let medal_only: &[&str] = if medal_free { &MEDAL_ONLY } else { &[] };
                let passes: &[post::Pass] = match (&sources, medal_free) {
                    (Ok(_), true) => &post::PASSES,
                    (Err(why), true) => {
                        report.refuse(&witnesses, format!("deployed pass: {why}"));
                        &[]
                    }
                    _ => &[],
                };
                let recorded = |stat: &str| {
                    let index = installed_content_core::equipment::stat_index(stat.as_bytes()).unwrap();
                    let at = DivisionPanelStatsRow::stored(index)?;
                    Some((index, panel.panel_stats[at]))
                };
                let detail = |calculated: Option<i64>, deployed: Option<i64>, recorded: i64| {
                    format!(
                        "division {} (template {}, owner {}, hour {hour}): calculated {calculated:?}, deployed {deployed:?}, recorded {recorded}",
                        division.id, division.template, division.roster_owner
                    )
                };
                for &stat in UNMODIFIED.iter().chain(medal_only) {
                    let Some((index, value)) = recorded(stat) else {
                        report.skip(format!("{stat}: the panel cache holds no such word"));
                        continue;
                    };
                    let Some(value) = value else {
                        report.skip(format!("{stat} cache unavailable"));
                        continue;
                    };
                    // The pass's common stage: the value times one.
                    let calculated = block.stats[index];
                    let deployed = calculated.map(|value| kernel::mul(value, kernel::ONE));
                    report.compare(stat, value, deployed == Some(value), || {
                        detail(calculated, deployed, value)
                    });
                }
                if let Ok(sources) = &sources {
                    for pass in passes {
                        let Some((index, Some(value))) = recorded(pass.stat) else {
                            report.skip(format!("{} cache unavailable", pass.stat));
                            continue;
                        };
                        let calculated = block.stats[index];
                        let deployed = match calculated.map(|calculated| sources.apply(pass, calculated)) {
                            Some(Ok(deployed)) => Some(deployed),
                            Some(Err(why)) => {
                                report.skip(format!("{}: {why}", pass.stat));
                                continue;
                            }
                            None => None,
                        };
                        if deployed != calculated {
                            *report.passed.entry(pass.stat).or_default() += 1;
                        }
                        report.compare(pass.stat, value, deployed == Some(value), || {
                            format!(
                                "{}; {}",
                                detail(calculated, deployed, value),
                                sources.describe(pass)
                            )
                        });
                    }
                    // The fuel capacity's witness is the stat block's capacity word, after the
                    // capacity's deployed pass; compared where the other passes are.
                    if !passes.is_empty() {
                        let word = status.and_then(|status| {
                            status
                                .movement_stats
                                .require("movement")
                                .ok()
                                .cloned()
                                .flatten()
                                .map(|movement| movement.fuel_rate)
                        });
                        let calculated = block.stats[slots.fuel_capacity];
                        match (word, calculated.map(|value| sources.fuel_capacity(value))) {
                            (None, _) => report.skip("fuel_capacity cache unavailable"),
                            (Some(_), Some(Err(why))) => {
                                report.skip(format!("fuel_capacity: {why}"));
                            }
                            (Some(value), deployed) => {
                                let deployed = deployed.and_then(Result::ok);
                                if deployed != calculated {
                                    *report.passed.entry("fuel_capacity").or_default() += 1;
                                }
                                report.compare(
                                    "fuel_capacity",
                                    value,
                                    deployed == Some(value),
                                    || {
                                        format!(
                                            "{}; {}",
                                            detail(calculated, deployed, value),
                                            sources.describe(&post::FUEL_CAPACITY)
                                        )
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    println!(
        "compared (matched/compared, of which the recorded value was nonzero, and of which the \
         deployed pass changed the calculator's value):"
    );
    for (stat, (compared, matched)) in &report.compared {
        let nonzero = report.nonzero.get(stat).copied().unwrap_or_default();
        let passed = report.passed.get(stat).copied().unwrap_or_default();
        println!("  {stat}: {matched}/{compared}, nonzero {nonzero}, changed by the pass {passed}");
    }
    for pass in &post::PASSES {
        if let Some(term) = pass.unreproduced {
            println!("  {}: its pass's {term} is not reproduced", pass.stat);
        }
    }
    for (stat, listed) in &report.mismatches {
        for line in listed {
            println!("  mismatch {stat}: {line}");
        }
    }
    println!(
        "researched technologies with a unit bonus, by level: {:?}",
        report.levels
    );
    println!("not compared:");
    for (why, count) in &report.skipped {
        println!("  {why} (count: {count})");
    }
    assert!(
        report.compared.values().any(|(compared, _)| *compared > 0),
        "nothing was compared"
    );
    println!("nonzero caches of rows not shown, on divisions not compared:");
    for ((stat, why), count) in &report.lost {
        println!("  {stat}: {why} (count: {count})");
    }
    println!("acceptance of the rows not shown (full-run comparisons, not attributed to the operands that built each cache):");
    for (stat, _) in super::rows::UNPROVEN {
        println!("  {stat}: {}", report.acceptance(stat));
    }
    // The owner's standard: every stat the designer shows matched every full-run comparison, with
    // at least the minimum of nonzero values among them and no term of its pass left
    // unreproduced. One mismatch fails it.
    for stat in super::rows::PROVEN {
        assert_eq!(report.acceptance(stat), Acceptance::Proven, "{stat}");
    }
}

/// The fewest nonzero recorded values a shown row's comparisons must hold: a match only at zero
/// does not exercise its rule.
const MINIMUM_NONZERO: u64 = 1;

/// What a stat's comparisons establish about its designer value.
#[derive(Debug, PartialEq, Eq)]
enum Acceptance {
    /// Every comparison matched, nonzero among them, with every term of its pass reproduced.
    Proven,
    NotCompared,
    Mismatched { matched: u64, compared: u64 },
    OnlyZero,
    /// Every comparison matched, but its pass holds a term the recording lacks.
    Unreproduced(&'static str),
}

impl std::fmt::Display for Acceptance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Proven => write!(f, "every comparison matched, nonzero among them"),
            Self::NotCompared => write!(f, "never compared"),
            Self::Mismatched { matched, compared } => write!(f, "{matched}/{compared} matched"),
            Self::OnlyZero => write!(f, "matched, but with fewer than {MINIMUM_NONZERO} nonzero values"),
            Self::Unreproduced(term) => {
                write!(f, "matched, but its pass's {term} is not reproduced")
            }
        }
    }
}

impl Report {
    fn acceptance(&self, stat: &str) -> Acceptance {
        let (compared, matched) = self.compared.get(stat).copied().unwrap_or_default();
        if compared == 0 {
            return Acceptance::NotCompared;
        }
        if matched != compared {
            return Acceptance::Mismatched { matched, compared };
        }
        if self.nonzero.get(stat).copied().unwrap_or_default() < MINIMUM_NONZERO {
            return Acceptance::OnlyZero;
        }
        match post::PASSES
            .iter()
            .find(|pass| pass.stat == stat)
            .and_then(|pass| pass.unreproduced)
        {
            Some(term) => Acceptance::Unreproduced(term),
            None => Acceptance::Proven,
        }
    }
}
