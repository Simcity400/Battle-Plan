//! Exact-a729 seven-surface intelligence visibility observation.
//!
//! Runtime pointers are sampled only after an authenticated controller refresh
//! has returned.  The stored observation and final receipt are pointer-free.

#![cfg(windows)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::mem;
use std::sync::Mutex;
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub const CURRENT_UI_MODE_POINTER_RVA: usize = 0x3324028;
pub const AGENCY_UI_MODE: i32 = 3;
pub const INTEL_LEDGER_UI_MODE: i32 = 0x1e;
pub const INTEL_LEDGER_REFRESH_RVA: usize = 0x19c7450;

pub const VISIBILITY_PRESENTER_JOB_ID: &str =
    "20260721T090420Z-wave6-xui14-seven-surface-presenters-0bd762bb";
pub const VISIBILITY_PRESENTER_MANIFEST_SHA256: &str =
    "abf3f75adabcd94ecae61cb035c1e52374332c087a1a900b6b91b93ca4ba37bd";
pub const AGENCY_GATE_JOB_ID: &str =
    "20260721T090956Z-wave6-xui14-agency-tab-visibility-afde0980";
pub const AGENCY_GATE_MANIFEST_SHA256: &str =
    "6f6639c028d3cb49a65ed2fe2b4a00b25839c464e67bc562f924c58c11d27863";
pub const OPERATION_CIPHER_JOB_ID: &str =
    "20260721T091244Z-wave6-xui14-operation-crypto-tab-clicks-ea806c56";
pub const OPERATION_CIPHER_MANIFEST_SHA256: &str =
    "79b580872121af00d23a50080bfb91a6b68a0b465c1e6a36a1b6c7107495e7f3";
pub const LEDGER_JOB_ID: &str =
    "20260721T092056Z-wave6-xui14-ledger-visibility-producer-8776bdb5";
pub const LEDGER_MANIFEST_SHA256: &str =
    "503e8d0490a157dc3a642aff5007ef34d091f26d52dee097c3ab3c901ac1bacb";
pub const UI_MODE_JOB_ID: &str =
    "20260721T093525Z-wave6-xui14-agency-vtable-lifecycle-10dc912a";
pub const UI_MODE_MANIFEST_SHA256: &str =
    "89c1136b21c4d6449297d4dac5400d234ad3be75a5fced26752b0f5f24ec1aa9";
pub const LEDGER_HOOK_JOB_ID: &str =
    "20260721T094709Z-wave6-xui14-ledger-hook-signature-6ef86474";
pub const LEDGER_HOOK_MANIFEST_SHA256: &str =
    "643bd80d2736b265cdfcd3802e0427d655538c23c1331cbc80a0c7e0d21dc14b";
pub const LEDGER_SENTINEL_JOB_ID: &str =
    "20260721T094832Z-wave6-xui14-ledger-hook-sentinel-2ac6f5a1";
pub const LEDGER_SENTINEL_ARTIFACT_SHA256: &str =
    "2cb2ca7be36864933aa7e15728d2357ae5443d7f8d889411200b7f466928b36c";
pub const GUI_FILE_SHA256: &str =
    "00153f0968113c3f29d914bda227c729ba5d95f5888135c0dd98db89748ef3e5";

const WIDGET_NAME_LOGO: usize = 0x630;
const WIDGET_UPGRADE_PROGRESS: usize = 0x638;
const WIDGET_OPERATIVE_PROGRESS: usize = 0x640;
const WIDGET_BEFORE_CREATION: usize = 0x648;
const WIDGET_UPGRADES_BEFORE: usize = 0x650;
const WIDGET_OPERATIVES_BEFORE: usize = 0x658;
const WIDGET_OPERATIONS: usize = 0x660;
const WIDGET_CIPHERS: usize = 0x668;
const WIDGET_HIDDEN_BYTE: usize = 0xa5;
const WIDGET_HIDDEN_BIT: u8 = 0x10;
const LEDGER_SELECTED_ID: usize = 0x1480;
const LEDGER_RECORDS: [usize; 4] = [0x1488, 0x14b8, 0x1518, 0x14e8];

