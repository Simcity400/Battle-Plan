//! XUI23 exact command-ability admission.
//!
//! Authored ability definitions establish identity, target type, localisation
//! keys and resolved normal/active icons. The replay row remains authoritative for selected-
//! context availability, activation, timers, effective cost, ordering, tooltip
//! and disabled reason.  No trigger, timer or command-power formula is run here.

use crate::assets::AssetResolver;
use crate::gfx::SpriteRegistry;
use crate::paradox_text::{parse_file, Document, Value};
use crate::{GameUiError, Result as UiResult};
use replay_core::{
    CommandAbilityEnvelope, CommandAbilityRow, CommandAbilityTimer, CommandAbilityTimerKind, Leaf,
    PoliticsDetailView,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct AbilityDefinition {
    pub ability_id: String,
    pub target_kind: String,
    pub name_key: String,
    pub description_key: String,
    pub icon: String,
    pub active_icon: String,
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
    pub cooldown_total: CommandAbilityTimer,
    pub cooldown_remaining: CommandAbilityTimer,
    pub cost_command_power_fixed: i64,
    pub cost_command_power_scale: i64,
    pub target_kind: String,
    pub target_id: String,
    pub duration_total: CommandAbilityTimer,
    pub duration_remaining: CommandAbilityTimer,
    pub display_order: u32,
    pub icon: String,
    pub tooltip: String,
    pub disabled_reason: String,
    pub affected_division_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAbilityRejection(pub String);

/// Parse the effective `common/abilities` namespace and resolve every icon
/// against the effective interface sprite registry supplied by the caller
/// (the same authenticated one the runtime builds, e.g. replay_app
/// ui_runtime). The resolver is the same authenticated UI14/UI15 VFS used by
/// XUI24; duplicate identities, parse recoveries, unknown top-level
/// constructs, missing required fields and unresolved icons reject the whole
/// catalog.
pub fn load_active_ability_definitions(
    resolver: &AssetResolver,
    sprites: &SpriteRegistry,
) -> UiResult<BTreeMap<String, AbilityDefinition>> {
    let ability_files = resolver.list_files_recursive("common/abilities", Some("txt"))?;
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
    definitions_from_documents(&documents, sprites)
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
                // 0x140FF44E0 creates the default CAbility+0x2A8 key and
                // 0x140FF5B20 recomputes it after an authored ordinary-icon
                // override. The final law is exactly ordinary icon plus this
                // suffix; 0x141D072F0 selects it for active/cancelable rows.
                let active_icon = format!("{icon}_animated");
                if sprites.get(&active_icon).is_none() {
                    return Err(GameUiError::parse(format!(
                        "active ability {ability_id} animated icon {active_icon} is unresolved in {source}"
                    )));
                }
                let definition = AbilityDefinition {
                    ability_id: ability_id.to_string(),
                    target_kind,
                    name_key,
                    description_key,
                    icon,
                    active_icon,
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

fn timer_hours(
    timer: &CommandAbilityTimer,
    field: &str,
) -> std::result::Result<Option<u32>, ActiveAbilityRejection> {
    match (timer.kind, timer.hours) {
        (CommandAbilityTimerKind::NotStarted | CommandAbilityTimerKind::Indefinite, None) => {
            Ok(None)
        }
        (CommandAbilityTimerKind::Hours, Some(hours)) => Ok(Some(hours)),
        _ => Err(ActiveAbilityRejection(format!(
            "{field} has a kind/value mismatch"
        ))),
    }
}

fn admit_row(
    row: &CommandAbilityRow,
    index: usize,
    definitions: &BTreeMap<String, AbilityDefinition>,
) -> std::result::Result<ActiveAbilityPresentation, ActiveAbilityRejection> {
    let prefix = format!("active_abilities.rows[{index}]");
    let ability_id = row.ability_id.clone();
    let definition = definitions.get(&ability_id).ok_or_else(|| {
        ActiveAbilityRejection(format!("{prefix}.ability_id has no effective definition"))
    })?;
    let available = row.available;
    let active = row.active;
    let activation_hour = row.activation_hour;
    let cooldown_total = row.cooldown_total.clone();
    let cooldown_remaining = row.cooldown_remaining.clone();
    let cost_command_power_fixed = row.cost_command_power_fixed;
    let cost_command_power_scale = row.cost_command_power_scale;
    let target_kind = row.target_kind.clone();
    let target_id = row.target_id.clone();
    let duration_total = row.duration_total.clone();
    let duration_remaining = row.duration_remaining.clone();
    let display_order = row.display_order;
    let icon = row.icon.clone();
    let tooltip = row.tooltip.clone();
    let disabled_reason = row.disabled_reason.clone();
    let affected_division_ids = row.affected_division_ids.clone();

    if display_order != index as u32 {
        return Err(ActiveAbilityRejection(format!(
            "{prefix}.display_order is not the exact source ordinal"
        )));
    }
    if target_kind != definition.target_kind {
        return Err(ActiveAbilityRejection(format!(
            "{prefix}.target_kind disagrees with the effective definition"
        )));
    }
    let expected_icon = if active {
        &definition.active_icon
    } else {
        &definition.icon
    };
    if &icon != expected_icon {
        return Err(ActiveAbilityRejection(format!(
            "{prefix}.icon disagrees with the exact active/ordinary definition icon"
        )));
    }
    if cost_command_power_fixed < 0 || cost_command_power_scale <= 0 {
        return Err(ActiveAbilityRejection(format!(
            "{prefix}.cost_command_power fixed-point value is invalid"
        )));
    }
    let cooldown_total_hours = timer_hours(&cooldown_total, &format!("{prefix}.cooldown_total"))?;
    let cooldown_remaining_hours =
        timer_hours(&cooldown_remaining, &format!("{prefix}.cooldown_remaining"))?;
    if matches!(
        (cooldown_total_hours, cooldown_remaining_hours),
        (Some(total), Some(remaining)) if remaining > total
    ) {
        return Err(ActiveAbilityRejection(format!(
            "{prefix} has an invalid cooldown interval"
        )));
    }
    let duration_total_hours = timer_hours(&duration_total, &format!("{prefix}.duration_total"))?;
    let duration_remaining_hours =
        timer_hours(&duration_remaining, &format!("{prefix}.duration_remaining"))?;
    if matches!(
        (duration_total_hours, duration_remaining_hours),
        (Some(total), Some(remaining)) if remaining > total
    ) || (active && duration_remaining_hours == Some(0))
        || (active && duration_remaining.kind == CommandAbilityTimerKind::NotStarted)
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
    let mut affected_identities = BTreeSet::new();
    if affected_division_ids
        .iter()
        .any(|identity| identity.is_empty() || !affected_identities.insert(identity.clone()))
    {
        return Err(ActiveAbilityRejection(format!(
            "{prefix}.affected_division_ids is empty or duplicated"
        )));
    }

    Ok(ActiveAbilityPresentation {
        ability_id,
        available,
        active,
        activation_hour,
        cooldown_total,
        cooldown_remaining,
        cost_command_power_fixed,
        cost_command_power_scale,
        target_kind,
        target_id,
        duration_total,
        duration_remaining,
        display_order,
        icon,
        tooltip,
        disabled_reason,
        affected_division_ids,
    })
}

fn admit_envelope(
    envelope: &CommandAbilityEnvelope,
    definitions: &BTreeMap<String, AbilityDefinition>,
) -> std::result::Result<Vec<ActiveAbilityPresentation>, ActiveAbilityRejection> {
    if !envelope.validates_exact_authority() {
        return Err(ActiveAbilityRejection(
            "active_abilities envelope does not carry exact ABA2/ABL1 authority".to_string(),
        ));
    }
    if !envelope.snapshot.complete {
        return Err(ActiveAbilityRejection(
            "active_abilities.snapshot.complete is not true".to_string(),
        ));
    }

    let rows = &envelope.snapshot.rows;
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

/// Admit the complete selected-context ability bar. A measured, exact-authority
/// ABA2/ABL1 envelope carrying an empty complete snapshot is valid; every
/// absent, unauthenticated, malformed or incomplete shape rejects.
pub fn admit_active_abilities(
    view: &PoliticsDetailView,
    definitions: &BTreeMap<String, AbilityDefinition>,
) -> std::result::Result<Vec<ActiveAbilityPresentation>, ActiveAbilityRejection> {
    let envelope = required(
        &view.command_ability_envelope,
        "active_abilities authenticated envelope",
    )?;
    admit_envelope(&envelope, definitions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradox_text::parse_str;
    const CANONICAL_ENVELOPE: &str = include_str!(
        "../../../re_evidence/rendering_laws/XUI23/canonical_active_abilities_envelope.json"
    );
    const CANONICAL_ENVELOPE_SHA256: &str = include_str!(
        "../../../re_evidence/rendering_laws/XUI23/canonical_active_abilities_envelope.sha256"
    );

    fn definition() -> AbilityDefinition {
        AbilityDefinition {
            ability_id: "force_attack".into(),
            target_kind: "army_leader".into(),
            name_key: "ABILITY_FORCE_ATTACK".into(),
            description_key: "ABILITY_FORCE_ATTACK_DESC".into(),
            icon: "GFX_ability_force_attack".into(),
            active_icon: "GFX_ability_force_attack_animated".into(),
            base_cost_fraction: 0.22,
            authored_duration_hours: 168,
            authored_cooldown_hours: None,
        }
    }

    fn row() -> CommandAbilityRow {
        CommandAbilityRow {
            ability_id: "force_attack".into(),
            available: false,
            active: true,
            activation_hour: 17,
            cooldown_total: CommandAbilityTimer {
                kind: CommandAbilityTimerKind::Hours,
                hours: Some(504),
            },
            cooldown_remaining: CommandAbilityTimer {
                kind: CommandAbilityTimerKind::Hours,
                hours: Some(336),
            },
            cost_command_power_fixed: 2_200_000,
            cost_command_power_scale: 100_000,
            target_kind: "army_leader".into(),
            target_id: "73".into(),
            duration_total: CommandAbilityTimer {
                kind: CommandAbilityTimerKind::Hours,
                hours: Some(168),
            },
            duration_remaining: CommandAbilityTimer {
                kind: CommandAbilityTimerKind::Hours,
                hours: Some(120),
            },
            display_order: 0,
            icon: "GFX_ability_force_attack_animated".into(),
            tooltip: "Force Attack".into(),
            disabled_reason: "Already active".into(),
            affected_division_ids: vec!["division:9".into()],
        }
    }

    fn envelope() -> CommandAbilityEnvelope {
        serde_json::from_str(CANONICAL_ENVELOPE).expect("canonical d88 envelope")
    }

    #[test]
    fn exact_row_is_admitted_without_reconstruction() {
        let defs = BTreeMap::from([("force_attack".into(), definition())]);
        let admitted = admit_row(&row(), 0, &defs).expect("exact row");
        assert_eq!(admitted.cooldown_remaining.hours, Some(336));
        assert_eq!(admitted.duration_remaining.hours, Some(120));
        assert_eq!(admitted.cost_command_power_fixed, 2_200_000);
        assert_eq!(admitted.cost_command_power_scale, 100_000);
        assert_eq!(admitted.tooltip, "Force Attack");
        assert_eq!(admitted.disabled_reason, "Already active");
    }

    #[test]
    fn exact_envelope_is_required_before_rows_are_admitted() {
        let defs = BTreeMap::from([("force_attack".into(), definition())]);
        let exact = envelope();
        assert_eq!(
            snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(
                CANONICAL_ENVELOPE.as_bytes()
            )),
            CANONICAL_ENVELOPE_SHA256
                .split_ascii_whitespace()
                .next()
                .expect("canonical d88 sha256")
        );
        let admitted = admit_envelope(&exact, &defs).expect("exact envelope");
        assert_eq!(admitted.len(), 1);

        let mut wrong_executable = envelope();
        wrong_executable.carrier.executable_sha256 = "0".repeat(64);
        assert!(admit_envelope(&wrong_executable, &defs).is_err());

        let mut wrong_source = envelope();
        wrong_source.snapshot.source_record_id = "a729:a36:16959360:GER:1:leader:99".into();
        assert!(admit_envelope(&wrong_source, &defs).is_err());

        let mut mismatched_frame = envelope();
        mismatched_frame.carrier.frame_hour = 16_959_361;
        assert!(admit_envelope(&mismatched_frame, &defs).is_err());
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
                frameAnimatedSpriteType = {
                    name = GFX_ability_force_attack_animated
                    texturefile = "gfx/interface/abilitylist/ability_force_attack_strip.dds"
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
        assert_eq!(row.active_icon, "GFX_ability_force_attack_animated");
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
    fn inconsistent_runtime_values_reject() {
        let defs = BTreeMap::from([("force_attack".into(), definition())]);
        let mut missing = row();
        missing.tooltip.clear();
        assert!(admit_row(&missing, 0, &defs).is_err());

        let mut duplicate_affected = row();
        duplicate_affected.affected_division_ids =
            vec!["division:9".into(), "division:9".into()];
        assert!(admit_row(&duplicate_affected, 0, &defs).is_err());

        let mut bad_order = row();
        bad_order.display_order = 1;
        assert!(admit_row(&bad_order, 0, &defs).is_err());

        let mut bad_timer = row();
        bad_timer.cooldown_remaining.hours = Some(505);
        assert!(admit_row(&bad_timer, 0, &defs).is_err());

        let mut collapsed_timer = row();
        collapsed_timer.cooldown_total = CommandAbilityTimer {
            kind: CommandAbilityTimerKind::Indefinite,
            hours: Some(0),
        };
        assert!(admit_row(&collapsed_timer, 0, &defs).is_err());

        let mut lossy_cost = row();
        lossy_cost.cost_command_power_scale = 0;
        assert!(admit_row(&lossy_cost, 0, &defs).is_err());

        let mut inferred_reason = row();
        inferred_reason.disabled_reason.clear();
        assert!(admit_row(&inferred_reason, 0, &defs).is_err());
    }
}
