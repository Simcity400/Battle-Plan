
use game_ui::decision_chrome::{DecisionAvailability, DecisionSet};
use game_ui::power_balance::{
    admit_power_balance_screen, AdmittedPowerBalanceScreen, BarState, ChangeCadence,
    PowerBalanceAdmissionRequest, PowerBalanceDecisionInput, PowerBalanceDecisionTextRole,
    PowerBalanceDefinitionCatalog, PowerBalanceRuntimeProvenance, PowerBalanceRuntimeSupplement,
    PowerBalanceRuntimeTextBinding, PowerBalanceRuntimeTextProvenance, PowerBalanceRuntimeTextSlot,
    PowerBalanceTrendInput, ResolvedPowerBalanceText,
};
use replay_core::{
    Leaf, PowerBalanceBarStateExact, PowerBalanceChangeCadenceExact,
    PowerBalanceDecisionAvailabilityExact, PowerBalancePresentationExactView, RuntimeTextCatalog,
    ViewStatus, WorldFrame,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedA25Text {
    text: String,
    provenance: PowerBalanceRuntimeTextProvenance,
}

pub fn power_balance_runtime_supplements(
    frame: &WorldFrame,
    country_tag: &str,
) -> Result<Vec<PowerBalanceRuntimeSupplement>, String> {
    let exact = frame.power_balance_presentation_exact();
    let presentation = match (exact.status, exact.presentation.as_ref()) {
        (ViewStatus::Present, Some(presentation)) => presentation,
        (ViewStatus::DomainAbsent, _) => {
            return Err("XUI18 exact d74 presentation is absent".into())
        }
        _ => return Err("XUI18 exact d74 presentation is unavailable".into()),
    };
    match (
        frame.power_balance_state_exact().status,
        frame.power_balance_state_exact().state.as_ref(),
    ) {
        (ViewStatus::Present, Some(state))
            if state.frame_hour == presentation.frame_hour
                && state.semantic_receipt_sha256 == presentation.state_semantic_receipt_sha256 => {}
        (ViewStatus::DomainAbsent, _) => return Err("XUI18 exact d74 state is absent".into()),
        _ => return Err("XUI18 exact d74 state/presentation join is unavailable".into()),
    }
    if !presentation
        .balances
        .iter()
        .any(|balance| balance.country_tag == country_tag)
    {
        return Ok(Vec::new());
    }
    let runtime_text = match frame
        .runtime_text()
        .map_err(|error| format!("XUI18 A25 admission failed: {error}"))?
    {
        Leaf::Measured(catalog) if catalog.contexts_complete => catalog,
        Leaf::Measured(_) => return Err("XUI18 A25 context set is incomplete".into()),
        Leaf::Unobtained => return Err("XUI18 A25 context set is unavailable".into()),
        Leaf::Absent => return Err("XUI18 A25 context set is absent".into()),
    };
    let expected_hour = i64::try_from(presentation.frame_hour)
        .map_err(|_| "XUI18 frame hour exceeds replay range".to_string())?;
    build_runtime_supplements(exact, country_tag, |context_id| {
        resolve_runtime_text(&runtime_text, context_id, expected_hour, country_tag)
    })
}

pub fn admit_country_power_balance_ui(
    frame: &WorldFrame,
    country_tag: &str,
    catalog: &PowerBalanceDefinitionCatalog,
    decisions: &DecisionSet,
    expected_content_manifest_sha256: &str,
) -> Result<AdmittedPowerBalanceScreen, String> {
    require_effective_content_manifest(frame, expected_content_manifest_sha256)?;
    let runtime = power_balance_runtime_supplements(frame, country_tag)?;
    admit_power_balance_screen(PowerBalanceAdmissionRequest {
        country_tag,
        view: frame.power_balance_state_exact(),
        catalog,
        decisions,
        runtime: &runtime,
    })
    .map_err(|error| format!("XUI18 admission rejected: {error:?}"))
}

fn require_effective_content_manifest(
    frame: &WorldFrame,
    expected_content_manifest_sha256: &str,
) -> Result<(), String> {
    if expected_content_manifest_sha256.len() != 64 {
        return Err("XUI18 renderer content manifest identity is invalid".into());
    }
    let state = frame
        .power_balance_state_exact()
        .state
        .as_ref()
        .ok_or_else(|| "XUI18 exact d74 state is unavailable for content join".to_string())?;
    let presentation = frame
        .power_balance_presentation_exact()
        .presentation
        .as_ref()
        .ok_or_else(|| {
            "XUI18 exact d74 presentation is unavailable for content join".to_string()
        })?;
    if state.effective_content_manifest_sha256 != expected_content_manifest_sha256
        || presentation.effective_content_manifest_sha256 != expected_content_manifest_sha256
    {
        return Err(
            "XUI18 state/presentation content manifest differs from the renderer's admitted VFS"
                .into(),
        );
    }
    Ok(())
}

fn resolve_runtime_text(
    catalog: &RuntimeTextCatalog,
    context_id: &str,
    expected_hour: i64,
    country_tag: &str,
) -> Result<ResolvedA25Text, String> {
    let record = catalog
        .contexts
        .get(context_id)
        .ok_or_else(|| format!("XUI18 missing A25 context {context_id}"))?;
    if record.context_id != context_id
        || record.frame_hour != expected_hour
        || (record.pov_country_or_global != country_tag && record.pov_country_or_global != "global")
    {
        return Err(format!(
            "XUI18 A25 context {context_id} hour/POV/provenance mismatch"
        ));
    }
    Ok(ResolvedA25Text {
        text: record.final_text_utf8.clone(),
        provenance: PowerBalanceRuntimeTextProvenance {
            context_id: record.context_id.clone(),
            frame_hour: record.frame_hour,
            pov_country_or_global: record.pov_country_or_global.clone(),
            view_id: record.view_id.clone(),
            element_id: record.element_id.clone(),
            setter_entry_rva: record.producer.setter_entry_rva,
            setter_caller_rva: record.producer.setter_caller_rva,
            formatter_entry_rva: record.producer.formatter_entry_rva,
            identity_source: record.producer.identity_source,
            final_text_sha256: record.final_text_sha256.clone(),
        },
    })
}

fn resolve_joined_text<F>(
    resolve_text: &mut F,
    contexts: &mut BTreeMap<String, PowerBalanceRuntimeTextProvenance>,
    bindings: &mut Vec<PowerBalanceRuntimeTextBinding>,
    slot: PowerBalanceRuntimeTextSlot,
    context_id: &str,
) -> Result<String, String>
where
    F: FnMut(&str) -> Result<ResolvedA25Text, String>,
{
    let resolved = resolve_text(context_id)?;
    if resolved.provenance.context_id != context_id {
        return Err(format!(
            "XUI18 A25 resolver returned {} for requested {context_id}",
            resolved.provenance.context_id
        ));
    }
    if let Some(existing) = contexts.get(context_id) {
        if existing != &resolved.provenance {
            return Err(format!(
                "XUI18 A25 context {context_id} changed provenance within one balance"
            ));
        }
    } else {
        contexts.insert(context_id.to_string(), resolved.provenance);
    }
    bindings.push(PowerBalanceRuntimeTextBinding {
        slot,
        context_id: context_id.to_string(),
    });
    Ok(resolved.text)
}

fn resolve_optional_text<F>(
    resolve_text: &mut F,
    contexts: &mut BTreeMap<String, PowerBalanceRuntimeTextProvenance>,
    bindings: &mut Vec<PowerBalanceRuntimeTextBinding>,
    slot: PowerBalanceRuntimeTextSlot,
    context_id: Option<&str>,
) -> Result<Option<String>, String>
where
    F: FnMut(&str) -> Result<ResolvedA25Text, String>,
{
    match context_id {
        Some(context_id) => {
            resolve_joined_text(resolve_text, contexts, bindings, slot, context_id).map(Some)
        }
        None => Ok(None),
    }
}

fn build_runtime_supplements<F>(
    exact: &PowerBalancePresentationExactView,
    country_tag: &str,
    mut resolve_text: F,
) -> Result<Vec<PowerBalanceRuntimeSupplement>, String>
where
    F: FnMut(&str) -> Result<ResolvedA25Text, String>,
{
    let presentation = match (exact.status, exact.presentation.as_ref()) {
        (ViewStatus::Present, Some(presentation)) => presentation,
        _ => return Err("XUI18 exact typed presentation is not Present".into()),
    };
    let presentation_frame_hour = i64::try_from(presentation.frame_hour)
        .map_err(|_| "XUI18 typed presentation frame hour exceeds replay range".to_string())?;
    let mut rows = Vec::new();
    for balance in presentation
        .balances
        .iter()
        .filter(|balance| balance.country_tag == country_tag)
    {
        if !rows.is_empty() {
            return Err(format!(
                "XUI18 {country_tag} has more than one controller-selected row"
            ));
        }
        let mut runtime_text_contexts = BTreeMap::new();
        let mut runtime_text_bindings = Vec::new();
        let trend = if let Some(trend) = balance.trend.as_ref() {
            Some(PowerBalanceTrendInput {
                cadence: match trend.cadence {
                    PowerBalanceChangeCadenceExact::Daily => ChangeCadence::Daily,
                    PowerBalanceChangeCadenceExact::Weekly => ChangeCadence::Weekly,
                },
                magnitude: trend.magnitude,
                resolved_text: resolve_joined_text(
                    &mut resolve_text,
                    &mut runtime_text_contexts,
                    &mut runtime_text_bindings,
                    PowerBalanceRuntimeTextSlot::Trend,
                    &trend.resolved_text_ref,
                )?,
            })
        } else {
            None
        };
        let mut decisions = Vec::with_capacity(balance.decisions.len());
        for decision in &balance.decisions {
            decisions.push(PowerBalanceDecisionInput {
                decision_id: decision.decision_id.clone(),
                source_record_id: decision.source_record_id.clone(),
                availability: match decision.availability {
                    PowerBalanceDecisionAvailabilityExact::Available => {
                        DecisionAvailability::Available
                    }
                    PowerBalanceDecisionAvailabilityExact::Unavailable => {
                        DecisionAvailability::Unavailable
                    }
                    PowerBalanceDecisionAvailabilityExact::Active => DecisionAvailability::Active,
                    PowerBalanceDecisionAvailabilityExact::OnCooldown => {
                        DecisionAvailability::OnCooldown
                    }
                },
                resolved_name: resolve_joined_text(
                    &mut resolve_text,
                    &mut runtime_text_contexts,
                    &mut runtime_text_bindings,
                    PowerBalanceRuntimeTextSlot::Decision {
                        decision_id: decision.decision_id.clone(),
                        source_record_id: decision.source_record_id.clone(),
                        role: PowerBalanceDecisionTextRole::Name,
                    },
                    &decision.resolved_name_text_ref,
                )?,
                cost_text: resolve_optional_text(
                    &mut resolve_text,
                    &mut runtime_text_contexts,
                    &mut runtime_text_bindings,
                    PowerBalanceRuntimeTextSlot::Decision {
                        decision_id: decision.decision_id.clone(),
                        source_record_id: decision.source_record_id.clone(),
                        role: PowerBalanceDecisionTextRole::Cost,
                    },
                    decision.cost_text_ref.as_deref(),
                )?,
                timer_text: resolve_optional_text(
                    &mut resolve_text,
                    &mut runtime_text_contexts,
                    &mut runtime_text_bindings,
                    PowerBalanceRuntimeTextSlot::Decision {
                        decision_id: decision.decision_id.clone(),
                        source_record_id: decision.source_record_id.clone(),
                        role: PowerBalanceDecisionTextRole::Timer,
                    },
                    decision.timer_text_ref.as_deref(),
                )?,
                target_tag: decision.target_tag_ref.clone(),
                target_flag_sprite: decision.target_flag_sprite.clone(),
            });
        }
        let launcher_percentage_text = resolve_optional_text(
            &mut resolve_text,
            &mut runtime_text_contexts,
            &mut runtime_text_bindings,
            PowerBalanceRuntimeTextSlot::LauncherPercentage,
            balance.launcher_percentage_text_ref.as_deref(),
        )?;
        let launcher_levels_text = resolve_optional_text(
            &mut resolve_text,
            &mut runtime_text_contexts,
            &mut runtime_text_bindings,
            PowerBalanceRuntimeTextSlot::LauncherLevels,
            balance.launcher_levels_text_ref.as_deref(),
        )?;
        let power_balance_modifier_desc = resolve_joined_text(
            &mut resolve_text,
            &mut runtime_text_contexts,
            &mut runtime_text_bindings,
            PowerBalanceRuntimeTextSlot::ModifierDescription,
            &balance.power_balance_modifier_desc_text_ref,
        )?;
        let resolved_text = ResolvedPowerBalanceText {
            balance_name: resolve_joined_text(
                &mut resolve_text,
                &mut runtime_text_contexts,
                &mut runtime_text_bindings,
                PowerBalanceRuntimeTextSlot::BalanceName,
                &balance.resolved_text_refs.balance_name,
            )?,
            left_side_name: resolve_joined_text(
                &mut resolve_text,
                &mut runtime_text_contexts,
                &mut runtime_text_bindings,
                PowerBalanceRuntimeTextSlot::LeftSideName,
                &balance.resolved_text_refs.left_side_name,
            )?,
            right_side_name: resolve_joined_text(
                &mut resolve_text,
                &mut runtime_text_contexts,
                &mut runtime_text_bindings,
                PowerBalanceRuntimeTextSlot::RightSideName,
                &balance.resolved_text_refs.right_side_name,
            )?,
            active_range_name: resolve_joined_text(
                &mut resolve_text,
                &mut runtime_text_contexts,
                &mut runtime_text_bindings,
                PowerBalanceRuntimeTextSlot::ActiveRangeName,
                &balance.resolved_text_refs.active_range_name,
            )?,
            balance_tooltip: resolve_joined_text(
                &mut resolve_text,
                &mut runtime_text_contexts,
                &mut runtime_text_bindings,
                PowerBalanceRuntimeTextSlot::BalanceTooltip,
                &balance.resolved_text_refs.balance_tooltip,
            )?,
            active_range_tooltip: resolve_joined_text(
                &mut resolve_text,
                &mut runtime_text_contexts,
                &mut runtime_text_bindings,
                PowerBalanceRuntimeTextSlot::ActiveRangeTooltip,
                &balance.resolved_text_refs.active_range_tooltip,
            )?,
        };
        let provenance = PowerBalanceRuntimeProvenance {
            producer_manifest_sha256: presentation.producer_manifest_sha256.clone(),
            completeness_record_identity: presentation.completeness_record_identity.clone(),
            effective_content_manifest_sha256: presentation
                .effective_content_manifest_sha256
                .clone(),
            state_semantic_receipt_sha256: presentation.state_semantic_receipt_sha256.clone(),
            presentation_semantic_receipt_sha256: presentation.semantic_receipt_sha256.clone(),
            frame_hour: presentation_frame_hour,
            expected_country_count: presentation.expected_country_count,
            expected_balance_count: presentation.expected_balance_count,
            runtime_text_contexts: runtime_text_contexts.into_values().collect(),
            runtime_text_bindings,
        };
        rows.push(PowerBalanceRuntimeSupplement {
            provenance,
            source_record_id: balance.source_record_id.clone(),
            country_tag: balance.country_tag.clone(),
            balance_id: balance.balance_id.clone(),
            manager_ordinal: balance.manager_ordinal,
            display_ordinal: balance.display_ordinal as usize,
            active_side_id: balance.active_side_id.clone(),
            trending_side_id: balance.trending_side_id.clone(),
            active_range_id: balance.active_range_id.clone(),
            effective_left_icon: balance.effective_left_icon.clone(),
            effective_right_icon: balance.effective_right_icon.clone(),
            launcher_visible: balance.launcher_visible,
            launcher_percentage_text,
            launcher_levels_text,
            launcher_icon_sprite: balance.launcher_icon_sprite.clone(),
            launcher_glow_visible: balance.launcher_glow_visible,
            bar_state: match balance.bar_state {
                PowerBalanceBarStateExact::StableLeft => BarState::StableLeft,
                PowerBalanceBarStateExact::StableRight => BarState::StableRight,
                PowerBalanceBarStateExact::MovingLeft => BarState::MovingLeft,
                PowerBalanceBarStateExact::MovingRight => BarState::MovingRight,
            },
            range_indicator_frame: balance.range_indicator_frame as usize,
            range_indicator_top_left: balance.range_indicator_top_left,
            trend,
            power_balance_modifier_desc,
            decisions_complete: balance.decisions_complete,
            decisions,
            resolved_text,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{
        PowerBalanceDecisionExact, PowerBalancePresentationExact, PowerBalancePresentationRowExact,
        PowerBalanceTrendExact, RawFrame, ReplaySource, ResolvedPowerBalanceTextRefsExact,
        RuntimeTextIdentitySource, SourceError, WorldTimeline,
    };
    use serde_json::{json, Map, Value};
    use sha2::{Digest, Sha256};

    #[derive(Clone)]
    struct OneFrameSource {
        frame: RawFrame,
        hours: [i64; 1],
    }

    impl ReplaySource for OneFrameSource {
        fn game_id(&self) -> &str {
            "a37-integration"
        }

        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
            if hour_index != self.hours[0] {
                return Err(SourceError::Open(format!(
                    "no A37 integration frame for hour {hour_index}"
                )));
            }
            Ok(self.frame.clone())
        }
    }

    fn runtime_context(context_id: &str, frame_hour: i64, pov: &str) -> Value {
        let text = format!("resolved:{context_id}");
        let bytes = text.as_bytes();
        let bytes_hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let sha256 = format!("{:x}", Sha256::digest(bytes));
        json!({
            "context_id": context_id,
            "frame_hour": frame_hour,
            "pov_country_or_global": pov,
            "view_id": "power_balance_view",
            "element_id": format!("element:{context_id}"),
            "source": {"kind": "setter_caller_rva", "rva": 0x01c4_baa0u32},
            "final_text": {
                "bytes_hex": bytes_hex,
                "utf8": text,
                "sha256": sha256,
            },
            "variables": {},
            "data_functions": {},
            "dynamic_colors": {},
            "layout": {"mode": "authored_reproducible"},
            "producer": {
                "setter_entry_rva": replay_core::A729_TEXT_SETTER_RVA,
                "setter_caller_rva": 0x01c4_baa0u32,
                "formatter_entry_rva": replay_core::A729_FORMATTER_RVA,
                "identity_source": "proven_setter_caller"
            }
        })
    }

    fn exact_companions() -> (Value, Value) {
        let effective = "11".repeat(32);
        let mut roster_bytes = b"D18R1\0".to_vec();
        roster_bytes.extend_from_slice(&1u32.to_le_bytes());
        roster_bytes.extend_from_slice(&17u32.to_le_bytes());
        roster_bytes.extend_from_slice(&3u16.to_le_bytes());
        roster_bytes.extend_from_slice(b"ITA");
        let roster = format!("{:x}", Sha256::digest(roster_bytes));
        let put_string = |hash: &mut Sha256, value: &str| {
            hash.update((value.len() as u32).to_le_bytes());
            hash.update(value.as_bytes());
        };
        let mut state_hash = Sha256::new();
        state_hash.update(b"D74S1\0");
        put_string(
            &mut state_hash,
            "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        );
        put_string(
            &mut state_hash,
            replay_core::A729_POWER_BALANCE_STATE_MANIFEST_SHA256,
        );
        state_hash.update(900u64.to_le_bytes());
        put_string(&mut state_hash, &effective);
        state_hash.update(18u16.to_le_bytes());
        state_hash.update(1u32.to_le_bytes());
        put_string(&mut state_hash, &roster);
        for value in [1u32, 1, 1, 1, 1, 0] {
            state_hash.update(value.to_le_bytes());
        }
        for value in [
            "ITA_power_balance",
            "ITA_grand_council_side",
            "ITA_mussolini_side",
            "ITA_mussolini_side",
            "ITA_mussolini_medium_control_range",
            effective.as_str(),
        ] {
            put_string(&mut state_hash, value);
        }
        state_hash.update(35_000i64.to_le_bytes());
        state_hash.update(1u32.to_le_bytes());
        state_hash.update(17u32.to_le_bytes());
        state_hash.update(1u32.to_le_bytes());
        state_hash.update(17u32.to_le_bytes());
        state_hash.update(1_000i64.to_le_bytes());
        state_hash.update(0i64.to_le_bytes());
        state_hash.update(0u32.to_le_bytes());
        let state_receipt = format!("{:x}", state_hash.finalize());
        let state = json!({
            "schema_version": 1,
            "authority": "exact_a729_all_country_power_balance_manager",
            "elapsed_wall_ms": 5,
            "frame_hour": 900,
            "static_authority_manifest_sha256": replay_core::A729_POWER_BALANCE_STATE_MANIFEST_SHA256,
            "effective_content_manifest_sha256": effective,
            "semantic_receipt_sha256": state_receipt,
            "d18_country_roster": {"frame_hour":900,"domain_id":18,"expected_country_count":1,"roster_identity_sha256":roster},
            "completeness_record_identity": format!("d74-state:{state_receipt}"),
            "expected_country_count": 1,
            "expected_record_count": 1,
            "expected_country_balance_pair_count": 1,
            "countries": [{"tag_db_id":17,"country_tag":"ITA"}],
            "records": [{
                "source_record_id":"d74:900:0:ITA_power_balance","manager_ordinal":0,
                "balance_id":"ITA_power_balance","left_side_id":"ITA_grand_council_side",
                "right_side_id":"ITA_mussolini_side","trending_side_id":"ITA_mussolini_side",
                "active_range_id":"ITA_mussolini_medium_control_range","effective_catalog_sha256":effective,
                "value_raw_fixed5":35000,"daily_sum_raw_fixed5":1000,"weekly_sum_raw_fixed5":0,
                "weekly_equivalent_raw_fixed5":7000,"change_cadence":"daily","member_tag_db_ids":[17],
                "member_accumulators":[{"tag_db_id":17,"daily_raw_fixed5":1000,"weekly_raw_fixed5":0}],
                "side_gfx_overrides":[]
            }]
        });
        let mut presentation = json!({
            "schema_version":4,"authority":"exact_a729_all_country_power_balance_presentation",
            "elapsed_wall_ms":7,"frame_hour":900,
            "producer_manifest_sha256":replay_core::A729_POWER_BALANCE_ALL_COUNTRY_PRESENTATION_MANIFEST_SHA256,
            "effective_content_manifest_sha256":effective,"state_semantic_receipt_sha256":state_receipt,
            "semantic_receipt_sha256":"","completeness_record_identity":"",
            "expected_country_count":1,"expected_balance_count":1,
            "balances":[{
                "source_record_id":"d74p:900:ITA:ITA_power_balance","country_tag":"ITA","balance_id":"ITA_power_balance",
                "manager_ordinal":0,"display_ordinal":0,"active_side_id":"ITA_mussolini_side",
                "trending_side_id":"ITA_mussolini_side","active_range_id":"ITA_mussolini_medium_control_range",
                "effective_left_icon":"GFX_bop_ITA_grand_council_side","effective_right_icon":"GFX_bop_ITA_mussolini_side",
                "launcher_visible":true,"launcher_percentage_text_ref":"percentage","launcher_levels_text_ref":"levels",
                "launcher_icon_sprite":"GFX_bop_ITA_mussolini_side","launcher_glow_visible":false,
                "bar_state":"moving_right","range_indicator_frame":1,"range_indicator_top_left":[198,101],
                "trend":{"cadence":"daily","magnitude":0.01,"resolved_text_ref":"trend"},
                "power_balance_modifier_desc_text_ref":"modifier_desc","decisions_complete":true,"decisions":[],
                "resolved_text_refs":{"balance_name":"name","left_side_name":"left","right_side_name":"right",
                    "active_range_name":"range","balance_tooltip":"tooltip","active_range_tooltip":"range_tooltip"}
            }]
        });
        let mut receipt_payload = presentation.clone();
        receipt_payload
            .as_object_mut()
            .unwrap()
            .remove("semantic_receipt_sha256");
        receipt_payload
            .as_object_mut()
            .unwrap()
            .remove("completeness_record_identity");
        let mut receipt_bytes = b"D74P4\0".to_vec();
        receipt_bytes.extend_from_slice(&serde_json::to_vec(&receipt_payload).unwrap());
        let receipt = format!("{:x}", Sha256::digest(receipt_bytes));
        presentation["semantic_receipt_sha256"] = Value::String(receipt.clone());
        presentation["completeness_record_identity"] =
            Value::String(format!("d74-presentation:{receipt}"));
        (state, presentation)
    }

    fn integration_frame(
        runtime_hour: i64,
        runtime_pov: &str,
        contexts_complete: bool,
        omit_context: Option<&str>,
    ) -> std::sync::Arc<WorldFrame> {
        let context_ids = [
            "percentage",
            "levels",
            "trend",
            "modifier_desc",
            "name",
            "left",
            "right",
            "range",
            "tooltip",
            "range_tooltip",
        ];
        let contexts = context_ids
            .into_iter()
            .filter(|context_id| omit_context != Some(*context_id))
            .map(|context_id| runtime_context(context_id, runtime_hour, runtime_pov))
            .collect::<Vec<_>>();
        let (state, presentation) = exact_companions();
        let root = json!({
            "countries": {"ITA":{"_r2_tag_db_id":17}},
            "_r2_power_balance_state_live": state,
            "_r2_power_balance_presentation_live": presentation,
            "runtime_text": {
                "schema": replay_core::RUNTIME_TEXT_SCHEMA,
                "state": "measured",
                "contexts_complete": contexts_complete,
                "contexts": contexts
            }
        });
        let mut rest: Map<String, Value> = root.as_object().unwrap().clone();
        if replay_core::COUNTRY_COMPOSED_MEMBERS
            .iter()
            .any(|member| rest.contains_key(*member))
        {
            rest.entry("tag_db_id_index")
                .or_insert_with(|| serde_json::json!({"country_composition_version": 2}));
        }
        let source = OneFrameSource {
            frame: RawFrame {
                hour_index: 900,
                date: None,
                save_version: None,
                schema_version: None,
                rest,
            },
            hours: [900],
        };
        WorldTimeline::open(Box::new(source))
            .unwrap()
            .frame(900)
            .unwrap()
    }

    fn resolved(context_id: &str) -> ResolvedA25Text {
        let text = format!("resolved:{context_id}");
        ResolvedA25Text {
            text: text.clone(),
            provenance: PowerBalanceRuntimeTextProvenance {
                context_id: context_id.into(),
                frame_hour: 900,
                pov_country_or_global: "ITA".into(),
                view_id: "power_balance_view".into(),
                element_id: format!("element:{context_id}"),
                setter_entry_rva: replay_core::A729_TEXT_SETTER_RVA,
                setter_caller_rva: Some(0x01c4_baa0),
                formatter_entry_rva: Some(replay_core::A729_FORMATTER_RVA),
                identity_source: RuntimeTextIdentitySource::ProvenSetterCaller,
                final_text_sha256: format!("{:x}", Sha256::digest(text.as_bytes())),
            },
        }
    }

    fn exact_view() -> PowerBalancePresentationExactView {
        PowerBalancePresentationExactView {
            status: ViewStatus::Present,
            presentation: Some(PowerBalancePresentationExact {
                schema_version: 4,
                authority: "exact_a729_all_country_power_balance_presentation".into(),
                elapsed_wall_ms: 7,
                frame_hour: 900,
                producer_manifest_sha256:
                    replay_core::A729_POWER_BALANCE_ALL_COUNTRY_PRESENTATION_MANIFEST_SHA256.into(),
                effective_content_manifest_sha256: "11".repeat(32),
                state_semantic_receipt_sha256: "22".repeat(32),
                semantic_receipt_sha256: "33".repeat(32),
                completeness_record_identity: format!("d74-presentation:{}", "33".repeat(32)),
                expected_country_count: 1,
                expected_balance_count: 1,
                balances: vec![PowerBalancePresentationRowExact {
                    source_record_id: "d74p:900:ITA:ITA_power_balance".into(),
                    country_tag: "ITA".into(),
                    balance_id: "ITA_power_balance".into(),
                    manager_ordinal: 0,
                    display_ordinal: 0,
                    active_side_id: "ITA_mussolini_side".into(),
                    trending_side_id: "ITA_mussolini_side".into(),
                    active_range_id: "ITA_mussolini_medium_control_range".into(),
                    effective_left_icon: "GFX_left".into(),
                    effective_right_icon: "GFX_right".into(),
                    launcher_visible: true,
                    launcher_percentage_text_ref: Some("percentage".into()),
                    launcher_levels_text_ref: Some("levels".into()),
                    launcher_icon_sprite: Some("GFX_right".into()),
                    launcher_glow_visible: false,
                    bar_state: PowerBalanceBarStateExact::MovingLeft,
                    range_indicator_frame: 1,
                    range_indicator_top_left: [198, 101],
                    trend: Some(PowerBalanceTrendExact {
                        cadence: PowerBalanceChangeCadenceExact::Daily,
                        magnitude: 0.01,
                        resolved_text_ref: "trend".into(),
                    }),
                    power_balance_modifier_desc_text_ref: "modifier_desc".into(),
                    decisions_complete: true,
                    decisions: vec![PowerBalanceDecisionExact {
                        decision_id: "ITA_bop_decision".into(),
                        source_record_id: "controller:900:ITA:decision:0".into(),
                        availability: PowerBalanceDecisionAvailabilityExact::Available,
                        resolved_name_text_ref: "decision_name".into(),
                        cost_text_ref: Some("decision_cost".into()),
                        timer_text_ref: None,
                        target_tag_ref: Some("SOV".into()),
                        target_flag_sprite: Some("GFX_flag_SOV".into()),
                    }],
                    resolved_text_refs: ResolvedPowerBalanceTextRefsExact {
                        balance_name: "name".into(),
                        left_side_name: "left".into(),
                        right_side_name: "right".into(),
                        active_range_name: "range".into(),
                        balance_tooltip: "tooltip".into(),
                        active_range_tooltip: "range_tooltip".into(),
                    },
                }],
            }),
        }
    }

    #[test]
    fn typed_d74_and_a25_refs_construct_runtime_without_losing_provenance() {
        let rows = build_runtime_supplements(&exact_view(), "ITA", |context| Ok(resolved(context)))
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].provenance.producer_manifest_sha256,
            replay_core::A729_POWER_BALANCE_ALL_COUNTRY_PRESENTATION_MANIFEST_SHA256
        );
        assert_eq!(
            rows[0].provenance.completeness_record_identity,
            format!("d74-presentation:{}", "33".repeat(32))
        );
        assert!(rows[0]
            .provenance
            .runtime_text_contexts
            .iter()
            .any(|context| context.context_id == "modifier_desc"
                && context.frame_hour == 900
                && context.pov_country_or_global == "ITA"
                && context.identity_source == RuntimeTextIdentitySource::ProvenSetterCaller));
        assert_eq!(rows[0].source_record_id, "d74p:900:ITA:ITA_power_balance");
        assert_eq!(rows[0].active_side_id, "ITA_mussolini_side");
        assert_eq!(rows[0].trending_side_id, "ITA_mussolini_side");
        assert_eq!(rows[0].bar_state, BarState::MovingLeft);
        assert_eq!(rows[0].range_indicator_frame, 1);
        assert_eq!(
            rows[0].power_balance_modifier_desc,
            "resolved:modifier_desc"
        );
        assert_eq!(
            rows[0].decisions[0].source_record_id,
            "controller:900:ITA:decision:0"
        );
        assert_eq!(rows[0].decisions[0].target_tag.as_deref(), Some("SOV"));
        assert_eq!(
            rows[0].decisions[0].target_flag_sprite.as_deref(),
            Some("GFX_flag_SOV")
        );
    }

    #[test]
    fn unrelated_country_is_measured_empty_without_resolving_text() {
        let rows = build_runtime_supplements(
            &exact_view(),
            "GER",
            |_| -> Result<ResolvedA25Text, String> {
                panic!("unrelated country must not resolve A25 text")
            },
        )
        .unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn mismatched_a25_context_identity_is_rejected() {
        let error =
            build_runtime_supplements(&exact_view(), "ITA", |_| Ok(resolved("wrong"))).unwrap_err();
        assert!(error.contains("resolver returned wrong for requested trend"));
    }

    #[test]
    fn world_frame_all_country_companion_admits_without_selected_present() {
        let frame = integration_frame(900, "ITA", true, None);
        assert_eq!(
            frame.power_balance_presentation_exact_status(),
            ViewStatus::Present
        );
        let rows = power_balance_runtime_supplements(&frame, "ITA").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].manager_ordinal, 0);
    }

    #[test]
    fn renderer_manifest_join_rejects_a_self_consistent_foreign_modset() {
        let frame = integration_frame(900, "ITA", true, None);
        require_effective_content_manifest(&frame, &"11".repeat(32))
            .expect("matching admitted renderer manifest");
        let error = require_effective_content_manifest(&frame, &"22".repeat(32)).unwrap_err();
        assert!(error.contains("differs from the renderer's admitted VFS"));
    }

    #[test]
    fn world_frame_a25_join_rejects_missing_cross_hour_cross_country_and_incomplete_text() {
        let missing = integration_frame(900, "ITA", true, Some("trend"));
        assert!(power_balance_runtime_supplements(&missing, "ITA")
            .unwrap_err()
            .contains("missing A25 context trend"));

        let wrong_hour = integration_frame(901, "ITA", true, None);
        assert!(power_balance_runtime_supplements(&wrong_hour, "ITA")
            .unwrap_err()
            .contains("hour/POV/provenance mismatch"));

        let wrong_pov = integration_frame(900, "GER", true, None);
        assert!(power_balance_runtime_supplements(&wrong_pov, "ITA")
            .unwrap_err()
            .contains("hour/POV/provenance mismatch"));

        let incomplete = integration_frame(900, "ITA", false, None);
        assert!(power_balance_runtime_supplements(&incomplete, "ITA")
            .unwrap_err()
            .contains("context set is incomplete"));
    }
}
