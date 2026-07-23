//! Owner-runnable evidence acquisition for reserved A34/d87, A36/d88, and
//! A40/d85.
//!
//! These owners are deliberately absent from the production scheduler. They
//! install the exact sentinel-gated hooks and uninstall only after all detour
//! callbacks have quiesced. A34/A40 destructively drain pointer-free payloads
//! and frame them as ordinary H4CR domain data on request. A36 instead retains
//! authenticated raw transactions plus hashes of the pointer-free ABL1 frames
//! constructed from collector-owned same-frame roots; it never schedules d88.

use design_catalog_context_live_lane::selected_designer_identity as designer;
use design_catalog_context_live_lane::a40_peace_evidence as peace;
use decision_panel_live_lane as panel;
use active_command_abilities_live_lane::a729_hook as abilities;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceInstallReceipt {
    pub lane: &'static str,
    pub domain_id: u16,
    pub exact_executable_sha256: String,
    pub authority_receipt_sha256s: Vec<String>,
    pub installed_hook_count: u32,
    pub production_scheduling_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceDrainReceipt {
    pub lane: &'static str,
    pub domain_id: u16,
    pub frame_hour: u64,
    pub payload_len: u64,
    pub payload_sha256: String,
    pub source_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceUninstallReceipt {
    pub lane: &'static str,
    pub domain_id: u16,
    pub callbacks_observed: u64,
    pub callbacks_quiesced: bool,
    /// A40 borrows d78's independently owned designer-refresh observer. The
    /// evidence owner never tears that shared prerequisite down.
    pub shared_prerequisite_retained: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservedDomainEvidenceRecord {
    pub domain_id: u16,
    pub frame_hour: u64,
    pub payload: Vec<u8>,
    pub receipt: EvidenceDrainReceipt,
}

impl ReservedDomainEvidenceRecord {
    fn new(
        lane: &'static str,
        domain_id: u16,
        frame_hour: u64,
        source_sequence: u64,
        payload: Vec<u8>,
    ) -> Result<Self, String> {
        if source_sequence == 0 || payload.is_empty() {
            return Err("reserved-domain evidence lacks sequence or payload".into());
        }
        let payload_len = payload
            .len()
            .try_into()
            .map_err(|_| "reserved-domain evidence length exceeds u64")?;
        let payload_sha256 = format!("{:x}", Sha256::digest(&payload));
        Ok(Self {
            domain_id,
            frame_hour,
            payload,
            receipt: EvidenceDrainReceipt {
                lane,
                domain_id,
                frame_hour,
                payload_len,
                payload_sha256,
                source_sequence,
            },
        })
    }

    /// Frame this already-authenticated payload as an ordinary H4CR domain
    /// record. Sequence authority remains with the recorder/harness caller.
    pub fn encode_h4cr(&self, sequence: u64, game_hour_raw: i32) -> Result<Vec<u8>, String> {
        if sequence == 0 {
            return Err("reserved-domain H4CR sequence is zero".into());
        }
        let expected_hour = u64::try_from(game_hour_raw)
            .map_err(|_| "reserved-domain H4CR game hour is negative")?;
        if expected_hour != self.frame_hour {
            return Err("reserved-domain payload/H4CR hour mismatch".into());
        }
        if self.payload.len() > compact_contract::COMPACT_MAX_PAYLOAD_LEN as usize {
            return Err("reserved-domain payload exceeds H4CR bound".into());
        }
        compact_contract::validate_compact_record_semantics(
            self.domain_id,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &self.payload,
        )?;
        let mut out = Vec::with_capacity(
            compact_contract::COMPACT_RECORD_HEADER_LEN.saturating_add(self.payload.len()),
        );
        out.extend_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&self.domain_id.to_le_bytes());
        out.extend_from_slice(&compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA.to_le_bytes());
        out.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }
}

pub struct A40SelectedDesignerEvidenceOwner {
    install_receipt: EvidenceInstallReceipt,
    installed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A40PeaceEvidenceInstallReceipt {
    pub lane: &'static str,
    pub exact_executable_sha256: &'static str,
    pub static_producer_consumer_manifest_sha256: &'static str,
    pub hook_sentinel_artifact_sha256: &'static str,
    pub installed_hook_count: u32,
    pub production_carrier_enabled: bool,
}

/// Evidence-only owner for autonomous positive active-conference validation.
///
/// Unlike d85 this observer has no compact domain. Exact-a729 proves there is
/// no intrinsic action/allocation id and no terminal allocation-result join,
/// so raw process-local pointers and memory blocks may only leave the hook as a
/// diagnostic receipt. A production reducer must emit complete snapshots with
/// snapshot-scoped row ids and preserve terminal rows as explicitly unjoined.
pub struct A40PeaceEvidenceOwner {
    install_receipt: A40PeaceEvidenceInstallReceipt,
    installed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A36ActiveAbilityInstallReceipt {
    pub lane: &'static str,
    pub exact_executable_sha256: &'static str,
    pub ghidra_job_id: &'static str,
    pub ghidra_shard: &'static str,
    pub ghidra_manifest_sha256: &'static str,
    pub final_static_audit_job_id: &'static str,
    pub final_static_audit_shard: &'static str,
    pub final_static_audit_manifest_sha256: &'static str,
    pub final_roots_job_id: &'static str,
    pub final_roots_shard: &'static str,
    pub final_roots_manifest_sha256: &'static str,
    pub terminal_active_record_job_id: &'static str,
    pub terminal_active_record_shard: &'static str,
    pub terminal_active_record_manifest_sha256: &'static str,
    pub installed_hook_count: u32,
    pub installed_hook_rvas: [u32; abilities::HOOK_COUNT],
    pub production_activation_authorized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A36ActiveAbilityDrain {
    pub observations: Vec<abilities::ControllerObservation>,
    pub disabled_reasons: Vec<abilities::DisabledReasonObservation>,
    pub diagnostics: abilities::HookDiagnostics,
    pub callbacks_quiesced: bool,
    pub uninstalled_hook_count: u32,
    pub portable_joins: Vec<A36PortableJoinReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A36PortableJoinReceipt {
    pub controller_sequence: u64,
    pub catalog_epoch: u64,
    pub observation_game_hour_raw: i32,
    pub frame_hour: u64,
    pub elapsed_wall_ms: u64,
    pub tooltip_sequences: Vec<u64>,
    pub tooltip_game_hours_raw: Vec<i32>,
    pub selected_context_runtime: usize,
    pub country_tag: String,
    pub context_kind: active_command_abilities_live_lane::SelectedContextKind,
    pub stable_context_id: u64,
    pub division_ids_by_leader_runtime: Vec<(usize, Vec<u32>)>,
    pub abl1_len: u64,
    pub abl1_hex: String,
    pub abl1_sha256: String,
    pub row_count: u32,
    pub abl1_rows: Vec<A36PortableAbilityRowReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A36PortableTimerReceipt {
    pub kind: &'static str,
    pub hours: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A36PortableAbilityRowReceipt {
    pub ability_id: String,
    pub available: bool,
    pub active: bool,
    pub activation_hour: Option<u64>,
    pub cooldown_total: A36PortableTimerReceipt,
    pub cooldown_remaining: A36PortableTimerReceipt,
    pub duration_total: A36PortableTimerReceipt,
    pub duration_remaining: A36PortableTimerReceipt,
    pub cost_command_power_fixed: Option<i64>,
    pub display_order: u32,
    pub tooltip: Option<String>,
    pub disabled_reason: Option<String>,
    pub affected_division_ids: Vec<String>,
}

fn a36_timer_receipt(
    value: active_command_abilities_live_lane::TimerValue,
) -> A36PortableTimerReceipt {
    use active_command_abilities_live_lane::TimerValue;
    match value {
        TimerValue::NotStarted => A36PortableTimerReceipt { kind: "not_started", hours: None },
        TimerValue::Indefinite => A36PortableTimerReceipt { kind: "indefinite", hours: None },
        TimerValue::Hours(hours) => A36PortableTimerReceipt { kind: "hours", hours: Some(hours) },
    }
}

/// Evidence-only owner for A36 selected-context ability refresh and tooltip
/// transactions. The raw receipt deliberately retains process-local runtime
/// identities so the fixture regrader can prove producer joins. It hashes the
/// ABL1 built by the real pointer-erasing source constructor, but can never
/// publish ABL1/ABA2 or authorize compact scheduling.
pub struct A36ActiveAbilityEvidenceOwner {
    install_receipt: A36ActiveAbilityInstallReceipt,
    installed: bool,
    portable_joins: Vec<A36PortableJoinReceipt>,
}

impl A36ActiveAbilityEvidenceOwner {
    pub fn install() -> Result<Self, String> {
        Self::install_with_mode(false)
    }

    pub fn install_production() -> Result<Self, String> {
        Self::install_with_mode(true)
    }

    fn install_with_mode(production_activation_authorized: bool) -> Result<Self, String> {
        abilities::install_a729_ability_transition_hook()?;
        Ok(Self {
            install_receipt: A36ActiveAbilityInstallReceipt {
                lane: "A36_ACTIVE_ABILITY_DIAGNOSTIC",
                exact_executable_sha256: abilities::EXACT_A729_EXECUTABLE_SHA256,
                ghidra_job_id: abilities::EXACT_A729_AUTHORITY_JOB_ID,
                ghidra_shard: abilities::EXACT_A729_AUTHORITY_SHARD,
                ghidra_manifest_sha256: abilities::EXACT_A729_AUTHORITY_MANIFEST_SHA256,
                final_static_audit_job_id: abilities::FINAL_STATIC_AUDIT_JOB_ID,
                final_static_audit_shard: abilities::FINAL_STATIC_AUDIT_SHARD,
                final_static_audit_manifest_sha256:
                    abilities::FINAL_STATIC_AUDIT_MANIFEST_SHA256,
                final_roots_job_id: abilities::FINAL_ROOTS_JOB_ID,
                final_roots_shard: abilities::FINAL_ROOTS_SHARD,
                final_roots_manifest_sha256: abilities::FINAL_ROOTS_MANIFEST_SHA256,
                terminal_active_record_job_id: abilities::TERMINAL_ACTIVE_RECORD_JOB_ID,
                terminal_active_record_shard: abilities::TERMINAL_ACTIVE_RECORD_SHARD,
                terminal_active_record_manifest_sha256:
                    abilities::TERMINAL_ACTIVE_RECORD_MANIFEST_SHA256,
                installed_hook_count: abilities::HOOK_COUNT as u32,
                installed_hook_rvas: [
                    abilities::CONTEXT_INVALID_PROBE_RVA,
                    abilities::ACTIVE_COOLDOWN_PROBE_RVA,
                    abilities::EFFECT_SCOPE_CASTER_RESOLVER_RVA,
                    abilities::AVAILABILITY_EVALUATOR_RVA,
                    abilities::SPECIAL_CONTEXT_PROBE_RVA,
                    abilities::STRING_APPEND_RVA,
                    abilities::EFFECTIVE_COMMAND_POWER_COST_RVA,
                    abilities::ICON_RESOLVER_RVA,
                    abilities::TOOLTIP_BUILDER_RVA,
                    abilities::ABILITY_REFRESH_RVA,
                ],
                production_activation_authorized,
            },
            installed: true,
            portable_joins: Vec::new(),
        })
    }

    pub fn install_receipt(&self) -> &A36ActiveAbilityInstallReceipt {
        &self.install_receipt
    }

    pub fn retain_portable_join(
        &mut self,
        frame_hour: u64,
        elapsed_wall_ms: u64,
        catalog: &abilities::ExactA729PortableIdentityCatalog,
        controller: &abilities::ControllerObservation,
        tooltips: &[abilities::DisabledReasonObservation],
    ) -> Result<(), String> {
        if !self.installed {
            return Err("A36 portable join attempted after owner teardown".into());
        }
        if self
            .portable_joins
            .iter()
            .any(|receipt| receipt.controller_sequence == controller.sequence)
        {
            return Ok(());
        }
        let source = abilities::JoinedExactA729AbilitySource::bind_from_exact_root_catalog(
            frame_hour,
            elapsed_wall_ms,
            catalog,
            controller,
            tooltips,
        )?;
        let frame = active_command_abilities_live_lane::ExactA729AbilityControllerSource::collect_frame(
            &source,
            frame_hour,
        )?;
        let tooltip_sequences = source.tooltip_sequences().to_vec();
        let tooltip_game_hours_raw = tooltip_sequences
            .iter()
            .map(|sequence| {
                tooltips
                    .iter()
                    .find(|tooltip| tooltip.sequence == *sequence)
                    .map(|tooltip| tooltip.observed_game_hour_raw)
                    .ok_or_else(|| "A36 portable join lost its exact tooltip sequence".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if tooltip_game_hours_raw
            .iter()
            .any(|hour| *hour != controller.observed_game_hour_raw)
        {
            return Err("A36 portable join tooltip hour differs from retained refresh".into());
        }
        let abl1 = active_command_abilities_live_lane::encode_payload(&frame)
            .map_err(|error| format!("A36 evidence ABL1 encode failed: {error:?}"))?;
        let abl1_rows = frame.rows.iter().map(|row| A36PortableAbilityRowReceipt {
            ability_id: row.ability_id.clone(),
            available: row.available,
            active: row.active,
            activation_hour: row.activation_hour,
            cooldown_total: a36_timer_receipt(row.cooldown_total),
            cooldown_remaining: a36_timer_receipt(row.cooldown_remaining),
            duration_total: a36_timer_receipt(row.duration_total),
            duration_remaining: a36_timer_receipt(row.duration_remaining),
            cost_command_power_fixed: row.cost_command_power_fixed,
            display_order: row.display_order,
            tooltip: row.tooltip.clone(),
            disabled_reason: row.disabled_reason.clone(),
            affected_division_ids: row.affected_division_ids.clone(),
        }).collect::<Vec<_>>();
        let selected_context_runtime = [
            controller.selected_leader_runtime,
            controller.selected_army_group_runtime,
            controller.selected_army_runtime,
            controller.selected_other_runtime,
        ]
        .into_iter()
        .find(|runtime| *runtime != 0)
        .ok_or("A36 portable join has no selected runtime")?;
        let context = catalog
            .contexts_by_runtime
            .get(&selected_context_runtime)
            .ok_or("A36 portable join lost selected context identity")?;
        let mut divisions = catalog
            .division_ids_by_leader_runtime
            .iter()
            .map(|(runtime, ids)| (*runtime, ids.clone()))
            .collect::<Vec<_>>();
        divisions.sort_by_key(|(runtime, _)| *runtime);
        self.portable_joins.push(A36PortableJoinReceipt {
            controller_sequence: controller.sequence,
            catalog_epoch: catalog.epoch,
            observation_game_hour_raw: controller.observed_game_hour_raw,
            frame_hour,
            elapsed_wall_ms,
            tooltip_sequences,
            tooltip_game_hours_raw,
            selected_context_runtime,
            country_tag: context.country_tag.clone(),
            context_kind: context.kind,
            stable_context_id: context.stable_id,
            division_ids_by_leader_runtime: divisions,
            abl1_len: abl1.len().try_into().map_err(|_| "A36 ABL1 length exceeds u64")?,
            abl1_hex: abl1.iter().map(|byte| format!("{byte:02x}")).collect(),
            abl1_sha256: format!("{:x}", Sha256::digest(&abl1)),
            row_count: frame.rows.len().try_into().map_err(|_| "A36 row count exceeds u32")?,
            abl1_rows,
        });
        self.portable_joins
            .sort_by_key(|receipt| receipt.controller_sequence);
        Ok(())
    }

    pub fn seal_drain_and_uninstall(&mut self) -> Result<A36ActiveAbilityDrain, String> {
        if !self.installed {
            return Err("A36 active-ability evidence owner is already uninstalled".into());
        }
        abilities::uninstall_a729_ability_transition_hook()?;
        self.installed = false;
        Ok(A36ActiveAbilityDrain {
            diagnostics: abilities::diagnostics(),
            observations: abilities::take_observations(),
            disabled_reasons: abilities::take_disabled_reason_observations(),
            callbacks_quiesced: true,
            uninstalled_hook_count: abilities::HOOK_COUNT as u32,
            portable_joins: std::mem::take(&mut self.portable_joins),
        })
    }
}

impl Drop for A36ActiveAbilityEvidenceOwner {
    fn drop(&mut self) {
        if self.installed {
            let _ = abilities::uninstall_a729_ability_transition_hook();
            self.installed = false;
        }
    }
}

impl A40PeaceEvidenceOwner {
    pub fn install() -> Result<Self, String> {
        peace::install_for_evidence()?;
        Ok(Self {
            install_receipt: A40PeaceEvidenceInstallReceipt {
                lane: "A40_PEACE_LIFECYCLE_DIAGNOSTIC",
                exact_executable_sha256: peace::EXACT_A729_EXECUTABLE_SHA256,
                static_producer_consumer_manifest_sha256:
                    peace::STATIC_PRODUCER_CONSUMER_MANIFEST_SHA256,
                hook_sentinel_artifact_sha256: peace::HOOK_SENTINEL_ARTIFACT_SHA256,
                installed_hook_count: 4,
                production_carrier_enabled: false,
            },
            installed: true,
        })
    }

    pub fn install_receipt(&self) -> &A40PeaceEvidenceInstallReceipt {
        &self.install_receipt
    }

    pub fn diagnostics(&self) -> peace::HookDiagnostics {
        peace::diagnostics()
    }

    pub fn seal_drain_and_uninstall(&mut self) -> Result<peace::HookDrain, String> {
        if !self.installed {
            return Err("A40 peace evidence owner is already uninstalled".into());
        }
        let drain = peace::seal_drain_and_uninstall()?;
        self.installed = false;
        Ok(drain)
    }
}

impl Drop for A40PeaceEvidenceOwner {
    fn drop(&mut self) {
        if self.installed {
            let _ = peace::seal_drain_and_uninstall();
            self.installed = false;
        }
    }
}

impl A40SelectedDesignerEvidenceOwner {
    pub fn install() -> Result<Self, String> {
        // d78 owns exact controller order and remains independently installed;
        // A40 never fabricates or accepts an ordered identity list.
        design_catalog_context_live_lane::a729_hook::install()
            .map_err(|error| format!("A40 evidence d78 prerequisite failed: {error}"))?;
        designer::a729_hook::install_for_fixture()?;
        Ok(Self {
            install_receipt: EvidenceInstallReceipt {
                lane: "A40_SELECTED_DESIGNER",
                domain_id: designer::DOMAIN_ID,
                exact_executable_sha256: hex(&designer::EXACT_A729_EXECUTABLE_SHA256),
                authority_receipt_sha256s: vec![
                    hex(&designer::PRODUCER_MANIFEST_SHA256),
                    hex(&designer::CLASSIFIER_MANIFEST_SHA256),
                    hex(&designer::DIVISION_CLICK_THUNK_ARTIFACT_SHA256),
                    hex(&designer::DIVISION_WRAPPER_ARTIFACT_SHA256),
                    designer::a729_hook::EQUIPMENT_AUTODESIGN_SUCCESS_MANIFEST_SHA256
                        .to_string(),
                    designer::a729_hook::EQUIPMENT_AUTODESIGN_SUCCESS_SENTINEL_ARTIFACT_SHA256
                        .to_string(),
                ],
                installed_hook_count: 5,
                production_scheduling_enabled: designer::PRODUCTION_ACTIVATION,
            },
            installed: true,
        })
    }

    pub fn install_receipt(&self) -> &EvidenceInstallReceipt {
        &self.install_receipt
    }

    pub fn drain(
        &mut self,
        frame_hour: u64,
        elapsed_wall_ms: u64,
        family: designer::SelectedDesignerFamily,
    ) -> Result<ReservedDomainEvidenceRecord, String> {
        if !self.installed {
            return Err("A40 evidence owner is uninstalled".into());
        }
        let payload = designer::a729_hook::drain_fixture_payload(
            frame_hour,
            elapsed_wall_ms,
            family,
        )
        .map_err(|error| format!("A40 SDI1 evidence drain failed: {error:?}"))?;
        let decoded = designer::decode_payload(&payload)
            .map_err(|error| format!("A40 drained SDI1 failed self-decode: {error:?}"))?;
        let sequence = decoded
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.transition_sequence)
            .ok_or("A40 drained SDI1 omitted its authenticated snapshot")?;
        ReservedDomainEvidenceRecord::new(
            "A40_SELECTED_DESIGNER",
            designer::DOMAIN_ID,
            frame_hour,
            sequence,
            payload,
        )
    }

    pub fn uninstall(&mut self) -> Result<EvidenceUninstallReceipt, String> {
        if !self.installed {
            return Err("A40 evidence owner is already uninstalled".into());
        }
        let diagnostics = designer::a729_hook::diagnostics();
        designer::a729_hook::uninstall_fixture_hooks()?;
        self.installed = false;
        Ok(EvidenceUninstallReceipt {
            lane: "A40_SELECTED_DESIGNER",
            domain_id: designer::DOMAIN_ID,
            callbacks_observed: diagnostics.callback_count,
            callbacks_quiesced: true,
            shared_prerequisite_retained: true,
        })
    }
}

impl Drop for A40SelectedDesignerEvidenceOwner {
    fn drop(&mut self) {
        if self.installed {
            let _ = designer::a729_hook::uninstall_fixture_hooks();
            self.installed = false;
        }
    }
}

/// Opaque one-shot A34 join ticket. The public witness rows contain only
/// hook-observed identity/presentation facts; callers cannot replace them.
pub struct A34DecisionPanelResolutionTicket {
    receipt: panel::a729_hook::MaterializationReceipt,
}

impl A34DecisionPanelResolutionTicket {
    pub fn frame_hour(&self) -> u64 {
        self.receipt.frame_hour()
    }

    pub fn source_sequence(&self) -> u64 {
        self.receipt.source_sequence()
    }

    pub fn active_country_tag_db_id(&self) -> u32 {
        self.receipt.active_country_tag_db_id()
    }

    pub fn rows(&self) -> &[panel::a729_hook::HookRowWitness] {
        self.receipt.rows()
    }
}

pub struct A34DecisionPanelEvidenceOwner {
    generation: Option<u64>,
    install_receipt: EvidenceInstallReceipt,
    installed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum A34TargetScope {
    Country,
    State,
}

#[derive(Clone, Debug)]
struct A34StaticDecision {
    category_id: String,
    target_scope: Option<A34TargetScope>,
    political_power: Option<i64>,
    custom_cost: bool,
}

struct A34DecisionCatalog {
    sha256: String,
    decisions: BTreeMap<String, A34StaticDecision>,
}

impl A34DecisionCatalog {
    fn parse(bytes: &[u8], expected_sha256: &str) -> Result<Self, String> {
        let actual_sha256 = format!("{:x}", Sha256::digest(bytes));
        if actual_sha256 != expected_sha256 {
            return Err(format!(
                "A34 decision catalog SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256}"
            ));
        }
        let root: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("A34 decision catalog JSON is invalid: {error}"))?;
        let decisions = root
            .get("decisions")
            .and_then(serde_json::Value::as_object)
            .ok_or("A34 decision catalog lacks decisions")?;
        let mut parsed = BTreeMap::new();
        for (map_id, raw) in decisions {
            let row = raw
                .as_object()
                .ok_or_else(|| format!("A34 decision {map_id} is not an object"))?;
            if row.get("id").and_then(serde_json::Value::as_str) != Some(map_id.as_str()) {
                return Err(format!(
                    "A34 decision catalog key {map_id} disagrees with its id"
                ));
            }
            let category_id = row
                .get("category")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("A34 decision {map_id} lacks category"))?
                .to_string();
            let targets = row
                .get("targets")
                .and_then(serde_json::Value::as_object);
            let targeted = targets.is_some_and(|targets| {
                targets
                    .get("targeted_decision")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                    || targets
                        .get("has_target_trigger")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                    || targets
                        .get("has_target_root_trigger")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
            });
            let target_scope = match targets
                .and_then(|targets| targets.get("target_scope"))
                .and_then(serde_json::Value::as_str)
            {
                Some("country") => Some(A34TargetScope::Country),
                Some("state") => Some(A34TargetScope::State),
                Some(other) => {
                    return Err(format!(
                        "A34 decision {map_id} has unknown target_scope {other}"
                    ))
                }
                None if targeted => Some(A34TargetScope::Country),
                None => None,
            };
            let cost = row.get("cost").and_then(serde_json::Value::as_object);
            let political_power = cost
                .and_then(|cost| cost.get("political_power"))
                .and_then(|value| {
                    value
                        .as_i64()
                        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
                });
            let custom_cost = cost.is_some_and(|cost| {
                cost.get("has_custom_cost_trigger")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                    || cost.get("custom_cost_text").is_some_and(|value| !value.is_null())
            });
            if parsed
                .insert(
                    map_id.clone(),
                    A34StaticDecision {
                        category_id,
                        target_scope,
                        political_power,
                        custom_cost,
                    },
                )
                .is_some()
            {
                return Err(format!("A34 decision catalog duplicates {map_id}"));
            }
        }
        if parsed.is_empty() {
            return Err("A34 decision catalog is empty".into());
        }
        Ok(Self {
            sha256: actual_sha256,
            decisions: parsed,
        })
    }
}

pub struct A34DecisionPanelProductionSession {
    generation: Option<u64>,
    frame_hour: Option<u64>,
    catalog: A34DecisionCatalog,
    installed: bool,
}

/// Complete owner-runnable A34 evidence transaction result. All three
/// receipts are retained so an external harness can prove that hook
/// installation, the one-shot DPN1 drain, and quiescent teardown belonged to
/// the same invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A34DecisionPanelEvidenceTransaction {
    pub install: EvidenceInstallReceipt,
    pub record: ReservedDomainEvidenceRecord,
    pub uninstall: EvidenceUninstallReceipt,
}

/// Run the complete A34 evidence lifecycle around one real engine UI update.
///
/// `drive_ui_update` is the only environment-owned action: it must cause the
/// already-installed hooks to observe one complete country-decision view
/// materialization and return the selected-Present stamp captured afterward.
/// `resolve` receives the hook-minted one-shot ticket and may only supply the
/// pointer-free semantic supplement; family, identity, order, geometry, text,
/// alert membership, and receipt chronology remain hook-owned.
///
/// Every failure attempts quiescent uninstall before returning. The production
/// scheduling and fixture-acceptance latches remain untouched.
pub fn run_a34_decision_panel_evidence_transaction<Drive, Resolve>(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    drive_ui_update: Drive,
    resolve: Resolve,
) -> Result<A34DecisionPanelEvidenceTransaction, String>
where
    Drive: FnOnce() -> Result<panel::SelectedPresentStamp, String>,
    Resolve: FnOnce(
        &A34DecisionPanelResolutionTicket,
    ) -> Result<panel::a729_hook::ResolvedDecisionPanelSupplement, String>,
{
    let mut owner = A34DecisionPanelEvidenceOwner::install()?;
    let install = owner.install_receipt().clone();
    let transaction = (|| {
        owner.arm(frame_hour)?;
        let selected_present = drive_ui_update()?;
        if selected_present.frame_hour != frame_hour {
            return Err("A34 evidence driver returned a different frame hour".into());
        }
        let ticket = owner.seal(selected_present)?;
        let supplement = resolve(&ticket)?;
        owner.resolve_and_drain(ticket, supplement, elapsed_wall_ms)
    })();
    match transaction {
        Ok(record) => {
            let uninstall = owner.uninstall()?;
            Ok(A34DecisionPanelEvidenceTransaction {
                install,
                record,
                uninstall,
            })
        }
        Err(primary) => {
            let abort_error = if owner.generation.is_some() {
                owner.abort(&primary).err()
            } else {
                None
            };
            let primary = match abort_error {
                Some(abort) => {
                    format!("{primary}; A34 evidence epoch abort also failed: {abort}")
                }
                None => primary,
            };
            match owner.uninstall() {
                Ok(_) => Err(primary),
                Err(cleanup) => Err(format!(
                    "{primary}; A34 evidence teardown after failure also failed: {cleanup}"
                )),
            }
        }
    }
}

impl A34DecisionPanelEvidenceOwner {
    pub fn install() -> Result<Self, String> {
        panel::a729_hook::install_for_evidence()?;
        Ok(Self {
            generation: None,
            install_receipt: EvidenceInstallReceipt {
                lane: "A34_DECISION_PANEL",
                domain_id: panel::DOMAIN_ID,
                exact_executable_sha256: hex(&panel::EXECUTABLE_SHA256),
                authority_receipt_sha256s: vec![
                    hex(&panel::EXACT20_FUNCTION_MANIFEST_SHA256),
                    hex(&panel::EXACT20_DATA_ARTIFACT_SHA256),
                    hex(&panel::COUNTRY_VIEW_OWNER_TAG_MANIFEST_SHA256),
                    hex(&panel::SEMANTIC_LEAF_MANIFEST_SHA256),
                    hex(&panel::ON_MAP_IDENTITY_MANIFEST_SHA256),
                ],
                installed_hook_count: 15,
                production_scheduling_enabled: panel::PRODUCTION_SCHEDULING_ENABLED,
            },
            installed: true,
        })
    }

    pub fn install_receipt(&self) -> &EvidenceInstallReceipt {
        &self.install_receipt
    }

    pub fn arm(&mut self, frame_hour: u64) -> Result<u64, String> {
        if !self.installed {
            return Err("A34 evidence owner is uninstalled".into());
        }
        if self.generation.is_some() {
            return Err("A34 evidence owner already has an armed epoch".into());
        }
        let generation = panel::a729_hook::begin_capture_epoch(frame_hour)?;
        self.generation = Some(generation);
        Ok(generation)
    }

    pub fn abort(&mut self, reason: &str) -> Result<(), String> {
        if !self.installed {
            return Err("A34 evidence owner is uninstalled".into());
        }
        let generation = self
            .generation
            .take()
            .ok_or("A34 evidence owner has no armed epoch")?;
        panel::a729_hook::abort_capture_epoch(generation, reason)
    }

    pub fn seal(
        &mut self,
        selected_present: panel::SelectedPresentStamp,
    ) -> Result<A34DecisionPanelResolutionTicket, String> {
        if !self.installed {
            return Err("A34 evidence owner is uninstalled".into());
        }
        let generation = self
            .generation
            .take()
            .ok_or("A34 evidence owner has no armed epoch")?;
        panel::a729_hook::seal_capture_epoch_at_selected_present(
            generation,
            selected_present,
        )?;
        Ok(A34DecisionPanelResolutionTicket {
            receipt: panel::a729_hook::take_materialization_receipt()?,
        })
    }

    pub fn resolve_and_drain(
        &mut self,
        ticket: A34DecisionPanelResolutionTicket,
        supplement: panel::a729_hook::ResolvedDecisionPanelSupplement,
        elapsed_wall_ms: u64,
    ) -> Result<ReservedDomainEvidenceRecord, String> {
        if !self.installed {
            return Err("A34 evidence owner is uninstalled".into());
        }
        let frame_hour = ticket.receipt.frame_hour();
        let source_sequence = ticket.receipt.source_sequence();
        panel::a729_hook::admit_resolved_supplement(ticket.receipt, supplement)?;
        let payload = panel::a729_hook::drain_evidence_payload(frame_hour, elapsed_wall_ms)
            .map_err(|error| format!("A34 DPN1 evidence drain failed: {error:?}"))?;
        ReservedDomainEvidenceRecord::new(
            "A34_DECISION_PANEL",
            panel::DOMAIN_ID,
            frame_hour,
            source_sequence,
            payload,
        )
    }

    pub fn uninstall(&mut self) -> Result<EvidenceUninstallReceipt, String> {
        if !self.installed {
            return Err("A34 evidence owner is already uninstalled".into());
        }
        if self.generation.is_some() {
            return Err("A34 evidence owner cannot uninstall with an open epoch".into());
        }
        let diagnostics = panel::a729_hook::diagnostics();
        panel::a729_hook::uninstall()?;
        self.installed = false;
        Ok(EvidenceUninstallReceipt {
            lane: "A34_DECISION_PANEL",
            domain_id: panel::DOMAIN_ID,
            callbacks_observed: diagnostics
                .materializer_calls
                .saturating_add(diagnostics.can_take_calls)
                .saturating_add(diagnostics.timeout_calls),
            callbacks_quiesced: true,
            shared_prerequisite_retained: true,
        })
    }
}

struct A34ProductionSemanticResolver<'a> {
    state: &'a super::attach_log::CollectorState,
    catalog: &'a A34DecisionCatalog,
    country_tag_db_id: Option<u32>,
    decisions: Option<super::attach_log::CountryDecisionsRow>,
}

impl<'a> A34ProductionSemanticResolver<'a> {
    fn new(
        state: &'a super::attach_log::CollectorState,
        catalog: &'a A34DecisionCatalog,
    ) -> Self {
        Self {
            state,
            catalog,
            country_tag_db_id: None,
            decisions: None,
        }
    }

    fn tag_for_db_id(&self, tag_db_id: u32) -> Option<String> {
        self.state
            .country_identity
            .rows
            .iter()
            .find(|row| row.tag_db_id == tag_db_id)
            .map(|row| row.tag.clone())
    }

    fn target_from_runtime(
        &self,
        query: &panel::a729_hook::ProductionDecisionSemanticQuery<'_>,
        scope: A34TargetScope,
    ) -> Result<panel::Target, String> {
        if query.family == panel::a729_hook::PresenterFamily::Targeted && query.record != 0 {
            let target_id = super::attach_log::read_u32_stable(
                query
                    .record
                    .checked_add(0x30)
                    .ok_or("A34 targeted record address overflow")?,
            )
            .filter(|value| *value != 0)
            .ok_or("A34 targeted record has no exact target id")?;
            return match scope {
                A34TargetScope::Country => self
                    .tag_for_db_id(target_id)
                    .map(panel::Target::Country)
                    .ok_or_else(|| {
                        format!(
                            "A34 targeted country DB id {target_id} is absent from the authenticated roster"
                        )
                    }),
                A34TargetScope::State => Ok(panel::Target::State(target_id)),
            };
        }
        if let Some(decisions) = self.decisions.as_ref() {
            let mut candidates = Vec::new();
            for row in decisions
                .targeted_live
                .rows
                .iter()
                .chain(decisions.targeted_timed_live.rows.iter())
                .filter(|row| row.decision == query.decision_id)
            {
                let target = match (scope, row.target) {
                    (
                        A34TargetScope::Country,
                        super::attach_log::TargetedDecisionTarget::Country(tag_db_id),
                    ) => self
                        .tag_for_db_id(tag_db_id)
                        .map(panel::Target::Country)
                        .ok_or_else(|| {
                            format!(
                                "A34 target country DB id {tag_db_id} is absent from the authenticated roster"
                            )
                        })?,
                    (
                        A34TargetScope::State,
                        super::attach_log::TargetedDecisionTarget::Numeric(state_id),
                    ) => panel::Target::State(state_id),
                    _ => {
                        return Err(format!(
                            "A34 decision {} runtime target kind disagrees with the effective catalog",
                            query.decision_id
                        ))
                    }
                };
                if !candidates.contains(&target) {
                    candidates.push(target);
                }
            }
            if let [target] = candidates.as_slice() {
                return Ok(target.clone());
            }
            if candidates.len() > 1 {
                return Err(format!(
                    "A34 decision {} has ambiguous same-hour targets",
                    query.decision_id
                ));
            }
        }
        Err(format!(
            "A34 decision {} requires a target but its same-hour target is unavailable",
            query.decision_id
        ))
    }

    fn active_days(&self, decision_id: &str, target: Option<&panel::Target>) -> Option<i32> {
        let decisions = self.decisions.as_ref()?;
        if target.is_none() {
            return decisions
                .timed_live
                .iter()
                .flatten()
                .find(|row| row.decision == decision_id)
                .map(|row| row.days);
        }
        decisions
            .targeted_live
            .rows
            .iter()
            .chain(decisions.targeted_timed_live.rows.iter())
            .find(|row| {
                row.decision == decision_id
                    && match (row.target, target) {
                        (
                            super::attach_log::TargetedDecisionTarget::Country(tag_db_id),
                            Some(panel::Target::Country(tag)),
                        ) => self.tag_for_db_id(tag_db_id).as_deref() == Some(tag.as_str()),
                        (
                            super::attach_log::TargetedDecisionTarget::Numeric(state_id),
                            Some(panel::Target::State(target_state_id)),
                        ) => state_id == *target_state_id,
                        _ => false,
                    }
            })
            .map(|row| row.days)
    }

    fn cooldown_days(&self, decision_id: &str) -> Option<i32> {
        let decisions = self.decisions.as_ref()?;
        decisions
            .re_enable_cooldowns
            .iter()
            .flatten()
            .find(|row| row.decision == decision_id)
            .map(|row| row.days)
    }
}

impl panel::a729_hook::ProductionDecisionSemanticSource
    for A34ProductionSemanticResolver<'_>
{
    fn country_tag(&mut self, country_tag_db_id: u32) -> Result<String, String> {
        let row = self
            .state
            .country_identity
            .rows
            .iter()
            .find(|row| row.tag_db_id == country_tag_db_id)
            .ok_or("A34 active country is absent from the authenticated roster")?;
        if row.object_ptr == 0
            || self.state.country_identity.country_vtable == 0
            || super::attach_log::read_u64_stable(row.object_ptr)
                != Some(self.state.country_identity.country_vtable as u64)
        {
            return Err("A34 active country object/vtable is unavailable".into());
        }
        self.decisions = super::attach_log::read_country_decisions(
            row.object_ptr,
            self.state.country_identity.country_vtable,
        );
        self.country_tag_db_id = Some(country_tag_db_id);
        Ok(row.tag.clone())
    }

    fn decision_catalog_sha256(&self) -> &str {
        &self.catalog.sha256
    }

    fn resolve(
        &mut self,
        query: panel::a729_hook::ProductionDecisionSemanticQuery<'_>,
    ) -> Result<panel::a729_hook::ProductionDecisionSemantics, String> {
        if self.country_tag_db_id.is_none() {
            return Err("A34 semantic resolver has no authenticated active country".into());
        }
        let static_row = self
            .catalog
            .decisions
            .get(query.decision_id)
            .ok_or_else(|| {
                format!(
                    "A34 decision {} is absent from the admitted effective catalog",
                    query.decision_id
                )
            })?;
        let target = static_row
            .target_scope
            .map(|scope| self.target_from_runtime(&query, scope))
            .transpose()?;
        let formatted_text = match query.cost_texts {
            [] => None,
            [text] => Some(text.clone()),
            _ => {
                return Err(format!(
                    "A34 decision {} has ambiguous exact cost text",
                    query.decision_id
                ))
            }
        };
        let cost = if static_row.political_power.is_some()
            || static_row.custom_cost
            || formatted_text.is_some()
        {
            Some(panel::Cost {
                political_power: static_row.political_power,
                custom_cost_available: Some(static_row.custom_cost),
                formatted_text,
            })
        } else {
            None
        };
        let mut remaining_timer_days =
            self.active_days(query.decision_id, target.as_ref());
        if remaining_timer_days.is_none()
            && query.family == panel::a729_hook::PresenterFamily::Timed
            && query.record != 0
        {
            remaining_timer_days = super::attach_log::read_i32_stable(
                query
                    .record
                    .checked_add(0x18)
                    .ok_or("A34 timed record address overflow")?,
            );
        }
        if remaining_timer_days.is_some_and(|days| days < 0) {
            return Err(format!(
                "A34 decision {} has a negative remaining timer",
                query.decision_id
            ));
        }
        let cooldown_remaining_days = self.cooldown_days(query.decision_id);
        if cooldown_remaining_days.is_some_and(|days| days <= 0) {
            return Err(format!(
                "A34 decision {} has a nonpositive cooldown",
                query.decision_id
            ));
        }
        Ok(panel::a729_hook::ProductionDecisionSemantics {
            category_id: static_row.category_id.clone(),
            target,
            cost,
            remaining_timer_days,
            cooldown_remaining_days,
        })
    }
}

impl A34DecisionPanelProductionSession {
    pub fn install(catalog_bytes: &[u8], catalog_sha256: &str) -> Result<Self, String> {
        let catalog = A34DecisionCatalog::parse(catalog_bytes, catalog_sha256)?;
        panel::a729_hook::install()?;
        Ok(Self {
            generation: None,
            frame_hour: None,
            catalog,
            installed: true,
        })
    }

    pub fn ensure_armed(&mut self, frame_hour: u64) -> Result<(), String> {
        if !self.installed {
            return Err("A34 production owner is uninstalled".into());
        }
        if self.generation.is_some() && self.frame_hour == Some(frame_hour) {
            return Ok(());
        }
        if self.generation.is_some() {
            self.abort_epoch("A34 frame hour advanced before selected Present")?;
        }
        self.generation = Some(panel::a729_hook::begin_capture_epoch(frame_hour)?);
        self.frame_hour = Some(frame_hour);
        Ok(())
    }

    pub fn seal_selected_present(
        &mut self,
        state: &super::attach_log::CollectorState,
        selected_present: panel::SelectedPresentStamp,
        elapsed_wall_ms: u64,
    ) -> Result<Vec<u8>, String> {
        let generation = self
            .generation
            .take()
            .ok_or("A34 production owner has no armed epoch")?;
        if self.frame_hour.take() != Some(selected_present.frame_hour) {
            panel::a729_hook::abort_capture_epoch(
                generation,
                "A34 armed and selected-Present frame hours differ",
            )?;
            return Err("A34 armed and selected-Present frame hours differ".into());
        }
        panel::a729_hook::seal_capture_epoch_at_selected_present(
            generation,
            selected_present.clone(),
        )?;
        let receipt = panel::a729_hook::take_materialization_receipt()?;
        let mut resolver = A34ProductionSemanticResolver::new(state, &self.catalog);
        panel::a729_hook::admit_production_semantics(receipt, &mut resolver)?;
        panel::collect_and_encode(
            &panel::a729_hook::A729DecisionPanelHookSource,
            selected_present.frame_hour,
            elapsed_wall_ms,
        )
        .map_err(|error| format!("A34 production DPN1 encode failed: {error:?}"))
    }

    pub fn abort_epoch(&mut self, reason: &str) -> Result<(), String> {
        let Some(generation) = self.generation.take() else {
            self.frame_hour = None;
            return Ok(());
        };
        self.frame_hour = None;
        panel::a729_hook::abort_capture_epoch(generation, reason)
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        if !self.installed {
            return Ok(());
        }
        if self.generation.is_some() {
            self.abort_epoch("A34 production session teardown")?;
        }
        panel::a729_hook::uninstall()?;
        self.installed = false;
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_record_frames_exact_domain_data_without_scheduling() {
        let record = ReservedDomainEvidenceRecord::new(
            "fixture",
            designer::DOMAIN_ID,
            42,
            7,
            vec![1, 2, 3],
        )
        .unwrap();
        let bytes = record.encode_h4cr(11, 42).unwrap();
        let header = compact_contract::parse_compact_record_header(&bytes).unwrap();
        assert_eq!(header.sequence, 11);
        assert_eq!(header.domain_id, designer::DOMAIN_ID);
        assert_eq!(header.record_type, compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA);
        assert_eq!(&bytes[compact_contract::COMPACT_RECORD_HEADER_LEN..], &[1, 2, 3]);
    }

    #[test]
    fn reserved_record_rejects_hour_or_sequence_substitution() {
        let record = ReservedDomainEvidenceRecord::new(
            "fixture",
            panel::DOMAIN_ID,
            42,
            7,
            vec![1],
        )
        .unwrap();
        assert!(record.encode_h4cr(0, 42).is_err());
        assert!(record.encode_h4cr(11, 43).is_err());
    }
}
