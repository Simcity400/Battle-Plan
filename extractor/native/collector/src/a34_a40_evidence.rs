//! Owner-runnable evidence acquisition for reserved A34/d87 and A40/d85.
//!
//! These owners are deliberately absent from the production scheduler. They
//! install the exact sentinel-gated hooks, destructively drain lane-owned
//! pointer-free payloads, frame them as ordinary H4CR domain data on request,
//! and uninstall only after all detour callbacks have quiesced.

use design_catalog_context_live_lane::selected_designer_identity as designer;
use decision_panel_live_lane as panel;
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
                ],
                installed_hook_count: 4,
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

    pub fn rows(&self) -> &[panel::a729_hook::HookRowWitness] {
        self.receipt.rows()
    }
}

pub struct A34DecisionPanelEvidenceOwner {
    generation: Option<u64>,
    install_receipt: EvidenceInstallReceipt,
    installed: bool,
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
                ],
                installed_hook_count: 10,
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

    pub fn seal(&mut self) -> Result<A34DecisionPanelResolutionTicket, String> {
        if !self.installed {
            return Err("A34 evidence owner is uninstalled".into());
        }
        let generation = self
            .generation
            .take()
            .ok_or("A34 evidence owner has no armed epoch")?;
        panel::a729_hook::seal_capture_epoch(generation)?;
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
