//! XUI23 exact command-ability admission.
//!
//! Authored ability definitions establish identity, target type, localisation
//! keys and resolved icon.  The replay row remains authoritative for selected-
//! context availability, activation, timers, effective cost, ordering, tooltip
//! and disabled reason.  No trigger, timer or command-power formula is run here.

use crate::assets::AssetResolver;
use crate::gfx::SpriteRegistry;
use crate::paradox_text::{parse_file, Document, Value};
use crate::{GameUiError, Result as UiResult};
use replay_core::{CommandAbilityRow, Leaf, PoliticsDetailView};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct AbilityDefinition {
    pub ability_id: String,
    pub target_kind: String,
    pub name_key: String,
    pub description_key: String,
    pub icon: String,
    pub base_cost_fraction: f64,
    pub authored_duration_hours: i64,
    pub authored_cooldown_hours: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveAbilityPresentation {
    pub ability_id: String,
    pub available: bool,
    pub active: bool,
    pub activation_hour: i64,
    pub cooldown_hours: i64,
    pub cooldown_remaining_hours: i64,
    pub cost_command_power: f64,
    pub target_kind: String,
    pub target_id: String,
    pub duration_hours: i64,
    pub duration_remaining_hours: i64,
    pub display_order: i64,
    pub icon: String,
    pub tooltip: String,
    pub disabled_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAbilityRejection(pub String);

/// Parse the effective `common/abilities` namespace and resolve every icon
/// against the effective interface sprite registry. The resolver is the same
/// authenticated UI14/UI15 VFS used by XUI24; duplicate identities, parse
/// recoveries, unknown top-level constructs, missing required fields and
/// unresolved icons reject the whole catalog.
pub fn load_active_ability_definitions(
    resolver: &AssetResolver,
) -> UiResult<BTreeMap<String, AbilityDefinition>> {
    let ability_files = resolver.list_files_recursive("common/abilities", Some("txt"))?;
    let gfx_files = resolver.list_files_recursive("interface", Some("gfx"))?;
    let mut sprites = SpriteRegistry::new();
    for file in &gfx_files {
        sprites.parse_file(&file.path, &file.rel_path)?;
    }
    let mut documents = Vec::with_capacity(ability_files.len());
    for file in &ability_files {
        let document = parse_file(&file.path)?;
        if !document.recoveries.is_empty() {
            return Err(GameUiError::parse(format!(
                "active ability definition recoveries in {}: {:?}",
                file.rel_path, document.recoveries
            )));
        }
        documents.push((file.rel_path.clone(), document));
    }
    definitions_from_documents(&documents, &sprites)
}

fn definitions_from_documents(
    documents: &[(String, Document)],
    sprites: &SpriteRegistry,
) -> UiResult<BTreeMap<String, AbilityDefinition>> {
    let mut definitions = BTreeMap::new();
    for (source, document) in documents {
        for top in document.root.fields() {
            if !top
                .key
                .as_deref()
                .is_some_and(|key| key.eq_ignore_ascii_case("ability"))
            {
                return Err(GameUiError::parse(format!(
                    "unknown active ability top-level construct {:?} in {source}",
                    top.key
                )));
            }
            let block = top.value.as_object().ok_or_else(|| {
                GameUiError::parse(format!("ability root is not an object in {source}"))
            })?;
            for entry in block {
                let ability_id = entry.key.as_deref().ok_or_else(|| {
                    GameUiError::parse(format!("anonymous active ability in {source}"))
                })?;
                let body = entry.value.as_object().ok_or_else(|| {
                    GameUiError::parse(format!(
                        "active ability {ability_id} is not an object in {source}"
                    ))
                })?;
                let value = Value::Object(body.to_vec());
                let required = |key: &str| {
                    value.first_scalar_ci(key).ok_or_else(|| {
                        GameUiError::parse(format!(
                            "active ability {ability_id} lacks {key} in {source}"
                        ))
                    })
                };
                let name_key = required("name")?.to_string();
                let description_key = required("desc")?.to_string();
                let target_kind = required("type")?.to_string();
                let base_cost_fraction = required("cost")?.parse::<f64>().map_err(|_| {
                    GameUiError::parse(format!(
                        "active ability {ability_id} has invalid cost in {source}"
                    ))
                })?;
                let authored_duration_hours =
                    required("duration")?.parse::<i64>().map_err(|_| {
                        GameUiError::parse(format!(
                            "active ability {ability_id} has invalid duration in {source}"
                        ))
                    })?;
                let authored_cooldown_hours = value
                    .first_scalar_ci("cooldown")
                    .map(|raw| {
                        raw.parse::<i64>().map_err(|_| {
                            GameUiError::parse(format!(
                                "active ability {ability_id} has invalid cooldown in {source}"
                            ))
                        })
                    })
                    .transpose()?;
                if !base_cost_fraction.is_finite()
                    || base_cost_fraction < 0.0
                    || authored_duration_hours < -1
                    || authored_cooldown_hours.is_some_and(|hours| hours <= 0)
                {
                    return Err(GameUiError::parse(format!(
                        "active ability {ability_id} has invalid authored bounds in {source}"
                    )));
                }
                let icon = value
                    .first_scalar_ci("icon")
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("GFX_ability_{ability_id}"));
                if sprites.get(&icon).is_none() {
                    return Err(GameUiError::parse(format!(
                        "active ability {ability_id} icon {icon} is unresolved in {source}"
                    )));
                }
                let definition = AbilityDefinition {
                    ability_id: ability_id.to_string(),
                    target_kind,
                    name_key,
                    description_key,
                    icon,
                    base_cost_fraction,
                    authored_duration_hours,
                    authored_cooldown_hours,
                };
                if definitions
                    .insert(ability_id.to_string(), definition)
                    .is_some()
                {
                    return Err(GameUiError::parse(format!(
                        "duplicate effective active ability identity {ability_id}"
                    )));
                }
            }
        }
    }
    if definitions.is_empty() {
        return Err(GameUiError::parse(
            "effective active ability catalog is empty",
        ));
    }
    Ok(definitions)
}

fn required<T: Clone>(
    leaf: &Leaf<T>,
    field: &str,
) -> std::result::Result<T, ActiveAbilityRejection> {
    leaf.measured()
        .cloned()
        .ok_or_else(|| ActiveAbilityRejection(format!("{field} is not measured")))
}

fn admit_row(
    row: &CommandAbilityRow,
    index: usize,
    definitions: &BTreeMap<String, AbilityDefinition>,
) -> std::result::Result<ActiveAbilityPresentation, ActiveAbilityRejection> {
    let prefix = format!("active_abilities.rows[{index}]");
    let ability_id = required(&row.ability_id, &format!("{prefix}.ability_id"))?;
    let definition = definitions.get(&ability_id).ok_or_else(|| {
        ActiveAbilityRejection(format!("{prefix}.ability_id has no effective definition"))
    })?;
    let available = required(&row.available, &format!("{prefix}.available"))?;
    let active = required(&row.active, &format!("{prefix}.active"))?;
    let activation_hour = required(&row.activation_hour, &format!("{prefix}.activation_hour"))?;
    let cooldown_hours = required(&row.cooldown_hours, &format!("{prefix}.cooldown_hours"))?;
    let cooldown_remaining_hours = required(
        &row.cooldown_remaining_hours,
        &format!("{prefix}.cooldown_remaining_hours"),
    )?;
    let cost_command_power = required(
        &row.cost_command_power,
        &format!("{prefix}.cost_command_power"),
    )?;
    let target_kind = required(&row.target_kind, &format!("{prefix}.target_kind"))?;
    let target_id = required(&row.target_id, &format!("{prefix}.target_id"))?;
    let duration_hours = required(&row.duration_hours, &format!("{prefix}.duration_hours"))?;
    let duration_remaining_hours = required(
        &row.duration_remaining_hours,
        &format!("{prefix}.duration_remaining_hours"),
    )?;
    let display_order = required(&row.display_order, &format!("{prefix}.display_order"))?;
    let icon = required(&row.icon, &format!("{prefix}.icon"))?;
    let tooltip = required(&row.tooltip, &format!("{prefix}.tooltip"))?;
    let disabled_reason = required(&row.disabled_reason, &format!("{prefix}.disabled_reason"))?;

    if display_order != index as i64 {
        return Err(ActiveAbilityRejection(format!(
            "{prefix}.display_order is not the exact source ordinal"
        )));
    }
    if target_kind != definition.target_kind {
        return Err(ActiveAbilityRejection(format!(
            "{prefix}.target_kind disagrees with the effective definition"
        )));
    }
    if icon != definition.icon {
        return Err(ActiveAbilityRejection(format!(
            "{prefix}.icon disagrees with the effective definition"
        )));
    }
    if !cost_command_power.is_finite() || cost_command_power < 0.0 {
        return Err(ActiveAbilityRejection(format!(
            "{prefix}.cost_command_power is invalid"
        )));
    }
    if cooldown_hours < -1
        || cooldown_remaining_hours < -1
        || (cooldown_hours >= 0 && cooldown_remaining_hours > cooldown_hours)
    {
        return Err(ActiveAbilityRejection(format!(
            "{prefix} has an invalid cooldown interval"
        )));
    }
    if duration_hours < -1
        || duration_remaining_hours < -1
        || (duration_hours >= 0 && duration_remaining_hours > duration_hours)
        || (active && duration_hours >= 0 && duration_remaining_hours == 0)
    {
        return Err(ActiveAbilityRejection(format!(
            "{prefix} has an invalid duration interval"
        )));
    }
    if available && !disabled_reason.is_empty() {
        return Err(ActiveAbilityRejection(format!(
            "{prefix} is available but carries a disabled reason"
        )));
    }
    if !available && disabled_reason.is_empty() {
        return Err(ActiveAbilityRejection(format!(
            "{prefix} is disabled without an exact reason"
        )));
    }
    if tooltip.is_empty() || target_id.is_empty() {
        return Err(ActiveAbilityRejection(format!(
            "{prefix} lacks resolved tooltip or target identity"
        )));
    }

    Ok(ActiveAbilityPresentation {
        ability_id,
        available,
        active,
        activation_hour,
        cooldown_hours,
        cooldown_remaining_hours,
        cost_command_power,
        target_kind,
        target_id,
        duration_hours,
        duration_remaining_hours,
        display_order,
        icon,
        tooltip,
        disabled_reason,
    })
}

/// Admit the complete selected-context ability bar.  A measured-empty list with
/// `complete=true` is valid; every other absent/incomplete shape rejects.
pub fn admit_active_abilities(
    view: &PoliticsDetailView,
    definitions: &BTreeMap<String, AbilityDefinition>,
) -> std::result::Result<Vec<ActiveAbilityPresentation>, ActiveAbilityRejection> {
    if required(
        &view.command_abilities_complete,
        "active_abilities.complete",
    )? != true
    {
        return Err(ActiveAbilityRejection(
            "active_abilities.complete is not true".to_string(),
        ));
    }
    let rows = required(&view.command_ability_rows, "active_abilities.rows")?;
    let mut out = Vec::with_capacity(rows.len());
    let mut identities = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let admitted = admit_row(row, index, definitions)?;
        let identity = (
            admitted.ability_id.clone(),
            admitted.target_kind.clone(),
            admitted.target_id.clone(),
        );
        if !identities.insert(identity) {
            return Err(ActiveAbilityRejection(format!(
                "active_abilities.rows[{index}] duplicates ability/target identity"
            )));
        }
        out.push(admitted);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradox_text::parse_str;

    fn definition() -> AbilityDefinition {
        AbilityDefinition {
            ability_id: "force_attack".into(),
            target_kind: "army_leader".into(),
            name_key: "ABILITY_FORCE_ATTACK".into(),
            description_key: "ABILITY_FORCE_ATTACK_DESC".into(),
            icon: "GFX_ability_force_attack".into(),
            base_cost_fraction: 0.22,
            authored_duration_hours: 168,
            authored_cooldown_hours: None,
        }
    }

    fn row() -> CommandAbilityRow {
        CommandAbilityRow {
            ability_id: Leaf::Measured("force_attack".into()),
            available: Leaf::Measured(false),
            active: Leaf::Measured(true),
            activation_hour: Leaf::Measured(17),
            cooldown_hours: Leaf::Measured(504),
            cooldown_remaining_hours: Leaf::Measured(336),
            cost_command_power: Leaf::Measured(22.0),
            target_kind: Leaf::Measured("army_leader".into()),
            target_id: Leaf::Measured("73".into()),
            duration_hours: Leaf::Measured(168),
            duration_remaining_hours: Leaf::Measured(120),
            display_order: Leaf::Measured(0),
            icon: Leaf::Measured("GFX_ability_force_attack".into()),
            tooltip: Leaf::Measured("Force Attack".into()),
            disabled_reason: Leaf::Measured("Already active".into()),
        }
    }

    #[test]
    fn exact_row_is_admitted_without_reconstruction() {
        let defs = BTreeMap::from([("force_attack".into(), definition())]);
        let admitted = admit_row(&row(), 0, &defs).expect("exact row");
        assert_eq!(admitted.cooldown_remaining_hours, 336);
        assert_eq!(admitted.duration_remaining_hours, 120);
        assert_eq!(admitted.tooltip, "Force Attack");
        assert_eq!(admitted.disabled_reason, "Already active");
    }

    #[test]
    fn effective_definition_catalog_parses_and_resolves_icons() {
        let ability_doc = parse_str(
            r#"ability = {
                force_attack = {
                    name = ABILITY_FORCE_ATTACK
                    desc = ABILITY_FORCE_ATTACK_DESC
                    type = army_leader
                    cost = 0.22
                    duration = 168
                }
            }"#,
        );
        let gfx_doc = parse_str(
            r#"spriteTypes = {
                spriteType = {
                    name = GFX_ability_force_attack
                    texturefile = "gfx/interface/abilitylist/ability_force_attack.dds"
                }
            }"#,
        );
        let mut sprites = SpriteRegistry::new();
        sprites.add_document("interface/abilitylist.gfx", &gfx_doc);
        let catalog = definitions_from_documents(
            &[("common/abilities/generic.txt".into(), ability_doc)],
            &sprites,
        )
        .expect("effective ability catalog");
        let row = catalog.get("force_attack").expect("force attack");
        assert_eq!(row.target_kind, "army_leader");
        assert_eq!(row.icon, "GFX_ability_force_attack");
        assert_eq!(row.base_cost_fraction, 0.22);
        assert_eq!(row.authored_duration_hours, 168);
    }

    #[test]
    fn duplicate_or_unresolved_effective_definitions_reject() {
        let ability = || {
            parse_str(
                r#"ability = { x = {
                    name = X desc = X_DESC type = army_leader cost = 0.1 duration = 24
                } }"#,
            )
        };
        let sprites = SpriteRegistry::new();
        assert!(definitions_from_documents(&[("a.txt".into(), ability())], &sprites).is_err());

        let gfx_doc = parse_str(
            r#"spriteTypes = { spriteType = {
                name = GFX_ability_x texturefile = "gfx/x.dds"
            } }"#,
        );
        let mut resolved = SpriteRegistry::new();
        resolved.add_document("interface/x.gfx", &gfx_doc);
        assert!(definitions_from_documents(
            &[("a.txt".into(), ability()), ("b.txt".into(), ability())],
            &resolved,
        )
        .is_err());
    }

    #[test]
    fn missing_or_inconsistent_runtime_values_reject() {
        let defs = BTreeMap::from([("force_attack".into(), definition())]);
        let mut missing = row();
        missing.tooltip = Leaf::Absent;
        assert!(admit_row(&missing, 0, &defs).is_err());

        let mut bad_order = row();
        bad_order.display_order = Leaf::Measured(1);
        assert!(admit_row(&bad_order, 0, &defs).is_err());

        let mut bad_timer = row();
        bad_timer.cooldown_remaining_hours = Leaf::Measured(505);
        assert!(admit_row(&bad_timer, 0, &defs).is_err());

        let mut inferred_reason = row();
        inferred_reason.disabled_reason = Leaf::Measured(String::new());
        assert!(admit_row(&inferred_reason, 0, &defs).is_err());
    }
}
