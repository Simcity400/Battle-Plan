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
    pub cost_command_power_fixed: i64,
    pub display_order: u32,
    pub tooltip: String,
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
                // Evidence acquisition never self-authorizes product
                // scheduling, even after a future independent gate promotion.
                production_activation_authorized: false,
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
                ],
                installed_hook_count: 12,
                production_scheduling_enabled: panel::PRODUCTION_SCHEDULING_ENABLED
                    && panel::LIVE_FIXTURE_ACCEPTED,
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
