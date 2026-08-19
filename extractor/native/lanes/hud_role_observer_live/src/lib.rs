use portable_content_manifest::hud_role::{validate_runtime_root_name, validate_sha256};
pub use portable_content_manifest::hud_role::{
    AuthenticatedHudSurfaceClosure, EffectiveVfsHudRoleIndexDocument, ExactHudRoleResolver,
    HudContentObservation, HudRoleSnapshot, HudRuntimeRootIdentity, HudSemanticRole, HudSurface,
    HudSurfaceActivation, HudSurfaceActivationFrame, OmittedHudSurface, ResolvedHudRoleBinding,
    ResolvedHudSurface, ALL_HUD_SURFACES, CONSUMER_PREFIX, MAX_BINDINGS_PER_SURFACE,
    MAX_ROOTS_PER_SNAPSHOT, MAX_RUNTIME_ROOT_NAME_BYTES,
};
use portable_content_manifest::UsedContentKey;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(windows)]
pub mod a729_hook;

pub const DOMAIN_ID: u16 = 79;
pub const DOMAIN_NAME: &str = "hud_surface_activation_live";
pub const ACTIVATION_MAGIC: u32 = u32::from_le_bytes(*b"HRA1");
pub const ACTIVATION_SCHEMA_VERSION: u32 = 1;
pub const TOPBAR_ALERT_MAGIC: u32 = u32::from_le_bytes(*b"TBA1");
pub const TOPBAR_ALERT_SCHEMA_VERSION: u32 = 2;
pub const ACTIVE_ALERT_TYPE_COUNT: u16 = 0x52;
pub const MAX_ACTIVE_ALERT_ROWS: usize = 4_096;
const ACTIVE_ALERT_ROW_BYTES: usize = 28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AlertDedupIdentity {
    ZeroTuple = 0,
    CountryDbId = 1,
    CommandGroupOmitted = 2,
    OpaqueEngineObjectOmitted = 3,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveAlertRow {
    pub alert_type: u16,
    pub source_ordinal: u16,
    pub priority: i32,
    pub template_class: u32,
    pub dedup_identity: AlertDedupIdentity,
    pub country_target_db_id: Option<u32>,
    pub icon_present: bool,
    pub icon_frame: Option<u16>,
    pub display_order: i32,
}

pub fn alert_icon_frame(alert_type: u16) -> Option<u16> {
    let frame = match alert_type {
        0..=8 | 0x13 | 0x2b | 0x2d | 0x2e => 1,
        9 => 4,
        10 => 5,
        11 => 6,
        12 => 7,
        13 => 8,
        14 => 9,
        15 => 10,
        16 => 2,
        17 => 14,
        18 => 11,
        20 => 12,
        21 => 13,
        22 => 15,
        23 => 16,
        24 => 17,
        25 => 18,
        26 => 46,
        27 => 47,
        28 => 19,
        29 => 20,
        30 => 21,
        31 => 22,
        32 => 23,
        33 => 26,
        34 => 24,
        35 => 25,
        36 => 27,
        37 => 28,
        38 => 29,
        39 => 30,
        40 => 31,
        41 => 32,
        42 => 33,
        44 => 34,
        47 => 35,
        48 => 36,
        49 => 37,
        50 => 38,
        51 => 39,
        52 => 40,
        53 => 41,
        54 => 42,
        55 => 43,
        56 => 44,
        57 => 45,
        58 => 3,
        59 => 48,
        60 => 49,
        61 => 51,
        62 => 50,
        63 => 52,
        64 => 53,
        65 => 54,
        66 => 55,
        67 => 56,
        68 => 57,
        69 => 58,
        70 => 59,
        71 => 60,
        72 => 64,
        73 => 61,
        74 => 62,
        75 => 63,
        76 => 65,
        77 => 66,
        78 => 67,
        79 => 68,
        80 => 69,
        81 => 70,
        _ => return None,
    };
    Some(frame)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopbarAlertCounts {
    pub country_tag_db_id: u32,
    pub intel_available: i32,
    pub intel_prepared_or_completed: i32,
    pub market_non_delivering: i32,
    pub market_ongoing: i32,
    pub active_alerts: Vec<ActiveAlertRow>,
}

pub const TOPBAR_SETUP_RVA: u32 = 0x0188_BA30;
pub const MINIMAP_SETUP_RVA: u32 = 0x017B_9A80;
pub const MAPMODE_SETUP_RVA: u32 = 0x0156_3C80;
pub const THEATRE_SETUP_RVA: u32 = 0x0187_8E40;
pub const DESIGNER_REFRESH_RVA: u32 = 0x0177_97A0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawHudControllerRoot {
    pub sequence: u64,
    pub setup_rva: u32,
    pub surface: HudSurface,
    pub controller_ptr: usize,
    pub runtime_root_ptr: usize,
    pub runtime_root_name: String,
}

pub fn encode_activation_payload(frame: &HudSurfaceActivationFrame) -> Result<Vec<u8>, String> {
    validate_activation_frame(frame)?;
    let mut out = Vec::with_capacity(32 + frame.active_surfaces.len() * 24);
    out.extend_from_slice(&ACTIVATION_MAGIC.to_le_bytes());
    out.extend_from_slice(&ACTIVATION_SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&frame.status_flags.to_le_bytes());
    out.extend_from_slice(&frame.frame_hour.to_le_bytes());
    out.extend_from_slice(&frame.observation_sequence.to_le_bytes());
    out.extend_from_slice(&(frame.active_surfaces.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    for row in &frame.active_surfaces {
        out.push(surface_code(row.surface));
        out.push(0);
        out.extend_from_slice(&(row.runtime_root_name.len() as u16).to_le_bytes());
        out.extend_from_slice(row.runtime_root_name.as_bytes());
    }
    Ok(out)
}

pub fn encode_activation_payload_with_topbar_alerts(
    frame: &HudSurfaceActivationFrame,
    alerts: &TopbarAlertCounts,
) -> Result<Vec<u8>, String> {
    validate_topbar_alert_counts(alerts)?;
    let mut out = encode_activation_payload(frame)?;
    out.extend_from_slice(&TOPBAR_ALERT_MAGIC.to_le_bytes());
    out.extend_from_slice(&TOPBAR_ALERT_SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&alerts.country_tag_db_id.to_le_bytes());
    out.extend_from_slice(&alerts.intel_available.to_le_bytes());
    out.extend_from_slice(&alerts.intel_prepared_or_completed.to_le_bytes());
    out.extend_from_slice(&alerts.market_non_delivering.to_le_bytes());
    out.extend_from_slice(&alerts.market_ongoing.to_le_bytes());
    out.extend_from_slice(&(alerts.active_alerts.len() as u32).to_le_bytes());
    for row in &alerts.active_alerts {
        out.extend_from_slice(&row.alert_type.to_le_bytes());
        out.extend_from_slice(&row.source_ordinal.to_le_bytes());
        out.push(row.dedup_identity as u8);
        out.push(u8::from(row.icon_present));
        out.extend_from_slice(&row.icon_frame.unwrap_or(u16::MAX).to_le_bytes());
        out.extend_from_slice(&row.priority.to_le_bytes());
        out.extend_from_slice(&row.template_class.to_le_bytes());
        out.extend_from_slice(&row.display_order.to_le_bytes());
        out.extend_from_slice(&row.country_target_db_id.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    Ok(out)
}

pub fn decode_activation_payload(payload: &[u8]) -> Result<HudSurfaceActivationFrame, String> {
    decode_activation_payload_with_topbar_alerts(payload).map(|(frame, _)| frame)
}

pub fn decode_activation_payload_with_topbar_alerts(
    payload: &[u8],
) -> Result<(HudSurfaceActivationFrame, Option<TopbarAlertCounts>), String> {
    if payload.len() < 32 {
        return Err("HUD activation payload has an invalid length".into());
    }
    let u32_at = |at| u32::from_le_bytes(payload[at..at + 4].try_into().unwrap());
    let u64_at = |at| u64::from_le_bytes(payload[at..at + 8].try_into().unwrap());
    if u32_at(0) != ACTIVATION_MAGIC || u32_at(4) != ACTIVATION_SCHEMA_VERSION {
        return Err("HUD activation payload has an invalid magic/schema".into());
    }
    let count = u16::from_le_bytes(payload[28..30].try_into().unwrap()) as usize;
    if payload[30..32] != [0, 0] || count > ALL_HUD_SURFACES.len() {
        return Err("HUD activation payload has invalid count/reserved fields".into());
    }
    let mut active_surfaces = Vec::with_capacity(count);
    let mut cursor = 32usize;
    for _ in 0..count {
        if cursor.saturating_add(4) > payload.len() || payload[cursor + 1] != 0 {
            return Err("HUD activation row header is truncated or invalid".into());
        }
        let surface = surface_from_code(payload[cursor])?;
        let name_len =
            u16::from_le_bytes(payload[cursor + 2..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        let end = cursor
            .checked_add(name_len)
            .filter(|end| *end <= payload.len())
            .ok_or("HUD activation root name is truncated")?;
        let runtime_root_name = std::str::from_utf8(&payload[cursor..end])
            .map_err(|_| "HUD activation root name is not UTF-8")?
            .to_owned();
        active_surfaces.push(HudSurfaceActivation {
            surface,
            runtime_root_name,
        });
        cursor = end;
    }
    let topbar_alerts = if cursor == payload.len() {
        None
    } else {
        if payload.len().saturating_sub(cursor) < 32 || u32_at(cursor) != TOPBAR_ALERT_MAGIC {
            return Err("HUD activation payload has an invalid topbar-alert trailer".into());
        }
        let trailer_schema = u32_at(cursor + 4);
        if !matches!(trailer_schema, 1 | TOPBAR_ALERT_SCHEMA_VERSION) {
            return Err("HUD activation payload has an unsupported topbar-alert schema".into());
        }
        let row_count = u32_at(cursor + 28) as usize;
        if trailer_schema == 1 && row_count != 0 {
            return Err("HUD topbar-alert v1 trailer has nonzero reserved data".into());
        }
        let expected = 32usize
            .checked_add(if trailer_schema == 1 {
                0
            } else {
                row_count
                    .checked_mul(ACTIVE_ALERT_ROW_BYTES)
                    .ok_or("HUD active-alert row length overflow")?
            })
            .ok_or("HUD topbar-alert trailer length overflow")?;
        if payload.len().saturating_sub(cursor) != expected || row_count > MAX_ACTIVE_ALERT_ROWS {
            return Err("HUD topbar-alert trailer has an invalid row length".into());
        }
        let mut active_alerts = Vec::with_capacity(row_count);
        let mut row_cursor = cursor + 32;
        for _ in 0..row_count {
            let dedup_identity = match payload[row_cursor + 4] {
                0 => AlertDedupIdentity::ZeroTuple,
                1 => AlertDedupIdentity::CountryDbId,
                2 => AlertDedupIdentity::CommandGroupOmitted,
                3 => AlertDedupIdentity::OpaqueEngineObjectOmitted,
                _ => return Err("HUD active-alert row has an unknown dedup identity".into()),
            };
            let flags = payload[row_cursor + 5];
            if flags & !1 != 0 || u32_at(row_cursor + 24) != 0 {
                return Err("HUD active-alert row has invalid flags/reserved data".into());
            }
            let country = u32_at(row_cursor + 20);
            let icon_frame =
                u16::from_le_bytes(payload[row_cursor + 6..row_cursor + 8].try_into().unwrap());
            active_alerts.push(ActiveAlertRow {
                alert_type: u16::from_le_bytes(
                    payload[row_cursor..row_cursor + 2].try_into().unwrap(),
                ),
                source_ordinal: u16::from_le_bytes(
                    payload[row_cursor + 2..row_cursor + 4].try_into().unwrap(),
                ),
                priority: i32::from_le_bytes(
                    payload[row_cursor + 8..row_cursor + 12].try_into().unwrap(),
                ),
                template_class: u32_at(row_cursor + 12),
                dedup_identity,
                country_target_db_id: (country != 0).then_some(country),
                icon_present: flags & 1 != 0,
                icon_frame: (icon_frame != u16::MAX).then_some(icon_frame),
                display_order: i32::from_le_bytes(
                    payload[row_cursor + 16..row_cursor + 20]
                        .try_into()
                        .unwrap(),
                ),
            });
            row_cursor += ACTIVE_ALERT_ROW_BYTES;
        }
        let alerts = TopbarAlertCounts {
            country_tag_db_id: u32_at(cursor + 8),
            intel_available: i32::from_le_bytes(
                payload[cursor + 12..cursor + 16].try_into().unwrap(),
            ),
            intel_prepared_or_completed: i32::from_le_bytes(
                payload[cursor + 16..cursor + 20].try_into().unwrap(),
            ),
            market_non_delivering: i32::from_le_bytes(
                payload[cursor + 20..cursor + 24].try_into().unwrap(),
            ),
            market_ongoing: i32::from_le_bytes(
                payload[cursor + 24..cursor + 28].try_into().unwrap(),
            ),
            active_alerts,
        };
        validate_topbar_alert_counts(&alerts)?;
        Some(alerts)
    };
    let frame = HudSurfaceActivationFrame {
        frame_hour: u64_at(12),
        status_flags: u32_at(8),
        observation_sequence: u64_at(20),
        active_surfaces,
    };
    validate_activation_frame(&frame)?;
    Ok((frame, topbar_alerts))
}

fn validate_topbar_alert_counts(alerts: &TopbarAlertCounts) -> Result<(), String> {
    if alerts.country_tag_db_id == 0
        || [
            alerts.intel_available,
            alerts.intel_prepared_or_completed,
            alerts.market_non_delivering,
            alerts.market_ongoing,
        ]
        .into_iter()
        .any(|count| !(0..=1_000_000).contains(&count))
    {
        return Err("HUD topbar-alert counts are out of bounds".into());
    }
    if alerts.active_alerts.len() > MAX_ACTIVE_ALERT_ROWS {
        return Err("HUD active-alert snapshot exceeds its row bound".into());
    }
    let mut previous = None;
    for row in &alerts.active_alerts {
        let identity_valid = match row.dedup_identity {
            AlertDedupIdentity::ZeroTuple => row.country_target_db_id.is_none(),
            AlertDedupIdentity::CountryDbId => {
                matches!(row.alert_type, 1 | 2 | 5 | 6 | 7 | 0x2d | 0x2e)
                    && row.country_target_db_id.is_some_and(|id| id != 0)
            }
            AlertDedupIdentity::CommandGroupOmitted => {
                matches!(row.alert_type, 3 | 4 | 0x2b) && row.country_target_db_id.is_none()
            }
            AlertDedupIdentity::OpaqueEngineObjectOmitted => {
                matches!(row.alert_type, 0x16 | 0x3c) && row.country_target_db_id.is_none()
            }
        };
        let canonical = match previous {
            None => row.source_ordinal == 0,
            Some((prior_type, prior_ordinal)) => {
                (row.alert_type > prior_type && row.source_ordinal == 0)
                    || (row.alert_type == prior_type && row.source_ordinal == prior_ordinal + 1)
            }
        };
        if row.alert_type >= ACTIVE_ALERT_TYPE_COUNT
            || !identity_valid
            || !canonical
            || row.icon_frame
                != row
                    .icon_present
                    .then(|| alert_icon_frame(row.alert_type).expect("range checked above"))
            || (row.icon_present && row.display_order < 0)
            || (!row.icon_present && row.display_order != -1)
        {
            return Err("HUD active-alert rows are invalid or non-canonical".into());
        }
        previous = Some((row.alert_type, row.source_ordinal));
    }
    Ok(())
}

fn surface_code(surface: HudSurface) -> u8 {
    match surface {
        HudSurface::Topbar => 0,
        HudSurface::Minimap => 1,
        HudSurface::Mapmodes => 2,
        HudSurface::TheatreSelector => 3,
        HudSurface::DesignerCountryEquipment => 4,
        HudSurface::DesignerTank => 5,
        HudSurface::DesignerPlane => 6,
    }
}

fn surface_from_code(code: u8) -> Result<HudSurface, String> {
    ALL_HUD_SURFACES
        .get(code as usize)
        .copied()
        .ok_or_else(|| "HUD activation payload contains an unknown surface code".into())
}

fn validate_activations(surfaces: &[HudSurfaceActivation]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    let mut designer_count = 0;
    let mut previous = None;
    for row in surfaces {
        if !seen.insert(row.surface) {
            return Err("HUD activation repeats a surface".into());
        }
        if previous.is_some_and(|prior| prior >= row.surface) {
            return Err("HUD activation surfaces are not in canonical order".into());
        }
        previous = Some(row.surface);
        validate_runtime_root_name(&row.runtime_root_name)?;
        if matches!(
            row.surface,
            HudSurface::DesignerCountryEquipment
                | HudSurface::DesignerTank
                | HudSurface::DesignerPlane
        ) {
            designer_count += 1;
        }
    }
    if designer_count > 1 {
        return Err("HUD activation contains multiple active designer families".into());
    }
    Ok(())
}

fn validate_activation_frame(frame: &HudSurfaceActivationFrame) -> Result<(), String> {
    if frame.observation_sequence == 0 {
        return Err("HUD activation frame has zero sequence".into());
    }
    if frame.active_surfaces.is_empty() && frame.status_flags == 0 {
        return Err("successful HUD activation frame is empty".into());
    }
    validate_activations(&frame.active_surfaces)?;
    Ok(())
}

#[derive(Clone, Debug)]
pub struct EffectiveVfsHudRoleResolver {
    authority_manifest_sha256: String,
    closures: BTreeMap<(HudSurface, String), ResolvedHudSurface>,
}

impl EffectiveVfsHudRoleResolver {
    pub fn decode_authenticated_index(bytes: &[u8]) -> Result<Self, String> {
        let document: EffectiveVfsHudRoleIndexDocument = serde_json::from_slice(bytes)
            .map_err(|error| format!("parse HUD role index failed: {error}"))?;
        document.validate()?;
        Self::new(
            document.authority_manifest_sha256,
            document.winning_content,
            document.closures,
        )
    }

    pub fn new(
        authority_manifest_sha256: String,
        winning_content: impl IntoIterator<Item = UsedContentKey>,
        closures: impl IntoIterator<Item = AuthenticatedHudSurfaceClosure>,
    ) -> Result<Self, String> {
        validate_sha256(
            &authority_manifest_sha256,
            "HUD resolver authority manifest",
        )?;
        let mut winners = BTreeMap::new();
        for content in winning_content {
            content.validate()?;
            if content.effective_vfs_manifest_sha256 != authority_manifest_sha256 {
                return Err("HUD winning-content row names a different effective VFS".into());
            }
            let path = content.normalized_game_relative_path.clone();
            if winners.insert(path, content).is_some() {
                return Err("HUD winning-content inventory repeats a path".into());
            }
        }
        if winners.is_empty() {
            return Err("HUD winning-content inventory is empty".into());
        }

        let mut resolved = BTreeMap::new();
        for closure in closures {
            validate_runtime_root_name(&closure.runtime_root_name)?;
            validate_sha256(&closure.dependency_closure_sha256, "HUD dependency closure")?;
            if closure.complete_dependency_closure && !closure.missing_roles.is_empty() {
                return Err("complete HUD dependency closure reports missing roles".into());
            }
            if !closure.complete_dependency_closure || closure.bindings.is_empty() {
                continue;
            }
            if closure.bindings.len() > MAX_BINDINGS_PER_SURFACE {
                return Err("HUD surface role list exceeds its bound".into());
            }
            let mut consumers = BTreeSet::new();
            let mut gui_roots = 0usize;
            for binding in &closure.bindings {
                binding.content.validate()?;
                let expected = winners
                    .get(&binding.content.normalized_game_relative_path)
                    .ok_or("HUD role is absent from the effective-VFS winning inventory")?;
                if expected != &binding.content {
                    return Err("HUD role does not equal its effective-VFS winning row".into());
                }
                if matches!(binding.semantic_role, HudSemanticRole::GuiRoot { .. }) {
                    gui_roots += 1;
                }
                let consumer = binding.semantic_role.consumer(closure.surface)?;
                if !consumers.insert(consumer) {
                    return Err("HUD dependency closure repeats a semantic consumer".into());
                }
            }
            if gui_roots != 1 {
                return Err("HUD dependency closure must have exactly one GUI root".into());
            }
            let surface = closure.surface;
            let runtime_root_name = closure.runtime_root_name;
            if resolved
                .insert(
                    (surface, runtime_root_name),
                    ResolvedHudSurface {
                        complete: true,
                        bindings: closure.bindings,
                    },
                )
                .is_some()
            {
                return Err("HUD resolver repeats a runtime surface".into());
            }
        }
        Ok(Self {
            authority_manifest_sha256,
            closures: resolved,
        })
    }

    pub fn promoted_surfaces(&self) -> impl Iterator<Item = HudSurface> + '_ {
        self.closures.keys().map(|(surface, _)| *surface)
    }
}

impl ExactHudRoleResolver for EffectiveVfsHudRoleResolver {
    fn authority_manifest_sha256(&self) -> &str {
        &self.authority_manifest_sha256
    }

    fn resolve_surface(&self, root: &HudRuntimeRootIdentity) -> Option<ResolvedHudSurface> {
        (root.setup_rva == setup_rva(root.surface))
            .then(|| {
                self.closures
                    .get(&(root.surface, root.runtime_root_name.clone()))
                    .cloned()
            })
            .flatten()
    }
}

pub fn promote_snapshot<R: ExactHudRoleResolver>(
    roots: &[RawHudControllerRoot],
    resolver: &R,
) -> Result<HudRoleSnapshot, String> {
    validate_sha256(
        resolver.authority_manifest_sha256(),
        "HUD resolver authority manifest",
    )?;
    if roots.is_empty() || roots.len() > MAX_ROOTS_PER_SNAPSHOT {
        return Err("HUD root snapshot is empty or above its bound".into());
    }
    let mut prior_sequence = 0;
    let mut seen_surfaces = BTreeSet::new();
    let mut identities = Vec::with_capacity(roots.len());
    for root in roots {
        if root.sequence == 0
            || root.sequence <= prior_sequence
            || root.controller_ptr == 0
            || root.runtime_root_ptr == 0
            || root.setup_rva != setup_rva(root.surface)
            || !seen_surfaces.insert(root.surface)
        {
            return Err("HUD controller roots are invalid, duplicated, or out of order".into());
        }
        prior_sequence = root.sequence;
        validate_runtime_root_name(&root.runtime_root_name)?;
        identities.push(HudRuntimeRootIdentity {
            setup_rva: root.setup_rva,
            surface: root.surface,
            runtime_root_name: root.runtime_root_name.clone(),
        });
    }
    promote_identities(&identities, resolver)
}

pub fn promote_activation_frame<R: ExactHudRoleResolver>(
    frame: &HudSurfaceActivationFrame,
    resolver: &R,
) -> Result<HudRoleSnapshot, String> {
    validate_activation_frame(frame)?;
    if frame.status_flags != 0 {
        return Err("failed or badged HUD activation frame is not promotion-eligible".into());
    }
    let identities = frame
        .active_surfaces
        .iter()
        .map(|row| HudRuntimeRootIdentity {
            setup_rva: setup_rva(row.surface),
            surface: row.surface,
            runtime_root_name: row.runtime_root_name.clone(),
        })
        .collect::<Vec<_>>();
    promote_identities(&identities, resolver)
}

fn promote_identities<R: ExactHudRoleResolver>(
    roots: &[HudRuntimeRootIdentity],
    resolver: &R,
) -> Result<HudRoleSnapshot, String> {
    validate_sha256(
        resolver.authority_manifest_sha256(),
        "HUD resolver authority manifest",
    )?;
    if roots.is_empty() || roots.len() > MAX_ROOTS_PER_SNAPSHOT {
        return Err("HUD root identity snapshot is empty or above its bound".into());
    }
    let mut seen_surfaces = BTreeSet::new();
    let mut observations = Vec::new();
    let mut omitted_surfaces = Vec::new();
    for root in roots {
        if root.setup_rva != setup_rva(root.surface) || !seen_surfaces.insert(root.surface) {
            return Err("HUD root identities are invalid or duplicated".into());
        }
        validate_runtime_root_name(&root.runtime_root_name)?;
        let Some(resolved) = resolver.resolve_surface(root) else {
            omitted_surfaces.push(OmittedHudSurface {
                surface: root.surface,
                reason: "runtime root did not resolve through the live effective VFS".into(),
            });
            continue;
        };
        if !resolved.complete || resolved.bindings.is_empty() {
            omitted_surfaces.push(OmittedHudSurface {
                surface: root.surface,
                reason: "surface has an unknown or incomplete authored semantic role".into(),
            });
            continue;
        }
        if resolved.bindings.len() > MAX_BINDINGS_PER_SURFACE {
            return Err("HUD surface role list exceeds its bound".into());
        }
        let mut surface_rows = Vec::with_capacity(resolved.bindings.len());
        let mut has_gui_root = false;
        let mut seen_consumers = BTreeSet::new();
        for binding in resolved.bindings {
            binding.content.validate()?;
            if binding.content.effective_vfs_manifest_sha256 != resolver.authority_manifest_sha256()
            {
                return Err("HUD role names a different effective VFS".into());
            }
            has_gui_root |= matches!(binding.semantic_role, HudSemanticRole::GuiRoot { .. });
            let kind = binding.semantic_role.required_kind();
            let consumer = binding.semantic_role.consumer(root.surface)?;
            if !seen_consumers.insert(consumer.clone()) {
                return Err("HUD surface repeats a semantic consumer".into());
            }
            surface_rows.push(HudContentObservation {
                content_key: binding.content.normalized_game_relative_path.clone(),
                kind,
                content: binding.content,
                consumer,
            });
        }
        if !has_gui_root {
            omitted_surfaces.push(OmittedHudSurface {
                surface: root.surface,
                reason: "surface has no authenticated GUI root binding".into(),
            });
            continue;
        }
        observations.extend(surface_rows);
    }
    observations.sort_by(|left, right| {
        (&left.content_key, &left.consumer).cmp(&(&right.content_key, &right.consumer))
    });
    omitted_surfaces.sort_by_key(|row| row.surface);
    let mut activated_surfaces = roots.iter().map(|root| root.surface).collect::<Vec<_>>();
    activated_surfaces.sort();
    let snapshot = HudRoleSnapshot {
        schema: "extractor.hud-role-snapshot.v1".into(),
        authority_manifest_sha256: resolver.authority_manifest_sha256().into(),
        activated_surfaces,
        observations,
        omitted_surfaces,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

fn setup_rva(surface: HudSurface) -> u32 {
    match surface {
        HudSurface::Topbar => TOPBAR_SETUP_RVA,
        HudSurface::Minimap => MINIMAP_SETUP_RVA,
        HudSurface::Mapmodes => MAPMODE_SETUP_RVA,
        HudSurface::TheatreSelector => THEATRE_SETUP_RVA,
        HudSurface::DesignerCountryEquipment
        | HudSurface::DesignerTank
        | HudSurface::DesignerPlane => DESIGNER_REFRESH_RVA,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    fn h(ch: char) -> String {
        ch.to_string().repeat(64)
    }

    fn key(path: &str) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: h('a'),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 3,
            layer_kind: ContentLayerKind::Mod,
            layer_id: "mod/override.mod".into(),
            source_sha256: h('b'),
            payload_sha256: h('c'),
            payload_bytes: 17,
        }
    }

    struct Resolver {
        complete: bool,
    }

    impl ExactHudRoleResolver for Resolver {
        fn authority_manifest_sha256(&self) -> &str {
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }

        fn resolve_surface(&self, _: &HudRuntimeRootIdentity) -> Option<ResolvedHudSurface> {
            Some(ResolvedHudSurface {
                complete: self.complete,
                bindings: vec![
                    ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::GuiRoot {
                            window_or_root: "topbar".into(),
                            layout_identity_sha256: h('d'),
                        },
                        content: key("interface/modded_topbar.gui"),
                    },
                    ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::Sprite {
                            element: "player_flag".into(),
                            sprite_id: "GFX_modded_shield".into(),
                        },
                        content: key("interface/modded_topbar.gfx"),
                    },
                ],
            })
        }
    }

    fn root() -> RawHudControllerRoot {
        RawHudControllerRoot {
            sequence: 1,
            setup_rva: TOPBAR_SETUP_RVA,
            surface: HudSurface::Topbar,
            controller_ptr: 0x1000,
            runtime_root_ptr: 0x2000,
            runtime_root_name: "mod_topbar".into(),
        }
    }

    #[test]
    fn mod_override_roles_are_portable_and_no_vanilla_path_is_assumed() {
        let snapshot = promote_snapshot(&[root()], &Resolver { complete: true }).unwrap();
        assert_eq!(snapshot.observations.len(), 2);
        assert!(snapshot.omitted_surfaces.is_empty());
        assert!(snapshot
            .observations
            .iter()
            .all(|row| row.consumer.starts_with("hud_role/v1/topbar/")));
        assert!(snapshot
            .observations
            .iter()
            .all(|row| row.content.layer_id == "mod/override.mod"));
    }

    #[test]
    fn incomplete_surface_is_omitted_as_a_unit() {
        let snapshot = promote_snapshot(&[root()], &Resolver { complete: false }).unwrap();
        assert!(snapshot.observations.is_empty());
        assert_eq!(snapshot.omitted_surfaces.len(), 1);
        snapshot.validate().unwrap();
    }

    #[test]
    fn snapshot_validation_rejects_partition_bypasses() {
        let valid = promote_snapshot(&[root()], &Resolver { complete: true }).unwrap();
        valid.validate().unwrap();

        let mut overlap = valid.clone();
        overlap.omitted_surfaces.push(OmittedHudSurface {
            surface: HudSurface::Topbar,
            reason: "fabricated omission".into(),
        });
        assert!(overlap.validate().is_err());

        let mut incomplete = valid.clone();
        incomplete.activated_surfaces.push(HudSurface::Minimap);
        assert!(incomplete.validate().is_err());

        let mut duplicate = promote_snapshot(&[root()], &Resolver { complete: false }).unwrap();
        duplicate
            .omitted_surfaces
            .push(duplicate.omitted_surfaces[0].clone());
        assert!(duplicate.validate().is_err());

        let mut value = serde_json::to_value(&valid).unwrap();
        value.as_object_mut().unwrap().remove("activated_surfaces");
        assert!(serde_json::from_value::<HudRoleSnapshot>(value).is_err());
    }

    #[test]
    fn wrong_hook_root_and_foreign_vfs_fail_closed() {
        let mut wrong = root();
        wrong.setup_rva = MINIMAP_SETUP_RVA;
        assert!(promote_snapshot(&[wrong], &Resolver { complete: true }).is_err());
        struct Foreign;
        impl ExactHudRoleResolver for Foreign {
            fn authority_manifest_sha256(&self) -> &str {
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            }
            fn resolve_surface(&self, root: &HudRuntimeRootIdentity) -> Option<ResolvedHudSurface> {
                Resolver { complete: true }.resolve_surface(root)
            }
        }
        assert!(promote_snapshot(&[root()], &Foreign).is_err());
    }

    #[test]
    fn concrete_resolver_accepts_only_exact_mod_winner_and_complete_closure() {
        let gui = key("interface/modded_topbar.gui");
        let gfx = key("interface/modded_topbar.gfx");
        let resolver = EffectiveVfsHudRoleResolver::new(
            h('a'),
            [gui.clone(), gfx.clone()],
            [AuthenticatedHudSurfaceClosure {
                surface: HudSurface::Topbar,
                runtime_root_name: "mod_topbar".into(),
                complete_dependency_closure: true,
                dependency_closure_sha256: h('d'),
                missing_roles: vec![],
                bindings: vec![
                    ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::GuiRoot {
                            window_or_root: "mod_topbar".into(),
                            layout_identity_sha256: h('e'),
                        },
                        content: gui,
                    },
                    ResolvedHudRoleBinding {
                        semantic_role: HudSemanticRole::Sprite {
                            element: "flag".into(),
                            sprite_id: "GFX_mod_flag".into(),
                        },
                        content: gfx,
                    },
                ],
            }],
        )
        .unwrap();
        let snapshot = promote_snapshot(&[root()], &resolver).unwrap();
        assert_eq!(
            resolver.promoted_surfaces().collect::<Vec<_>>(),
            vec![HudSurface::Topbar]
        );
        assert_eq!(snapshot.observations.len(), 2);
        assert!(snapshot
            .observations
            .iter()
            .all(|row| row.content.winning_layer_ordinal == 3));
    }

    #[test]
    fn concrete_resolver_rejects_shadowed_or_unauthenticated_content() {
        let winner = key("interface/overridden.gui");
        let mut shadowed = winner.clone();
        shadowed.winning_layer_ordinal = 0;
        shadowed.layer_id = "base".into();
        assert!(EffectiveVfsHudRoleResolver::new(
            h('a'),
            [winner],
            [AuthenticatedHudSurfaceClosure {
                surface: HudSurface::Topbar,
                runtime_root_name: "mod_topbar".into(),
                complete_dependency_closure: true,
                dependency_closure_sha256: h('d'),
                missing_roles: vec![],
                bindings: vec![ResolvedHudRoleBinding {
                    semantic_role: HudSemanticRole::GuiRoot {
                        window_or_root: "mod_topbar".into(),
                        layout_identity_sha256: h('e'),
                    },
                    content: shadowed,
                }],
            }],
        )
        .is_err());
    }

    #[test]
    fn designer_family_is_a_runtime_role_not_a_path_inference() {
        let gui = key("interface/any_winning_name.gui");
        let resolver = EffectiveVfsHudRoleResolver::new(
            h('a'),
            [gui.clone()],
            [AuthenticatedHudSurfaceClosure {
                surface: HudSurface::DesignerTank,
                runtime_root_name: "runtime_observed_root".into(),
                complete_dependency_closure: true,
                dependency_closure_sha256: h('d'),
                missing_roles: vec![],
                bindings: vec![ResolvedHudRoleBinding {
                    semantic_role: HudSemanticRole::GuiRoot {
                        window_or_root: "runtime_observed_root".into(),
                        layout_identity_sha256: h('e'),
                    },
                    content: gui,
                }],
            }],
        )
        .unwrap();
        let designer = RawHudControllerRoot {
            sequence: 9,
            setup_rva: DESIGNER_REFRESH_RVA,
            surface: HudSurface::DesignerTank,
            controller_ptr: 0x3000,
            runtime_root_ptr: 0x4000,
            runtime_root_name: "runtime_observed_root".into(),
        };
        let snapshot = promote_snapshot(&[designer], &resolver).unwrap();
        assert_eq!(snapshot.observations.len(), 1);
        assert!(snapshot.observations[0]
            .consumer
            .starts_with("hud_role/v1/designer/tank/gui/"));
        assert_eq!(
            snapshot.observations[0].content_key,
            "interface/any_winning_name.gui"
        );
    }

    #[test]
    fn pointer_free_activation_carrier_roundtrips_fixed_and_one_designer_surface() {
        let frame = HudSurfaceActivationFrame {
            frame_hour: 42,
            status_flags: 0,
            observation_sequence: 9,
            active_surfaces: vec![
                activation(HudSurface::Topbar, "mod_topbar"),
                activation(HudSurface::Minimap, "mod_minimap"),
                activation(HudSurface::Mapmodes, "mod_mapmodes"),
                activation(HudSurface::TheatreSelector, "mod_theatre"),
                activation(HudSurface::DesignerPlane, "mod_designer"),
            ],
        };
        let bytes = encode_activation_payload(&frame).unwrap();
        assert!(bytes.len() > 36);
        assert_eq!(decode_activation_payload(&bytes).unwrap(), frame);
        assert!(!bytes
            .windows(std::mem::size_of::<usize>())
            .any(|row| { row == 0x0000_0001_4000_0000usize.to_le_bytes().as_slice() }));
    }

    #[test]
    fn latest_clean_activation_promotes_without_reconstructed_process_pointers() {
        let frame = HudSurfaceActivationFrame {
            frame_hour: 42,
            status_flags: 0,
            observation_sequence: 9,
            active_surfaces: vec![activation(HudSurface::Topbar, "mod_topbar")],
        };
        let snapshot = promote_activation_frame(&frame, &Resolver { complete: true }).unwrap();
        assert_eq!(snapshot.activated_surfaces, vec![HudSurface::Topbar]);
        assert_eq!(snapshot.observations.len(), 2);
        snapshot.validate().unwrap();

        let mut badged = frame;
        badged.status_flags = 1;
        assert!(promote_activation_frame(&badged, &Resolver { complete: true }).is_err());
    }

    #[test]
    fn activation_carrier_rejects_multiple_designer_families_and_unknown_bits() {
        let bad = HudSurfaceActivationFrame {
            frame_hour: 1,
            status_flags: 0,
            observation_sequence: 1,
            active_surfaces: vec![
                activation(HudSurface::DesignerTank, "tank"),
                activation(HudSurface::DesignerPlane, "plane"),
            ],
        };
        assert!(encode_activation_payload(&bad).is_err());
        let good = HudSurfaceActivationFrame {
            frame_hour: 1,
            status_flags: 0,
            observation_sequence: 1,
            active_surfaces: vec![activation(HudSurface::Topbar, "topbar")],
        };
        let mut bytes = encode_activation_payload(&good).unwrap();
        bytes[32] = 0xff;
        assert!(decode_activation_payload(&bytes).is_err());
    }

    fn activation(surface: HudSurface, runtime_root_name: &str) -> HudSurfaceActivation {
        HudSurfaceActivation {
            surface,
            runtime_root_name: runtime_root_name.into(),
        }
    }
}