static OBSERVATIONS: Mutex<Vec<VisibilityObservation>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelligenceSurface {
    Agency,
    Upgrades,
    Operatives,
    Operations,
    Ciphers,
    IntelLedger,
    Networks,
}

impl IntelligenceSurface {
    pub const ALL: [Self; 7] = [
        Self::Agency,
        Self::Upgrades,
        Self::Operatives,
        Self::Operations,
        Self::Ciphers,
        Self::IntelLedger,
        Self::Networks,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityDisposition {
    Visible,
    Hidden,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligenceSurfaceGate {
    pub surface: IntelligenceSurface,
    pub disposition: VisibilityDisposition,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkProductWitness {
    frame_hour: u64,
    owner_tag_db_id: u32,
    producer_record_identity: [u8; 32],
    payload_sha256: String,
    row_count: u32,
}

impl NetworkProductWitness {
    pub fn from_domain53_report(
        report: &crate::extractor::Domain53ExtractionReport,
        frame_hour: u64,
        owner_tag_db_id: u32,
        producer_record_identity: [u8; 32],
    ) -> Result<Self, String> {
        if frame_hour == 0
            || owner_tag_db_id == 0
            || producer_record_identity.iter().all(|byte| *byte == 0)
            || !report.errors.is_empty()
            || report.source_proofs & crate::extractor::SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF == 0
        {
            return Err("A30 network visibility source proof is unavailable".into());
        }
        let row_count = u32::try_from(report.snapshot.intel_network_graphs.len())
            .map_err(|_| "A30 network visibility row count overflow")?;
        Ok(Self {
            frame_hour,
            owner_tag_db_id,
            producer_record_identity,
            payload_sha256: hex(&Sha256::digest(&report.payload)),
            row_count,
        })
    }

    pub fn from_authenticated_payload(
        frame_hour: u64,
        owner_tag_db_id: u32,
        producer_record_identity: [u8; 32],
        payload_sha256: String,
        row_count: u32,
        source_proof_present: bool,
    ) -> Result<Self, String> {
        if frame_hour == 0
            || owner_tag_db_id == 0
            || producer_record_identity.iter().all(|byte| *byte == 0)
            || !source_proof_present
            || !lower_sha256(&payload_sha256)
        {
            return Err("A30 network payload authority is unavailable".into());
        }
        Ok(Self { frame_hour, owner_tag_db_id, producer_record_identity, payload_sha256, row_count })
    }

    pub(crate) fn matches(
        &self,
        frame_hour: u64,
        owner_tag_db_id: u32,
        producer_record_identity: &[u8; 32],
    ) -> bool {
        self.frame_hour == frame_hour
            && self.owner_tag_db_id == owner_tag_db_id
            && &self.producer_record_identity == producer_record_identity
            && lower_sha256(&self.payload_sha256)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibilityProductJoins {
    frame_hour: u64,
    owner_tag_db_id: u32,
    producer_record_identity: [u8; 32],
    upgrade_controller_complete: bool,
    cipher_controller_complete: bool,
}

impl VisibilityProductJoins {
    pub fn from_controller_refreshes(
        agency: &crate::agency_upgrade_controller::AgencyUpgradeControllerRefresh,
        cipher: &crate::cipher_controller::CipherControllerRefresh,
    ) -> Result<Self, String> {
        if agency.frame_hour == 0
            || agency.owner_tag_db_id == 0
            || agency.frame_hour != cipher.frame_hour
            || agency.owner_tag_db_id != cipher.owner_tag_db_id
            || agency.producer_record_identity != cipher.producer_record_identity
            || agency.producer_record_identity.iter().all(|byte| *byte == 0)
        {
            return Err("A30 visibility controller products are torn".into());
        }
        Ok(Self {
            frame_hour: agency.frame_hour,
            owner_tag_db_id: agency.owner_tag_db_id,
            producer_record_identity: agency.producer_record_identity,
            upgrade_controller_complete: true,
            cipher_controller_complete: true,
        })
    }

    fn matches(&self, frame_hour: u64, owner_tag_db_id: u32, identity: &str) -> bool {
        self.frame_hour == frame_hour
            && self.owner_tag_db_id == owner_tag_db_id
            && hex(&self.producer_record_identity) == identity
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityObservationKind {
    AgencyRefresh,
    IntelLedgerRefresh,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VisibilityObservation {
    kind: VisibilityObservationKind,
    frame_hour: u64,
    owner_tag_db_id: u32,
    producer_record_identity: [u8; 32],
    refresh_end_qpc_ticks: u64,
    current_ui_mode: i32,
    agency_created: Option<bool>,
    operations_visible: Option<bool>,
    ciphers_visible: Option<bool>,
    ledger_selected_tab_id: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityReceipt {
    pub schema: String,
    pub executable_sha256: String,
    pub frame_hour: u64,
    pub owner_tag_db_id: u32,
    pub producer_record_identity: String,
    pub observation_kind: VisibilityObservationKind,
    pub refresh_end_qpc_ticks: u64,
    pub current_ui_mode: i32,
    pub agency_created: Option<bool>,
    pub operations_visible: Option<bool>,
    pub ciphers_visible: Option<bool>,
    pub ledger_selected_tab_id: Option<i32>,
    pub network_payload_sha256: Option<String>,
    pub network_row_count: Option<u32>,
    pub gates: Vec<IntelligenceSurfaceGate>,
    pub visibility_presenter_manifest_sha256: String,
    pub agency_gate_manifest_sha256: String,
    pub operation_cipher_manifest_sha256: String,
    pub ledger_manifest_sha256: String,
    pub ui_mode_manifest_sha256: String,
    pub ledger_hook_manifest_sha256: String,
    pub ledger_sentinel_artifact_sha256: String,
    pub gui_file_sha256: String,
}

pub(crate) fn reset() -> Result<(), String> {
    OBSERVATIONS
        .lock()
        .map_err(|_| "A30 visibility observation lock poisoned")?
        .clear();
    Ok(())
}

/// Process-local surface classification for the autonomous evidence owner.
/// It exposes no controller/widget pointer and is used only to avoid sealing a
/// completed controller transaction against the wrong UI transition.
pub fn latest_observed_surface() -> Option<IntelligenceSurface> {
    let observations = OBSERVATIONS.try_lock().ok()?;
    let row = observations.iter().max_by_key(|row| row.refresh_end_qpc_ticks)?;
    match row.kind {
        VisibilityObservationKind::IntelLedgerRefresh => Some(IntelligenceSurface::IntelLedger),
        VisibilityObservationKind::AgencyRefresh => match (
            row.operations_visible,
            row.ciphers_visible,
        ) {
            (Some(true), Some(false)) => Some(IntelligenceSurface::Operations),
            (Some(false), Some(true)) => Some(IntelligenceSurface::Ciphers),
            _ => Some(IntelligenceSurface::Agency),
        },
    }
}

pub(crate) fn record_after_agency_refresh_tail(
    view: usize,
    module_base: usize,
    frame_hour: u64,
    owner_tag_db_id: u32,
    producer_record_identity: [u8; 32],
    refresh_end_qpc_ticks: u64,
) -> Result<(), String> {
    let current_ui_mode = read_current_ui_mode(module_base)?;
    let created_pattern = [
        widget_visible(view, WIDGET_NAME_LOGO),
        widget_visible(view, WIDGET_UPGRADE_PROGRESS),
        widget_visible(view, WIDGET_OPERATIVE_PROGRESS),
        widget_visible(view, WIDGET_BEFORE_CREATION),
        widget_visible(view, WIDGET_UPGRADES_BEFORE),
        widget_visible(view, WIDGET_OPERATIVES_BEFORE),
    ];
    let agency_created = match created_pattern {
        [Some(true), Some(true), Some(true), Some(false), Some(false), Some(false)] => Some(true),
        [Some(false), Some(false), Some(false), Some(true), Some(true), Some(true)] => Some(false),
        _ => None,
    };
    let operations_visible = widget_visible(view, WIDGET_OPERATIONS);
    let ciphers_visible = widget_visible(view, WIDGET_CIPHERS);
    push(VisibilityObservation {
        kind: VisibilityObservationKind::AgencyRefresh,
        frame_hour,
        owner_tag_db_id,
        producer_record_identity,
        refresh_end_qpc_ticks,
        current_ui_mode,
        agency_created,
        operations_visible,
        ciphers_visible,
        ledger_selected_tab_id: None,
    })
}

/// A production hook supplies the entry `view` unchanged after calling the
/// original function; the selected id must match exactly one of four records.
pub(crate) fn record_after_ledger_refresh(
    view: usize,
    module_base: usize,
    frame_hour: u64,
    owner_tag_db_id: u32,
    producer_record_identity: [u8; 32],
    refresh_end_qpc_ticks: u64,
) -> Result<(), String> {
    let current_ui_mode = read_current_ui_mode(module_base)?;
    let selected = read_stable::<i32>(view.saturating_add(LEDGER_SELECTED_ID))
        .ok_or_else(|| "A30 ledger selected id is unreadable".to_string())?;
    let matches = LEDGER_RECORDS
        .iter()
        .filter(|offset| read_stable::<i32>(view.saturating_add(**offset)) == Some(selected))
        .count();
    push(VisibilityObservation {
        kind: VisibilityObservationKind::IntelLedgerRefresh,
        frame_hour,
        owner_tag_db_id,
        producer_record_identity,
        refresh_end_qpc_ticks,
        current_ui_mode,
        agency_created: None,
        operations_visible: None,
        ciphers_visible: None,
        ledger_selected_tab_id: (matches == 1).then_some(selected),
    })
}

pub(crate) fn seal_for_selected_present(
    frame_hour: u64,
    owner_tag_db_id: u32,
    producer_record_identity: &str,
    selected_qpc_ticks: u64,
    products: &VisibilityProductJoins,
    network: Option<&NetworkProductWitness>,
) -> Result<VisibilityReceipt, String> {
    if !products.matches(frame_hour, owner_tag_db_id, producer_record_identity) {
        return Err("A30 visibility product join identity mismatch".into());
    }
    let identity = decode_sha256(producer_record_identity)?;
    let mut observations = OBSERVATIONS
        .lock()
        .map_err(|_| "A30 visibility observation lock poisoned")?;
    let candidates = observations
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.frame_hour == frame_hour
                && row.owner_tag_db_id == owner_tag_db_id
                && row.producer_record_identity == identity
                && row.refresh_end_qpc_ticks <= selected_qpc_ticks
        })
        .map(|(index, row)| (index, row.refresh_end_qpc_ticks))
        .collect::<Vec<_>>();
    let latest_qpc = candidates.iter().map(|(_, qpc)| *qpc).max()
        .ok_or_else(|| "A30 visibility observation is missing or stale".to_string())?;
    let latest = candidates.iter().filter(|(_, qpc)| *qpc == latest_qpc).collect::<Vec<_>>();
    if latest.len() != 1 { return Err("A30 visibility observation is duplicated or torn".into()); }
    let observation = observations.remove(latest[0].0);
    let network = network.filter(|witness| witness.matches(frame_hour, owner_tag_db_id, &identity));
    let gates = materialize_gates(&observation, products, network);
    validate_gates(&gates)?;
    Ok(VisibilityReceipt {
        schema: "hoi4.a30.intelligence_visibility.v1".into(),
        executable_sha256: super::a30_operative_operation_evidence::EXECUTABLE_SHA256.into(),
        frame_hour,
        owner_tag_db_id,
        producer_record_identity: producer_record_identity.into(),
        observation_kind: observation.kind,
        refresh_end_qpc_ticks: observation.refresh_end_qpc_ticks,
        current_ui_mode: observation.current_ui_mode,
        agency_created: observation.agency_created,
        operations_visible: observation.operations_visible,
        ciphers_visible: observation.ciphers_visible,
        ledger_selected_tab_id: observation.ledger_selected_tab_id,
        network_payload_sha256: network.map(|witness| witness.payload_sha256.clone()),
        network_row_count: network.map(|witness| witness.row_count),
        gates,
        visibility_presenter_manifest_sha256: VISIBILITY_PRESENTER_MANIFEST_SHA256.into(),
        agency_gate_manifest_sha256: AGENCY_GATE_MANIFEST_SHA256.into(),
        operation_cipher_manifest_sha256: OPERATION_CIPHER_MANIFEST_SHA256.into(),
        ledger_manifest_sha256: LEDGER_MANIFEST_SHA256.into(),
        ui_mode_manifest_sha256: UI_MODE_MANIFEST_SHA256.into(),
        ledger_hook_manifest_sha256: LEDGER_HOOK_MANIFEST_SHA256.into(),
        ledger_sentinel_artifact_sha256: LEDGER_SENTINEL_ARTIFACT_SHA256.into(),
        gui_file_sha256: GUI_FILE_SHA256.into(),
    })
}

fn materialize_gates(
    row: &VisibilityObservation,
    products: &VisibilityProductJoins,
    network: Option<&NetworkProductWitness>,
) -> Vec<IntelligenceSurfaceGate> {
    if row.current_ui_mode != AGENCY_UI_MODE && row.current_ui_mode != INTEL_LEDGER_UI_MODE {
        return IntelligenceSurface::ALL
            .into_iter()
            .map(|surface| gate(surface, VisibilityDisposition::Hidden, "different authenticated top-level UI mode"))
            .collect();
    }
    if row.current_ui_mode == INTEL_LEDGER_UI_MODE {
        return IntelligenceSurface::ALL
            .into_iter()
            .map(|surface| {
                if surface == IntelligenceSurface::IntelLedger && row.ledger_selected_tab_id.is_some() {
                    gate(surface, VisibilityDisposition::Visible, "")
                } else if surface == IntelligenceSurface::IntelLedger {
                    gate(surface, VisibilityDisposition::Unavailable, "ledger refresh did not select exactly one tab")
                } else {
                    gate(surface, VisibilityDisposition::Hidden, "intel ledger top-level UI mode selected")
                }
            })
            .collect();
    }
    let agency_created = row.agency_created;
    IntelligenceSurface::ALL
        .into_iter()
        .map(|surface| match surface {
            IntelligenceSurface::Agency => gate(surface, VisibilityDisposition::Visible, ""),
            IntelligenceSurface::IntelLedger => gate(surface, VisibilityDisposition::Hidden, "agency top-level UI mode selected"),
            _ if agency_created == Some(false) => gate(surface, VisibilityDisposition::Hidden, "intelligence agency not created"),
            IntelligenceSurface::Upgrades if agency_created == Some(true) && products.upgrade_controller_complete => gate(surface, VisibilityDisposition::Visible, ""),
            IntelligenceSurface::Operatives if agency_created == Some(true) => gate(surface, VisibilityDisposition::Visible, ""),
            IntelligenceSurface::Operations | IntelligenceSurface::Ciphers if agency_created == Some(true) => {
                match (row.operations_visible, row.ciphers_visible, surface) {
                    (Some(true), Some(false), IntelligenceSurface::Operations) => gate(surface, VisibilityDisposition::Visible, ""),
                    (Some(true), Some(false), IntelligenceSurface::Ciphers) => gate(surface, VisibilityDisposition::Hidden, "operations tab selected"),
                    (Some(false), Some(true), IntelligenceSurface::Ciphers) if products.cipher_controller_complete => gate(surface, VisibilityDisposition::Visible, ""),
                    (Some(false), Some(true), IntelligenceSurface::Operations) => gate(surface, VisibilityDisposition::Hidden, "ciphers tab selected"),
                    _ => gate(surface, VisibilityDisposition::Unavailable, "operation and cipher widget bits are not complementary or controller product is missing"),
                }
            }
            IntelligenceSurface::Networks if agency_created == Some(true) && network.is_some() => gate(surface, VisibilityDisposition::Visible, ""),
            IntelligenceSurface::Networks if agency_created == Some(true) => gate(surface, VisibilityDisposition::Unavailable, "same-frame D53 network source proof unavailable"),
            _ => gate(surface, VisibilityDisposition::Unavailable, "agency-created visibility producer unavailable"),
        })
        .collect()
}

fn gate(surface: IntelligenceSurface, disposition: VisibilityDisposition, reason: &str) -> IntelligenceSurfaceGate {
    IntelligenceSurfaceGate {
        surface,
        disposition,
        reason: (disposition != VisibilityDisposition::Visible).then(|| reason.into()),
    }
}

fn validate_gates(gates: &[IntelligenceSurfaceGate]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    if gates.len() != IntelligenceSurface::ALL.len()
        || gates.iter().any(|gate| {
            !seen.insert(gate.surface)
                || (gate.disposition == VisibilityDisposition::Visible && gate.reason.is_some())
                || (gate.disposition != VisibilityDisposition::Visible
                    && gate.reason.as_deref().is_none_or(str::is_empty))
        })
    {
        return Err("A30 seven-surface visibility set is incomplete".into());
    }
    Ok(())
}

fn push(row: VisibilityObservation) -> Result<(), String> {
    if row.frame_hour == 0
        || row.owner_tag_db_id == 0
        || row.producer_record_identity.iter().all(|byte| *byte == 0)
        || row.refresh_end_qpc_ticks == 0
    {
        return Err("A30 visibility observation identity is incomplete".into());
    }
    let mut rows = OBSERVATIONS
        .try_lock()
        .map_err(|_| "A30 visibility observation lock busy")?;
    if rows.len() >= 16 {
        return Err("A30 visibility observation bound exceeded".into());
    }
    rows.push(row);
    Ok(())
}

fn read_current_ui_mode(module_base: usize) -> Result<i32, String> {
    let mode_pointer = read_stable::<usize>(module_base.saturating_add(CURRENT_UI_MODE_POINTER_RVA))
        .filter(|pointer| *pointer != 0)
        .ok_or_else(|| "A30 current UI mode pointer is unavailable".to_string())?;
    read_stable::<i32>(mode_pointer)
        .ok_or_else(|| "A30 current UI mode is unavailable".to_string())
}

fn widget_visible(view: usize, field_offset: usize) -> Option<bool> {
    let widget = read_stable::<usize>(view.saturating_add(field_offset))?;
    if widget == 0 {
        return None;
    }
    read_stable::<u8>(widget.saturating_add(WIDGET_HIDDEN_BYTE))
        .map(|flags| flags & WIDGET_HIDDEN_BIT == 0)
}

fn read_stable<T: Copy + PartialEq>(address: usize) -> Option<T> {
    let first = read_value::<T>(address)?;
    let second = read_value::<T>(address)?;
    (first == second).then_some(first)
}

fn read_value<T: Copy>(address: usize) -> Option<T> {
    if address == 0 {
        return None;
    }
    let mut value = mem::MaybeUninit::<T>::uninit();
    let mut copied = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const _,
            value.as_mut_ptr().cast(),
            mem::size_of::<T>(),
            &mut copied,
        )
    };
    (ok != 0 && copied == mem::size_of::<T>()).then(|| unsafe { value.assume_init() })
}

fn decode_sha256(value: &str) -> Result<[u8; 32], String> {
    if !lower_sha256(value) {
        return Err("A30 visibility producer identity is not lowercase SHA-256".into());
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| "A30 visibility producer identity hex is invalid")?;
    }
    Ok(out)
}

fn lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value.bytes().any(|byte| byte != b'0')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}
