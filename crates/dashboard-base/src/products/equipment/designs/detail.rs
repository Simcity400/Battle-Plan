//! Plain equipment detail sections, evaluated only for the requested design.
use super::{queries, *};
use crate::{
    Source, catalogue,
    products::{LoadContext, Product, check_reply_budget},
};
use serde::{Deserialize, Serialize};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct EquipmentDesignDetail;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesignDetailParams {
    pub branch_id: String,
    pub hour: i32,
    pub variant: Option<String>,
    pub from_variant: Option<String>,
    pub ship: Option<String>,
    pub wing: Option<String>,
    pub observer: Option<u32>,
    pub target_country: Option<u32>,
    pub fog: Option<bool>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DesignChoice {
    pub id: String,
    pub name: String,
    pub creator: u32,
    pub obsolete: bool,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DesignField {
    pub label: String,
    pub value: Option<String>,
    pub refusal: Option<String>,
}
impl DesignField {
    fn known(label: impl Into<String>, value: impl ToString) -> Self {
        Self {
            label: label.into(),
            value: Some(value.to_string()),
            refusal: None,
        }
    }
    fn result(label: impl Into<String>, value: Result<String, String>) -> Self {
        match value {
            Ok(value) => Self::known(label, value),
            Err(reason) => Self {
                label: label.into(),
                value: None,
                refusal: Some(reason),
            },
        }
    }
    fn refused(label: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::result(label, Err(reason.into()))
    }
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DesignSection {
    pub title: String,
    pub fields: Vec<DesignField>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DesignDetailReply {
    pub warnings: Vec<String>,
    pub branch_id: String,
    pub hour: i32,
    pub designs: Vec<DesignChoice>,
    pub sections: Vec<DesignSection>,
}
crate::reply_warnings!(DesignDetailReply);

fn handle(value: &Option<String>) -> Result<Option<u64>, &'static str> {
    value
        .as_ref()
        .map(|value| {
            value
                .parse()
                .map_err(|_| "Invalid design or unit reference")
        })
        .transpose()
}
fn fixed(raw: i64) -> String {
    let sign = if raw < 0 { "-" } else { "" };
    let value = raw.unsigned_abs();
    format!("{sign}{}.{:05}", value / 100_000, value % 100_000)
}
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
fn section(title: &str, fields: Vec<DesignField>) -> DesignSection {
    DesignSection {
        title: title.into(),
        fields,
    }
}

/// Pure presentation projection, shared by product loading and synthetic tests.
// Keep design/unit selections and independently available content inputs explicit.
#[allow(clippy::too_many_arguments)]
pub fn inspect(
    frame: &DecodedFrame,
    id: u64,
    source: Option<u64>,
    ship: Option<u64>,
    wing: Option<u64>,
    content: &Database,
    cat: Option<&catalogue::Catalogue>,
    defines: Result<&installed_content_core::defines::Defines, &str>,
) -> Vec<DesignSection> {
    let definition = match queries::definition(frame, id) {
        Ok(row) => row,
        Err(reason) => {
            return vec![section(
                "Saved design",
                vec![DesignField::refused("Design", reason)],
            )];
        }
    };
    let mut sections = vec![section(
        "Saved design",
        vec![
            DesignField::known("Name", text(&definition.display_name)),
            DesignField::result(
                "Equipment",
                queries::spelling(frame, definition.equipment_token).map(text),
            ),
            DesignField::known("Creator", definition.creator_link),
            DesignField::known("Origin", definition.origin_link),
            DesignField::known("Obsolete", definition.obsolete != 0),
            DesignField::known("Model", text(&definition.explicit_entity)),
            DesignField::known(
                "Icon override",
                format!(
                    "{} / {} / {}",
                    definition.icon_active,
                    definition.icon_kind,
                    text(&definition.icon)
                ),
            ),
        ],
    )];
    let state = definition
        .design
        .require("VariantDefinitions.design")
        .map_err(|e| e.to_string());
    match &state {
        Ok(state) => sections.push(section(
            "Ancestry and designer",
            vec![
                DesignField::known(
                    "Parent",
                    state
                        .parent
                        .map_or_else(|| "None".into(), |id| id.to_string()),
                ),
                DesignField::known("Ancestor version ceiling", state.max_version),
                DesignField::known("Pending parent version", state.pending_parent_version),
                DesignField::known("Industrial organisation reference", state.design_team),
                DesignField::result(
                    "Organisation resolves for saving",
                    queries::author_live(frame, state.design_team).map(|live| live.to_string()),
                ),
                DesignField::known("Organisation bonus marker", state.design_team_traits),
                DesignField::known("Module priority sum", state.module_priority_sum),
                DesignField::known(
                    "Named bonus handles",
                    state
                        .named_bonuses
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                DesignField::known(
                    "Design niche indices",
                    state
                        .niches
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                DesignField::result(
                    "Module mission exclusions",
                    state
                        .module_mission_exclusions
                        .iter()
                        .map(|entry| {
                            queries::spelling(frame, entry.module_token)
                                .map(|name| format!("{}: {}", text(name), entry.mask))
                        })
                        .collect::<Result<Vec<_>, _>>()
                        .map(|entries| entries.join(", ")),
                ),
                DesignField::known("Role icon selection", state.role_icon_index),
                DesignField::known("Frame", state.is_frame != 0),
                DesignField::known("Auto-upgraded", state.auto_upgraded != 0),
                DesignField::known("Highlighted", state.highlight != 0),
                DesignField::known(
                    "Template naming group",
                    state
                        .division_names_group
                        .as_deref()
                        .map(text)
                        .unwrap_or_else(|| "None".into()),
                ),
                DesignField::known(
                    "Upgrade permissions",
                    format!(
                        "type {} / design {} / modules {}",
                        state.can_upgrade_type,
                        state.can_upgrade_variant,
                        state.can_upgrade_modules
                    ),
                ),
            ],
        )),
        Err(reason) => sections.push(section(
            "Ancestry and designer",
            vec![DesignField::refused("Stored design state", reason.clone())],
        )),
    }
    if let Ok(state) = &state {
        let mut attribution = Vec::new();
        let snapshots = frame
            .complete::<tables::VariantBonusSnapshots>()
            .map_err(|e| e.to_string());
        for (ordinal, id) in state
            .bonus_sources
            .iter()
            .chain(&state.named_sources)
            .enumerate()
        {
            let source = snapshots.as_ref().map_err(Clone::clone).and_then(|rows| {
                rows.binary_search_by_key(id, |row| row.id)
                    .map(|index| &rows[index].source)
                    .map_err(|_| format!("applied source snapshot {id} is not recorded"))
            });
            match source {
                Ok(source) => {
                    attribution.push(DesignField::known(
                        format!("Source {ordinal} company"),
                        source
                            .idea
                            .as_deref()
                            .map(text)
                            .unwrap_or_else(|| "None".into()),
                    ));
                    if let Some(named) = &source.named {
                        attribution.push(DesignField::known(
                            format!("Source {ordinal} name"),
                            text(&named.name),
                        ));
                        attribution.push(DesignField::known(
                            format!("Source {ordinal} description"),
                            text(&named.description),
                        ));
                        attribution.push(DesignField::known(
                            format!("Source {ordinal} owner and handle"),
                            format!("{} / {}", named.owner, named.handle),
                        ));
                    }
                    attribution.push(DesignField::known(
                        format!("Source {ordinal} terrain attrition"),
                        source
                            .terrain_attrition
                            .iter()
                            .map(|value| fixed(*value))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ));
                }
                Err(reason) => {
                    attribution.push(DesignField::refused(format!("Source {ordinal}"), reason))
                }
            }
        }
        sections.push(section("Applied source attribution", attribution));
    }
    let sheet = sheet(frame, id, content);
    match &sheet {
        Ok(sheet) => {
            sections.push(section(
                "Stat sheet",
                installed_content_core::equipment::stat_metadata()
                    .iter()
                    .enumerate()
                    .map(|(index, (name, _, _))| {
                        DesignField::known(*name, fixed(sheet.values[index]))
                    })
                    .collect(),
            ));
            sections.push(section(
                "Build and resources",
                vec![
                    DesignField::known("Build IC", fixed(sheet.build_cost_raw)),
                    DesignField::known("Manpower", sheet.manpower),
                    DesignField::result(
                        "Resources",
                        sheet
                            .build_resources
                            .iter()
                            .map(|entry| {
                                queries::resource_name(frame, entry.index)
                                    .map(|name| format!("{}: {}", text(name), fixed(entry.value)))
                            })
                            .collect::<Result<Vec<_>, _>>()
                            .map(|entries| entries.join(", ")),
                    ),
                    DesignField::known(
                        "Terrain attrition (native order)",
                        sheet
                            .terrain_attrition
                            .iter()
                            .map(|value| fixed(*value))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ],
            ));
            let mut fields = Vec::new();
            for source in &sheet.sources {
                for (kind, values) in [
                    ("add", &source.terms.add),
                    ("average", &source.terms.average),
                    ("factor", &source.terms.multiply),
                ] {
                    for term in values {
                        let name = installed_content_core::equipment::stat_metadata()[term.index].0;
                        fields.push(DesignField::known(
                            format!("{} / {name} / {kind}", source.name),
                            format!("{} × {}", fixed(term.value), fixed(source.scale)),
                        ));
                    }
                }
            }
            if let Some(reason) = &sheet.breakdown_refusal {
                fields.push(DesignField::refused(
                    "Cached rebuild reconciliation",
                    reason,
                ));
            }
            sections.push(section("Stat contributors", fields));
            let mut missions = Vec::new();
            for mission in &sheet.mission_stats {
                for stat in &mission.stats {
                    let name = installed_content_core::equipment::stat_metadata()
                        .get(stat.index as usize)
                        .map_or("Unknown stat", |entry| entry.0);
                    missions.push(DesignField::known(
                        format!("Mission mask {} / {name}", mission.mask),
                        fixed(stat.value),
                    ));
                }
                match queries::mission_sheet(frame, id, mission.mask, content) {
                    Ok(detail) => {
                        for source in &detail.contributors {
                            for (kind, terms) in [
                                ("add", &source.terms.add),
                                ("average", &source.terms.average),
                                ("factor", &source.terms.multiply),
                            ] {
                                for term in terms {
                                    missions.push(DesignField::known(
                                        format!(
                                            "Mission {} / {} / {} / {kind}",
                                            mission.mask,
                                            source.name,
                                            installed_content_core::equipment::stat_metadata()[term.index]
                                                .0
                                        ),
                                        format!("{} × {}", fixed(term.value), fixed(source.scale)),
                                    ));
                                }
                            }
                        }
                        missions.push(DesignField::result(
                            format!("Mission {} contributor reconciliation", mission.mask),
                            detail
                                .reconciliation
                                .map(|()| "Matches cached mission statistics".into()),
                        ));
                    }
                    Err(reason) => missions.push(DesignField::refused(
                        format!("Mission {} contributors", mission.mask),
                        reason,
                    )),
                }
            }
            if let Ok(modules) = frame.complete::<tables::VariantModules>() {
                for module in modules.iter().filter(|row| row.variant == id) {
                    let Ok(name) = queries::spelling(frame, module.definition_token) else {
                        continue;
                    };
                    if let Some(module) = content.module_design(name) {
                        for (limits, terms) in &module.mission_terms {
                            for (kind, terms) in [
                                ("add", &terms.add),
                                ("average", &terms.average),
                                ("factor", &terms.multiply),
                            ] {
                                for term in terms {
                                    missions.push(DesignField::known(
                                        format!(
                                            "{} / {} / {} / {kind}",
                                            text(name),
                                            limits
                                                .iter()
                                                .map(|name| text(name))
                                                .collect::<Vec<_>>()
                                                .join(", "),
                                            installed_content_core::equipment::stat_metadata()[term.index]
                                                .0
                                        ),
                                        fixed(term.value),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            sections.push(section("Mission statistics and contributors", missions));
        }
        Err(reason) => sections.push(section(
            "Stat sheet",
            vec![DesignField::refused("Equipment statistics", reason.clone())],
        )),
    }
    let mut costs = Vec::new();
    if let Some(source) = source {
        costs.push(DesignField::result(
            "Source-specific consumable XP discount",
            queries::template_discount(frame, definition.creator_link, source).map(fixed),
        ));
        let generic = defines
            .map_err(str::to_owned)
            .and_then(|defines| {
                installed_content_core::equipment::UpgradeXpRules::load(defines).map_err(|e| e.to_string())
            })
            .and_then(|rules| queries::generic_upgrade_xp(frame, id, source, content, &rules));
        costs.push(DesignField::result(
            "Generic upgrade XP before template and organisation costs",
            generic.map(fixed),
        ));
        costs.push(DesignField::result(
            "Build IC difference",
            sheet.as_ref().map_err(Clone::clone).and_then(|target| {
                super::sheet(frame, source, content)
                    .map(|source| fixed(target.build_cost_raw.wrapping_sub(source.build_cost_raw)))
            }),
        ));
        let rules = defines.map_err(str::to_owned).and_then(|defines| {
            installed_content_core::equipment::ModuleXpRules::load(defines).map_err(|e| e.to_string())
        });
        match frame.complete::<tables::VariantModules>() {
            Ok(modules) => {
                let mut slots: Vec<_> = modules
                    .iter()
                    .filter(|row| row.variant == id || row.variant == source)
                    .map(|row| row.slot)
                    .collect();
                slots.sort_unstable();
                slots.dedup();
                for slot in slots {
                    let next = modules
                        .iter()
                        .find(|row| row.variant == id && row.slot == slot)
                        .map(|row| row.definition_token);
                    let previous = modules
                        .iter()
                        .find(|row| row.variant == source && row.slot == slot)
                        .map(|row| (row.definition_token, row.definition_active != 0));
                    costs.push(DesignField::result(
                        format!("Slot {slot} XP before final discounts"),
                        rules
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|rules| {
                                module_change_xp(
                                    frame,
                                    definition.creator_link,
                                    next,
                                    previous,
                                    content,
                                    rules,
                                )
                            })
                            .map(fixed),
                    ));
                }
            }
            Err(reason) => costs.push(DesignField::refused("Module XP", reason.to_string())),
        }
        let organisation = |reference| {
            let cat = cat.ok_or("organisation cost content is unavailable".to_owned())?;
            let inputs = cat
                .content
                .layers
                .derived::<catalogue::CachedResult<super::costs::OrganisationContent>>()
                .get_or_init(|| super::costs::OrganisationContent::load(&cat.content.layers))
                .map_err(|e| e.to_string())?;
            let world = cat.scripted_content()?.recorded_world(frame)?;
            super::costs::organisation_change_xp(
                frame,
                reference,
                &inputs.traits,
                &inputs.definitions.policies,
                &world,
            )
        };
        costs.push(DesignField::result(
            "Generic design save XP",
            defines
                .map_err(str::to_owned)
                .and_then(|defines| {
                    super::costs::base_save_xp(frame, id, source, content, defines, organisation)
                })
                .map(fixed),
        ));
        costs.push(DesignField::result(
            "Module design save XP",
            defines
                .map_err(str::to_owned)
                .and_then(|defines| {
                    super::costs::module_save_xp(frame, id, source, content, defines, organisation)
                })
                .map(fixed),
        ));
        let compare = |organisation| {
            defines.map_err(str::to_owned).and_then(|defines| {
                super::industrial::conversion_cost(
                    frame,
                    id,
                    source,
                    organisation,
                    content,
                    defines,
                    |reference| {
                        let cat = cat.ok_or(
                            "organisation conversion content unavailable (count: 1)".to_owned(),
                        )?;
                        let inputs = cat
                            .content
                            .layers
                            .derived::<catalogue::CachedResult<super::costs::OrganisationContent>>()
                            .get_or_init(|| {
                                super::costs::OrganisationContent::load(&cat.content.layers)
                            })
                            .map_err(|e| e.to_string())?;
                        let world = cat.scripted_content()?.recorded_world(frame)?;
                        super::costs::organisation_conversion_bonus(
                            frame, reference, id, &inputs, content, &world,
                        )
                    },
                )
            })
        };
        let team = state
            .as_ref()
            .ok()
            .map(|state| state.design_team)
            .filter(|reference| *reference != 0);
        let comparison = compare(None);
        let organisation_cost = team.map(|team| compare(Some(team)));
        if let Some(organisation_cost) = organisation_cost {
            match organisation_cost {
                Ok(cost) => {
                    costs.push(DesignField::known(
                        "Refit and conversion IC with design organisation",
                        fixed(cost.total.ic),
                    ));
                    costs.push(DesignField::known(
                        "Organisation conversion and hull IC",
                        fixed(cost.conversion),
                    ));
                    for (token, value) in cost.total.resources {
                        match queries::resource_name(frame, token) {
                            Ok(name) => costs.push(DesignField::known(
                                format!("Organisation conversion {}", text(name)),
                                fixed(value),
                            )),
                            Err(reason) => costs.push(DesignField::refused(
                                "Organisation conversion resource",
                                reason,
                            )),
                        }
                    }
                }
                Err(reason) => costs.push(DesignField::refused(
                    "Organisation conversion IC/resources",
                    reason,
                )),
            }
        }
        match comparison {
            Ok(comparison) => {
                costs.extend([
                    DesignField::known("Industrial comparison IC", fixed(comparison.total.ic)),
                    DesignField::known("Conversion and hull IC", fixed(comparison.conversion)),
                    DesignField::known(
                        "Construction and upgrade IC",
                        fixed(comparison.construction),
                    ),
                    DesignField::known("Dismantling IC", fixed(comparison.dismantling)),
                    DesignField::known(
                        "Multiplier difference IC",
                        fixed(comparison.multiplier_difference),
                    ),
                    DesignField::known(
                        "Construction factor",
                        fixed(comparison.construction_factor),
                    ),
                    DesignField::known("Dismantling factor", fixed(comparison.dismantling_factor)),
                ]);
                for (token, value) in comparison.total.resources {
                    match queries::resource_name(frame, token) {
                        Ok(name) => costs.push(DesignField::known(
                            format!("Conversion {}", text(name)),
                            fixed(value),
                        )),
                        Err(reason) => {
                            costs.push(DesignField::refused("Conversion resource", reason))
                        }
                    }
                }
            }
            Err(reason) => costs.push(DesignField::refused(
                "Industrial comparison IC/resources",
                reason,
            )),
        }
    } else {
        costs.push(DesignField::known(
            "Source design",
            "Choose a source design for XP and industrial comparisons",
        ));
    }
    sections.push(section("Costs and XP", costs));
    let discounts = match queries::design_templates(frame, definition.creator_link) {
        Ok(entries) if entries.uses.is_empty() => {
            vec![DesignField::known("Discount records", "None")]
        }
        Ok(entries) => entries
            .entries()
            .enumerate()
            .flat_map(|(ordinal, (uses, factor, tokens))| {
                let equipment = tokens
                    .iter()
                    .map(|token| queries::spelling(frame, *token).map(text))
                    .collect::<Result<Vec<_>, _>>()
                    .map(|names| names.join(", "));
                [
                    DesignField::known(
                        format!("Discount {ordinal} remaining uses"),
                        uses.to_string(),
                    ),
                    DesignField::known(format!("Discount {ordinal} stored factor"), fixed(factor)),
                    DesignField::result(format!("Discount {ordinal} equipment"), equipment),
                ]
            })
            .collect(),
        Err(reason) => vec![DesignField::refused("Consumable design discounts", reason)],
    };
    sections.push(section("Consumable design discounts", discounts));
    let modifiers = match frame.complete::<tables::DesignCountryModifiers>() {
        Ok(values) => values
            .iter()
            .filter(|row| row.country == definition.creator_link)
            .map(|row| {
                let label = frame
                    .complete::<tables::ModifierDefinitions>()
                    .ok()
                    .and_then(|definitions| {
                        definitions
                            .iter()
                            .find(|definition| definition.index == row.modifier)
                    })
                    .and_then(|definition| queries::spelling(frame, definition.token).ok())
                    .map(text)
                    .unwrap_or_else(|| format!("Modifier {}", row.modifier));
                DesignField::result(
                    label,
                    row.value
                        .filter(|_| row.failure == 0)
                        .map(fixed)
                        .ok_or_else(|| {
                            format!(
                                "country {} modifier {} acquisition refused ({})",
                                row.country, row.modifier, row.failure
                            )
                        }),
                )
            })
            .collect(),
        Err(reason) => vec![DesignField::refused(
            "Country design modifiers",
            reason.to_string(),
        )],
    };
    sections.push(section("Country modifiers applied by designers", modifiers));
    let world = cat
        .ok_or("recorded trigger content is unavailable".to_owned())
        .and_then(|cat| cat.scripted_content()?.recorded_world(frame));
    let unlock_content = cat
        .ok_or("module unlock content is unavailable".to_owned())
        .and_then(|cat| {
            cat.content
                .layers
                .derived::<catalogue::CachedResult<unlocks::Content>>()
                .get_or_init(|| unlocks::Content::load(&cat.content.layers))
                .map_err(|e| e.to_string())
        });
    let validity = match frame.complete::<tables::VariantModules>() {
        Ok(modules) => modules
            .iter()
            .filter(|row| row.variant == id)
            .map(|row| {
                if row.definition_active == 0 {
                    return DesignField::known(
                        format!("Slot {} country admission", row.slot),
                        "No active module",
                    );
                }
                let name = queries::spelling(frame, row.definition_token);
                DesignField::result(
                    format!("Slot {} country admission", row.slot),
                    name.and_then(|name| {
                        world
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|world| {
                                queries::module_admission(
                                    world,
                                    definition.creator_link,
                                    name,
                                    content,
                                )
                            })
                            .map(|valid| valid.to_string())
                    }),
                )
            })
            .collect(),
        Err(reason) => vec![DesignField::refused("Modules", reason.to_string())],
    };
    sections.push(section("Validity and roles", validity));
    sections
        .last_mut()
        .unwrap()
        .fields
        .push(DesignField::result(
            "Slot dependencies and module counts",
            queries::structural_validity(frame, id, content).map(|result| {
                if result.failures.is_empty() {
                    "Valid".into()
                } else {
                    result.failures.join("; ")
                }
            }),
        ));
    if let Ok(kind) = queries::spelling(frame, definition.equipment_token).and_then(|name| {
        content
            .named(name)
            .ok_or("equipment slot content is unavailable".into())
    }) {
        for (name, slot) in kind.module_slots.iter().zip(&kind.module_slot_rules) {
            sections.last_mut().unwrap().fields.extend([
                DesignField::known(format!("Slot {} required", text(name)), slot.required),
                DesignField::known(
                    format!("Slot {} declared categories", text(name)),
                    slot.categories
                        .iter()
                        .map(|category| text(category))
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ]);
        }
    }
    if let Ok(modules) = frame.complete::<tables::VariantModules>() {
        for module in modules.iter().filter(|row| row.variant == id) {
            sections
                .last_mut()
                .unwrap()
                .fields
                .push(DesignField::result(
                    format!("Slot {} module unlock", module.slot),
                    (|| {
                        if module.definition_active == 0 {
                            return Ok("No active module".into());
                        }
                        let world = world.as_ref().map_err(Clone::clone)?;
                        let unlock_content = unlock_content.as_ref().map_err(Clone::clone)?;
                        let result = unlocks::module_unlock(
                            frame,
                            world,
                            definition.creator_link,
                            queries::spelling(frame, module.definition_token)?,
                            unlock_content,
                        )?;
                        Ok(if result.unlocked() {
                            let mut text = result.sources.join(", ");
                            for reason in result.refusals {
                                text.push_str(&format!("; unavailable contributor: {reason}"));
                            }
                            text
                        } else {
                            "Locked".into()
                        })
                    })(),
                ));
        }
    }
    sections
        .last_mut()
        .unwrap()
        .fields
        .extend([DesignField::result(
            "Permitted roles",
            queries::permitted_roles(frame, id, content).map(|roles| roles.join(", ")),
        )]);
    let mut unit = Vec::new();
    if let Some(ship) = ship {
        unit.push(DesignField::result(
            "Selected ship XP",
            queries::ship_context(frame, id, ship).map(|row| fixed(row.experience)),
        ));
        let effects = cat
            .ok_or_else(|| "unit effect content is unavailable".to_owned())
            .and_then(|cat| {
                let defines = defines.map_err(str::to_owned)?;
                super::unit_effects::ship(frame, id, ship, &cat.content.layers, defines)
            });
        unit_effect_fields(&mut unit, "Ship", effects);
    }
    if let Some(wing) = wing {
        match queries::wing_context(frame, id, wing) {
            Ok(context) => {
                unit.push(DesignField::known(
                    "Selected wing XP",
                    fixed(context.experience),
                ));
                unit.push(DesignField::known(
                    "Wing mission mask",
                    context.wing.mission_mask,
                ));
                let effects = cat
                    .ok_or_else(|| "unit effect content is unavailable".to_owned())
                    .and_then(|cat| {
                        let defines = defines.map_err(str::to_owned)?;
                        super::unit_effects::wing(frame, id, wing, &cat.content.layers, defines)
                    });
                unit_effect_fields(&mut unit, "Wing", effects);
                unit.push(DesignField::result("Wing replacement setting", context.wing.replacement_setting
                    .require("AirWings.replacement_setting").map_err(|e| e.to_string())
                    .and_then(|value| value.ok_or("wing replacement writer certificate or read unavailable (count: 1)".into()))
                    .map(|value| value.to_string())));
                if let Ok(equipment) = frame.complete::<tables::WingEquipment>() {
                    for row in equipment.iter().filter(|row| row.wing == wing) {
                        unit.push(DesignField::known(
                            format!(
                                "Wing equipment bank entry {} / design {}",
                                row.ordinal, row.variant
                            ),
                            fixed(row.amount_raw),
                        ));
                    }
                }
                unit.push(DesignField::known(
                    "Assigned ace",
                    context
                        .ace
                        .map(|ace| {
                            format!(
                                "{} {} {} / {}",
                                text(&ace.name),
                                text(&ace.surname),
                                text(&ace.callsign),
                                ace.id
                            )
                        })
                        .unwrap_or_else(|| "None".into()),
                ));
                if let Some(ace) = context.ace {
                    unit.push(DesignField::result(
                        "Ace definition",
                        ace.presentation
                            .require("AirAces.presentation")
                            .map_err(|e| e.to_string())
                            .and_then(|p| {
                                p.as_ref().ok_or("ace presentation capture refused".into())
                            })
                            .map(|p| {
                                p.definition
                                    .as_deref()
                                    .map(text)
                                    .unwrap_or_else(|| "None".into())
                            }),
                    ));
                }
            }
            Err(reason) => unit.push(DesignField::refused("Selected wing context", reason)),
        }
    }
    if ship.is_none() && wing.is_none() {
        unit.push(DesignField::known(
            "Unit context",
            "Choose a ship or wing reference to inspect its XP and ace context",
        ));
    }
    sections.push(section("Ship and wing context", unit));
    sections
}

fn unit_effect_fields(
    fields: &mut Vec<DesignField>,
    unit: &str,
    effects: Result<super::unit_effects::Effects, String>,
) {
    match effects {
        Ok(effects) => {
            if let Some(level) = effects.level {
                fields.push(DesignField::known(format!("{unit} XP level"), level));
            }
            for (name, value) in effects.sources {
                fields.push(DesignField::known(
                    format!("{unit} contributor / {name}"),
                    fixed(value),
                ));
            }
            for (name, value) in effects.values {
                fields.push(DesignField::known(
                    format!("{unit} XP effective modifier / {name}"),
                    fixed(value),
                ));
            }
            for reason in effects.refusals {
                fields.push(DesignField::refused(format!("{unit} contributor"), reason));
            }
        }
        Err(reason) => fields.push(DesignField::refused(
            format!("{unit} XP and ace effects"),
            reason,
        )),
    }
}

fn selected_tables() -> Vec<rows::TableId> {
    let mut selected = dashboard_text::triggers::RECORDED_TABLES.to_vec();
    selected.extend([
        rows::TableId::VariantDefinitions,
        rows::TableId::VariantModules,
        rows::TableId::VariantUpgrades,
        rows::TableId::VariantStats,
        rows::TableId::VariantBonusSnapshots,
        rows::TableId::VariantChanges,
        rows::TableId::DesignCountryModifiers,
        rows::TableId::DesignCountryTemplates,
        rows::TableId::DesignObserverContext,
        rows::TableId::DesignOrganisationLiveness,
        rows::TableId::LocalPlayer,
        rows::TableId::SimulationFeatures,
        rows::TableId::IndustrialOrganisations,
        rows::TableId::IndustrialOrganisationDetails,
        rows::TableId::IndustrialOrganisationPolicies,
        rows::TableId::AirWings,
        rows::TableId::WingEquipment,
        rows::TableId::WingCumulative,
        rows::TableId::AirAces,
        rows::TableId::Ships,
        rows::TableId::ShipModels,
        rows::TableId::CountryCommandPower,
        rows::TableId::CountryIntel,
        rows::TableId::ReservedCountryNaming,
        rows::TableId::ResourceDefinitions,
    ]);
    selected.sort_unstable();
    selected.dedup();
    selected
}

impl Product for EquipmentDesignDetail {
    const KEY: &'static str = "equipment-design-detail";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = DesignDetailParams;
    type Reply = DesignDetailReply;
    fn types() -> Vec<String> {
        vec![
            DesignDetailParams::decl(),
            DesignDetailReply::decl(),
            DesignChoice::decl(),
            DesignSection::decl(),
            DesignField::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid design branch")?;
        for value in [
            &params.variant,
            &params.from_variant,
            &params.ship,
            &params.wing,
        ] {
            handle(value)?;
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        Self::validate(params).map_err(crate::invalid)?;
        let cat = catalogue::catalogue(source, context.content()?)?;
        let content = super::super::units_of(&cat).map_err(crate::invalid)?;
        let selected = selected_tables();
        let frame = match source.replay_ref().seek_selected_with(
            params.branch_id.parse().unwrap(),
            params.hour,
            Some(&selected),
            || {
                if cancelled.load(Ordering::Acquire) {
                    Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "design request cancelled",
                    ))
                } else {
                    Ok(())
                }
            },
        )? {
            replay::campaign::SeekResult::Frame(frame) => frame,
            replay::campaign::SeekResult::Gap { reason, .. } => {
                return Err(crate::invalid(reason));
            }
        };
        let definitions = frame.complete::<tables::VariantDefinitions>()?;
        let designs = definitions
            .iter()
            .map(|row| DesignChoice {
                id: row.id.to_string(),
                name: if row.display_name.is_empty() {
                    format!(
                        "{} / {}",
                        queries::spelling(&frame, row.equipment_token)
                            .map(text)
                            .unwrap_or_else(|_| "Equipment".into()),
                        row.id
                    )
                } else {
                    text(&row.display_name)
                },
                creator: row.creator_link,
                obsolete: row.obsolete != 0,
            })
            .collect();
        let defines = cat
            .content
            .layers
            .derived::<catalogue::CachedResult<installed_content_core::defines::Defines>>()
            .get_or_init(|| installed_content_core::defines::read(&cat.content.layers))
            .map_err(|e| e.to_string());
        let mut sections = handle(&params.variant)
            .map_err(crate::invalid)?
            .map(|id| {
                inspect(
                    &frame,
                    id,
                    handle(&params.from_variant).unwrap(),
                    handle(&params.ship).unwrap(),
                    handle(&params.wing).unwrap(),
                    &content,
                    Some(&cat),
                    defines.as_deref().map_err(String::as_str),
                )
            })
            .unwrap_or_default();
        if let Some(id) = handle(&params.variant).map_err(crate::invalid)? {
            // An absent design must not suppress the roster or the Saved design refusal.
            let target_country = params.target_country.map(Ok).unwrap_or_else(|| {
                queries::definition(&frame, id).map(|definition| definition.creator_link)
            });
            let observer: Result<u32, String> = params.observer.map(Ok).unwrap_or_else(|| {
                let [row] = frame
                    .complete::<tables::LocalPlayer>()
                    .map_err(|e| e.to_string())?
                else {
                    return Err("LocalPlayer singleton is unavailable".into());
                };
                row.viewing_country()
                    .ok_or("LocalPlayer viewing country is unavailable; select an observer".into())
            });
            let fields = match observer {
                Ok(observer) => vec![
                    DesignField::known("Observer", observer),
                    DesignField::result(
                        "Target country",
                        target_country.clone().map(|target| target.to_string()),
                    ),
                    DesignField::result(
                        "Design details revealed",
                        target_country
                            .and_then(|target_country| {
                                let defines = defines.as_deref().map_err(Clone::clone)?;
                                queries::reveal(
                                    &frame,
                                    id,
                                    observer,
                                    target_country,
                                    params
                                        .fog
                                        .map(Ok)
                                        .unwrap_or_else(|| queries::recorded_fog(&frame)),
                                    false,
                                    defines,
                                )
                            })
                            .map(|shown| shown.to_string()),
                    ),
                    DesignField::known(
                        "Fog context",
                        params.fog.map_or("Recorded game switch", |fog| {
                            if fog {
                                "Query with fog enabled"
                            } else {
                                "Query with fog disabled"
                            }
                        }),
                    ),
                ],
                Err(reason) => vec![DesignField::refused("Observer", reason)],
            };
            sections.push(section("Observer reveal", fields));
        }
        let mut warnings = cat
            .verification
            .iter()
            .chain(&cat.warnings)
            .cloned()
            .collect::<Vec<_>>();
        if let Err(reason) = frame.complete::<tables::VariantChanges>() {
            warnings.push(format!(
                "Within-day design history is unavailable: {reason}"
            ));
        }
        let reply = DesignDetailReply {
            warnings,
            branch_id: params.branch_id.clone(),
            hour: params.hour,
            designs,
            sections,
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_design_tables_include_recorded_ace_owner_operands() {
        let selected = selected_tables();
        for table in [
            rows::TableId::CountryCommandPower,
            rows::TableId::AirAces,
            rows::TableId::WingCumulative,
            rows::TableId::AirWings,
            rows::TableId::DesignCountryModifiers,
            rows::TableId::ShipModels,
        ] {
            assert!(selected.contains(&table), "missing {table:?}");
        }
        assert!(selected.windows(2).all(|pair| pair[0] < pair[1]));
    }
    #[test]
    fn design_detail_preserves_full_references_and_exact_signed_fixed_point() {
        assert_eq!(handle(&Some(u64::MAX.to_string())).unwrap(), Some(u64::MAX));
        assert!(handle(&Some("9007199254740993.0".into())).is_err());
        assert_eq!(fixed(i64::MIN), "-92233720368547.75808");
        assert_eq!(fixed(-1), "-0.00001");
        let field = DesignField::refused("Refit", "missing owner discount");
        let value = serde_json::to_value(field).unwrap();
        assert!(value["value"].is_null());
        assert_eq!(value["refusal"], "missing owner discount");
    }
}
