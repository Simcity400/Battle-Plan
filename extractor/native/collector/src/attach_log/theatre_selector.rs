use super::*;

use theatre_selector_live_lane as lane;
use theatre_selector_live_lane::SectionRejection as Rejection;

pub(super) struct CompactTheatreSelectorRead {
    pub(super) payload: Vec<u8>,
    pub(super) observed_sections: u32,
    pub(super) observed_theatres: u32,
    pub(super) observed_members: u32,
    pub(super) read_time_us: u64,
    pub(super) status_flags: u32,
    pub(super) diagnostic_units: Vec<CompactDomainPerfDiagnosticUnit>,
}

fn frame_error(message: impl Into<String>) -> lane::CarrierError {
    lane::CarrierError::Source(message.into())
}

const NO_CONTENT_IDENTITY: &str = "no authenticated theatre-selector content identity";

fn raw_vector(
    owner: usize,
    data_off: usize,
    capacity_off: usize,
    count_off: usize,
) -> Result<Vec<u64>, Rejection> {
    let data_addr = owner
        .checked_add(data_off)
        .ok_or(Rejection::SourceUnreadable)?;
    let capacity_addr = owner
        .checked_add(capacity_off)
        .ok_or(Rejection::SourceUnreadable)?;
    let count_addr = owner
        .checked_add(count_off)
        .ok_or(Rejection::SourceUnreadable)?;
    let data = read_u64_stable(data_addr).ok_or(Rejection::SourceUnreadable)? as usize;
    let capacity = read_u32_stable(capacity_addr).ok_or(Rejection::SourceUnreadable)?;
    let count = read_u32_stable(count_addr).ok_or(Rejection::SourceUnreadable)?;
    if count > capacity
        || count > MAX_SELECTOR_ROWS_PER_VECTOR
        || capacity > MAX_SELECTOR_ROWS_PER_VECTOR.saturating_mul(4)
    {
        return Err(Rejection::BoundExceeded);
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    if !is_map_control_heap_pointer(data) {
        return Err(Rejection::SourceUnreadable);
    }
    let bytes_len = (count as usize)
        .checked_mul(8)
        .ok_or(Rejection::BoundExceeded)?;
    let first = read_bytes(data, bytes_len).ok_or(Rejection::SourceUnreadable)?;
    let second = read_bytes(data, bytes_len).ok_or(Rejection::SourceUnreadable)?;
    if first != second
        || read_u64_stable(data_addr).map(|value| value as usize) != Some(data)
        || read_u32_stable(capacity_addr) != Some(capacity)
        || read_u32_stable(count_addr) != Some(count)
    {
        return Err(Rejection::IncompleteWalk);
    }
    (0..count as usize)
        .map(|index| read_u64_from_slice(&first, index * 8).ok_or(Rejection::SourceUnreadable))
        .collect()
}

fn ptr_vector(
    owner: usize,
    data_off: usize,
    capacity_off: usize,
    count_off: usize,
) -> Result<Vec<usize>, Rejection> {
    raw_vector(owner, data_off, capacity_off, count_off)?
        .into_iter()
        .map(|value| {
            let pointer = value as usize;
            if is_map_control_heap_pointer(pointer) {
                Ok(pointer)
            } else {
                Err(Rejection::SourceUnreadable)
            }
        })
        .collect()
}

fn army_orders_visibility(
    army: usize,
    border_conflict_raw: u64,
) -> Result<lane::ArmyOrdersVisibility, Rejection> {
    let training_visible = read_u8_stable(
        army.checked_add(ARMY_OFF_TRAINING_STATE)
            .ok_or(Rejection::SourceUnreadable)?,
    )
    .ok_or(Rejection::SourceUnreadable)?
        != 0;
    let stop_at_max_raw = read_u8_stable(
        army.checked_add(ARMY_OFF_STOP_AT_MAX_STATE)
            .ok_or(Rejection::SourceUnreadable)?,
    )
    .ok_or(Rejection::SourceUnreadable)?;
    Ok(orders_visibility_from_raw(
        training_visible,
        stop_at_max_raw,
        border_conflict_raw,
    ))
}

pub(super) fn orders_visibility_from_raw(
    training_visible: bool,
    stop_at_max_raw: u8,
    border_conflict_raw: u64,
) -> lane::ArmyOrdersVisibility {
    lane::ArmyOrdersVisibility {
        training_visible,
        stop_at_max_visible: training_visible && stop_at_max_raw != 0,
        border_conflict_visible: !training_visible && border_conflict_raw != 0,
    }
}

/// The witnessed identity of an orders group, keeping the class that
/// `validate_order_container_body` discards: `CArmy` and `CArmyGroup` share
/// `ARMY_TYPE` and are told apart only by vtable.
fn orders_group_identity(base: usize, object: usize) -> Result<(usize, u32, u32), Rejection> {
    if let Some(id) = validate_army_body(base, object) {
        return Ok((ARMY_VTABLE_RVA, ARMY_TYPE, id));
    }
    validate_oob_body(base, object, AG_VTABLE_RVA, ARMY_TYPE)
        .map(|id| (AG_VTABLE_RVA, ARMY_TYPE, id))
        .ok_or(Rejection::IdentityAuthenticationFailed)
}

/// Enrol an object in the witness set. Re-authenticating one address to a
/// DIFFERENT class or id means it changed under the walk, so the disagreement
/// refuses rather than letting the later tuple overwrite the earlier one.
pub(super) fn enrol(
    witnesses: &mut BTreeMap<usize, (usize, u32, u32)>,
    object: usize,
    identity: (usize, u32, u32),
) -> Result<(), Rejection> {
    match witnesses.insert(object, identity) {
        Some(previous) if previous != identity => Err(Rejection::IdentityAuthenticationFailed),
        _ => Ok(()),
    }
}

fn is_army_group(orders_group: usize) -> Result<bool, Rejection> {
    Ok(read_u8_stable(
        orders_group
            .checked_add(ORDERS_GROUP_OFF_IS_ARMY_GROUP)
            .ok_or(Rejection::SourceUnreadable)?,
    )
    .ok_or(Rejection::SourceUnreadable)?
        != 0)
}

fn army_border_conflict_state(army: usize) -> Result<u64, Rejection> {
    read_u64_stable(
        army.checked_add(ARMY_OFF_BORDER_CONFLICT_STATE)
            .ok_or(Rejection::SourceUnreadable)?,
    )
    .ok_or(Rejection::SourceUnreadable)
}

fn display_name(object: usize, offset: usize) -> Result<String, Rejection> {
    let address = object
        .checked_add(offset)
        .ok_or(Rejection::DisplayNameUnreadable)?;
    let snapshot = read_stable_msvc_string_snapshot(address, lane::MAX_DISPLAY_NAME_BYTES)
        .ok_or(Rejection::DisplayNameUnreadable)?;
    let name = String::from_utf8(snapshot.bytes).map_err(|_| Rejection::DisplayNameUnreadable)?;
    if name.is_empty() {
        return Err(Rejection::DisplayNameUnreadable);
    }
    Ok(name)
}

fn reference_id(
    object: usize,
    module_base: usize,
    vtable_rva: usize,
    expected_type: Option<u32>,
) -> Result<(u32, u64), Rejection> {
    if !is_map_control_heap_pointer(object)
        || read_u64_stable(object)
            != module_base
                .checked_add(vtable_rva)
                .and_then(|value| u64::try_from(value).ok())
    {
        return Err(Rejection::IdentityAuthenticationFailed);
    }
    let handle = read_u64_stable(
        object
            .checked_add(0x08)
            .ok_or(Rejection::IdentityAuthenticationFailed)?,
    )
    .ok_or(Rejection::IdentityAuthenticationFailed)?;
    let type_raw = handle as u32;
    let object_id = (handle >> 32) as u32;
    if object_id == 0
        || object_id > MAX_OOB_COMMAND_ID
        || type_raw == 0
        || expected_type.is_some_and(|expected| expected != type_raw)
    {
        return Err(Rejection::IdentityAuthenticationFailed);
    }
    Ok((object_id, handle))
}

fn alert_categories_by_theatre(
    module_base: usize,
) -> Result<BTreeMap<usize, Vec<u32>>, lane::CarrierError> {
    let global = read_u64_stable(
        module_base
            .checked_add(ALERT_MANAGER_GLOBAL_PTR_RVA)
            .ok_or_else(|| frame_error("alert global address overflow"))?,
    )
    .ok_or_else(|| frame_error("alert global is torn"))? as usize;
    let manager = read_u64_stable(
        global
            .checked_add(ALERT_MANAGER_FROM_GLOBAL)
            .ok_or_else(|| frame_error("alert manager address overflow"))?,
    )
    .ok_or_else(|| frame_error("alert manager is torn"))? as usize;
    if !is_map_control_heap_pointer(global) || !is_map_control_heap_pointer(manager) {
        return Err(frame_error("alert root is not an admitted heap pointer"));
    }
    let mut index: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
    for category in 0..ALERT_MANAGER_CATEGORY_COUNT {
        let vector = manager
            .checked_add(ALERT_MANAGER_THEATRE_BASE)
            .and_then(|base| base.checked_add(category * ALERT_MANAGER_CATEGORY_STRIDE))
            .ok_or_else(|| frame_error("alert category vector overflow"))?;
        let data = read_u64_stable(vector)
            .ok_or_else(|| frame_error("alert category data is torn"))? as usize;
        let capacity = read_u32_stable(vector + 8)
            .ok_or_else(|| frame_error("alert category capacity is torn"))?;
        let count = read_u32_stable(vector + 12)
            .ok_or_else(|| frame_error("alert category count is torn"))?;
        if count > capacity || count > MAX_SELECTOR_ROWS_PER_VECTOR {
            return Err(frame_error("alert category vector violates exact bounds"));
        }
        if count > 0 && !is_map_control_heap_pointer(data) {
            return Err(frame_error("alert category data pointer is invalid"));
        }
        for entry in 0..count as usize {
            let record = data
                .checked_add(
                    entry
                        .checked_mul(ALERT_THEATRE_RECORD_STRIDE)
                        .ok_or_else(|| frame_error("alert record index overflow"))?,
                )
                .ok_or_else(|| frame_error("alert record address overflow"))?;
            let flags = read_u32_stable(record + ALERT_THEATRE_RECORD_ACTIVE)
                .ok_or_else(|| frame_error("alert record flags are torn"))?;
            let owner = read_u64_stable(record + ALERT_THEATRE_RECORD_THEATRE)
                .ok_or_else(|| frame_error("alert record theatre is torn"))?
                as usize;
            if flags & 0xff == 0 && owner != 0 {
                let categories = index.entry(owner).or_default();
                if categories.last() != Some(&(category as u32)) {
                    categories.push(category as u32);
                }
            }
        }
        if read_u64_stable(vector).map(|value| value as usize) != Some(data)
            || read_u32_stable(vector + 8) != Some(capacity)
            || read_u32_stable(vector + 12) != Some(count)
        {
            return Err(frame_error("alert category changed during walk"));
        }
    }
    Ok(index)
}

pub(super) fn content_identity(
    control: &CollectorControl,
) -> Result<lane::ContentIdentity, lane::CarrierError> {
    content_identity_with(control, engine_localization_digest, engine_content_roster)
}

pub(super) fn content_identity_with(
    control: &CollectorControl,
    localization_digest: impl Fn(&str) -> Result<[u8; 32], String>,
    engine_roster: impl Fn() -> Result<lane::EngineContentRoster, String>,
) -> Result<lane::ContentIdentity, lane::CarrierError> {
    let authority = control
        .d98_theatre_selector_content_identity
        .as_ref()
        .ok_or(lane::CarrierError::Source(NO_CONTENT_IDENTITY.into()))?;
    if !control.d98_theatre_selector_content_identity_valid {
        return Err(frame_error(
            "theatre-selector content identity is malformed",
        ));
    }
    let mut ordered_layers = Vec::with_capacity(authority.layers.len());
    for (index, layer) in authority.layers.iter().enumerate() {
        let layer_kind = match layer.layer_kind.as_str() {
            "base" => lane::ContentLayerKind::Base,
            "integrated_dlc" => lane::ContentLayerKind::IntegratedDlc,
            "dlc" => lane::ContentLayerKind::Dlc,
            "mod" => lane::ContentLayerKind::Mod,
            _ => return Err(frame_error("content layer kind is unknown")),
        };
        ordered_layers.push(lane::ContentLayer {
            ordinal: u32::try_from(index)
                .map_err(|_| frame_error("content layer ordinal overflow"))?,
            layer_kind,
            layer_id: layer.layer_id.clone(),
            content_sha256: sha256_hex_to_bytes(&layer.sha256)
                .ok_or_else(|| frame_error("content layer hash is not a sha256"))?,
        });
    }
    Ok(lane::ContentIdentity {
        language: authority.language.clone(),
        localization_content_sha256: localization_digest(&authority.language)
            .map_err(|error| frame_error(format!("engine localisation: {error}")))?,
        ordered_layers,
        engine_roster: engine_roster()
            .map_err(|error| frame_error(format!("engine mod/DLC roster: {error}")))?,
    })
}

fn engine_content_roster() -> Result<lane::EngineContentRoster, String> {
    let module_base = unsafe { crate::attach_log::GetModuleHandleW(core::ptr::null()) } as u64;
    if module_base == 0 {
        return Err("GetModuleHandleW(NULL) failed".to_string());
    }
    let read = |image: &crate::engine_localisation_reader::InProcessEngineImage| {
        lane::engine_modset_provenance::read_engine_modset_provenance_once(image, module_base)
    };
    let first = read(&crate::engine_localisation_reader::InProcessEngineImage::new())?;
    let second = read(&crate::engine_localisation_reader::InProcessEngineImage::new())?;
    if first != second {
        return Err("engine mod/DLC roster changed between two consecutive reads".to_string());
    }
    wire_roster(&first)
}

fn wire_roster(
    provenance: &lane::engine_modset_provenance::EngineModsetProvenance,
) -> Result<lane::EngineContentRoster, String> {
    use lane::engine_modset_provenance::EngineContentKind;
    Ok(lane::EngineContentRoster {
        requested_mods: provenance.requested_mods.clone(),
        requested_disabled_dlcs: provenance.requested_disabled_dlcs.clone(),
        discovered: provenance
            .discovered
            .iter()
            .map(|entry| lane::EngineRosterEntry {
                kind: match entry.kind {
                    EngineContentKind::Dlc => lane::ContentLayerKind::Dlc,
                    EngineContentKind::Mod => lane::ContentLayerKind::Mod,
                },
                descriptor_entry: entry.descriptor_entry.clone(),
                name: entry.name.clone(),
                selected: entry.selected,
                mounted: entry.mounted,
            })
            .collect(),
        roster_sha256: sha256_hex_to_bytes(&provenance.provenance_sha256)
            .ok_or("engine roster digest is not a sha256")?,
    })
}

fn engine_localization_digest(expected_language: &str) -> Result<[u8; 32], String> {
    static CACHE: std::sync::Mutex<Option<(String, [u8; 32])>> = std::sync::Mutex::new(None);
    if let Ok(cache) = CACHE.lock() {
        if let Some((language, digest)) = cache.as_ref() {
            if language == expected_language {
                return Ok(*digest);
            }
        }
    }
    let module_base = unsafe { crate::attach_log::GetModuleHandleW(core::ptr::null()) } as u64;
    if module_base == 0 {
        return Err("GetModuleHandleW(NULL) failed".to_string());
    }
    let read = |image: &crate::engine_localisation_reader::InProcessEngineImage| {
        lane::engine_localisation_provenance::read_engine_localisation_provenance_once(
            image,
            module_base,
            expected_language,
        )
    };
    let first = read(&crate::engine_localisation_reader::InProcessEngineImage::new())?;
    let second = read(&crate::engine_localisation_reader::InProcessEngineImage::new())?;
    if first != second {
        return Err("engine localisation changed between two consecutive reads".to_string());
    }
    let digest = sha256_hex_to_bytes(&first.provenance_sha256)
        .ok_or("engine localisation digest is not a sha256")?;
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((expected_language.to_string(), digest));
    }
    Ok(digest)
}

fn sha256_hex_to_bytes(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

struct AllCountrySource<'a> {
    state: &'a CollectorState,
    roster: Vec<Domain51CountryRoot>,
    wings_by_owner: BTreeMap<u32, Vec<CompactAirWingRow>>,
    alerts: BTreeMap<usize, Vec<u32>>,
    identity: lane::ContentIdentity,
}

impl AllCountrySource<'_> {
    fn country_root(&self, owner_tag_db_id: u32) -> Result<Domain51CountryRoot, Rejection> {
        let mut found = None;
        for row in &self.roster {
            if row.tag_db_id == owner_tag_db_id {
                if found.is_some() {
                    return Err(Rejection::DuplicateIdentity);
                }
                found = Some(*row);
            }
        }
        found.ok_or(Rejection::IncompleteWalk)
    }

    fn land(
        &self,
        country: Domain51CountryRoot,
        out: &mut Vec<lane::TheatreRow>,
    ) -> Result<(), Rejection> {
        use lane::{MemberRow, ObjectKind, TheatreRow};
        let theatres = ptr_vector(
            country.object_ptr,
            COUNTRY_THEATRE_DATA,
            COUNTRY_THEATRE_CAPACITY,
            COUNTRY_THEATRE_COUNT,
        )?;
        let mut seen_theatres = BTreeSet::new();
        let mut theatre_order = 0u32;
        for owning_theatre in theatres {
            let (owning_theatre_id, _) = reference_id(
                owning_theatre,
                self.state.base,
                THEATRE_VTABLE_RVA,
                Some(OOB_TYPE_THEATRE),
            )?;
            for theater_group in ptr_vector(
                owning_theatre,
                THEATRE_OFF_THEATER_GROUP_DATA,
                THEATRE_OFF_THEATER_GROUP_CAPACITY,
                THEATRE_OFF_THEATER_GROUP_COUNT,
            )? {
                let (theatre_id, _) = reference_id(
                    theater_group,
                    self.state.base,
                    THEATER_GROUP_VTABLE_RVA,
                    Some(OOB_TYPE_THEATER_GROUP),
                )?;
                if !seen_theatres.insert(theatre_id) {
                    return Err(Rejection::DuplicateIdentity);
                }
                // Every object this row is built from, re-checked once after
                // the last read of any of them. A guard only proves the class
                // at the instant it runs, and the `==` tests below would still
                // pass against an address freed and reused mid-walk.
                let mut witnesses: BTreeMap<usize, (usize, u32, u32)> = BTreeMap::new();
                enrol(
                    &mut witnesses,
                    owning_theatre,
                    (THEATRE_VTABLE_RVA, OOB_TYPE_THEATRE, owning_theatre_id),
                )?;
                enrol(
                    &mut witnesses,
                    theater_group,
                    (
                        THEATER_GROUP_VTABLE_RVA,
                        OOB_TYPE_THEATER_GROUP,
                        theatre_id,
                    ),
                )?;
                if read_u64_stable(theater_group + THEATER_GROUP_OFF_THEATRE)
                    .map(|value| value as usize)
                    != Some(owning_theatre)
                {
                    return Err(Rejection::OrphanRow);
                }
                let army_rows = ptr_vector(
                    theater_group,
                    THEATER_GROUP_OFF_ARMY_DATA,
                    THEATER_GROUP_OFF_ARMY_CAPACITY,
                    THEATER_GROUP_OFF_ARMY_COUNT,
                )?;
                let army_set = army_rows.iter().copied().collect::<BTreeSet<_>>();
                let mut border_conflict_by_army = BTreeMap::new();
                for &army in &army_rows {
                    // container guard is the union; the row sites below narrow it.
                    // Enrolling BEFORE the field read binds the published value
                    // to the identity the sweep will re-check.
                    enrol(
                        &mut witnesses,
                        army,
                        orders_group_identity(self.state.base, army)?,
                    )?;
                    border_conflict_by_army.insert(army, army_border_conflict_state(army)?);
                }
                let delete_eligible = border_conflict_by_army.values().any(|&value| value != 0);
                let groups = ptr_vector(
                    owning_theatre,
                    THEATRE_OFF_ARMY_GROUP_DATA,
                    THEATRE_OFF_ARMY_GROUP_CAPACITY,
                    THEATRE_OFF_ARMY_GROUP_COUNT,
                )?;
                let mut members = Vec::new();
                let mut grouped_armies = BTreeSet::new();
                let mut group_order = 0u32;
                for group in groups {
                    let group_id =
                        validate_oob_body(self.state.base, group, AG_VTABLE_RVA, ARMY_TYPE)
                            .ok_or(Rejection::IdentityAuthenticationFailed)?;
                    if !is_army_group(group)? {
                        return Err(Rejection::IdentityAuthenticationFailed);
                    }
                    enrol(&mut witnesses, group, (AG_VTABLE_RVA, ARMY_TYPE, group_id))?;
                    let backref = read_u64_stable(group + ARMY_OFF_THEATRE_GROUP)
                        .ok_or(Rejection::SourceUnreadable)?
                        as usize;
                    if backref != theater_group {
                        reference_id(
                            backref,
                            self.state.base,
                            THEATER_GROUP_VTABLE_RVA,
                            Some(OOB_TYPE_THEATER_GROUP),
                        )?;
                        continue;
                    }
                    members.push(MemberRow {
                        kind: ObjectKind::ArmyGroup,
                        object_id: group_id,
                        parent_kind: ObjectKind::LandTheatre,
                        parent_id: theatre_id,
                        engine_order: group_order,
                        reserve_navy: false,
                        display_name: None,
                        name_provenance: None,
                        priority_ordinal: lane::PriorityOrdinal::Unavailable,
                        orders_visibility: Some(army_orders_visibility(
                            group,
                            army_border_conflict_state(group)?,
                        )?),
                    });
                    group_order = group_order.saturating_add(1);
                    let children = ptr_vector(
                        group,
                        ARMY_GROUP_OFF_MEMBER_DATA,
                        ARMY_GROUP_OFF_MEMBER_CAPACITY,
                        ARMY_GROUP_OFF_MEMBER_COUNT,
                    )?;
                    for (child_order, army) in children.into_iter().enumerate() {
                        // CArmy, not CArmyGroup: a nested army group has no Army row
                        // shape on the wire, and this vtable refuses it by name.
                        let army_id = validate_army_body(self.state.base, army)
                            .ok_or(Rejection::IdentityAuthenticationFailed)?;
                        enrol(&mut witnesses, army, (ARMY_VTABLE_RVA, ARMY_TYPE, army_id))?;
                        if !army_set.contains(&army) {
                            return Err(Rejection::OrphanRow);
                        }
                        if read_u64_stable(army + ARMY_OFF_AG_PARENT).map(|value| value as usize)
                            != Some(group)
                        {
                            return Err(Rejection::OrphanRow);
                        }
                        if !grouped_armies.insert(army) {
                            return Err(Rejection::DuplicateIdentity);
                        }
                        let border_conflict_raw = *border_conflict_by_army
                            .get(&army)
                            .ok_or(Rejection::OrphanRow)?;
                        members.push(MemberRow {
                            kind: ObjectKind::Army,
                            object_id: army_id,
                            parent_kind: ObjectKind::ArmyGroup,
                            parent_id: group_id,
                            engine_order: child_order as u32,
                            reserve_navy: false,
                            display_name: None,
                            name_provenance: None,
                            priority_ordinal: lane::PriorityOrdinal::Unavailable,
                            orders_visibility: Some(army_orders_visibility(
                                army,
                                border_conflict_raw,
                            )?),
                        });
                    }
                }
                let priority_ordinal = read_u32_stable(
                    theater_group
                        .checked_add(THEATER_GROUP_OFF_PRIORITY_ORDINAL)
                        .ok_or(Rejection::SourceUnreadable)?,
                )
                .ok_or(Rejection::SourceUnreadable)?;
                let mut standalone_order = 0u32;
                for &army in &army_rows {
                    let army_id = validate_army_body(self.state.base, army)
                        .ok_or(Rejection::IdentityAuthenticationFailed)?;
                    enrol(&mut witnesses, army, (ARMY_VTABLE_RVA, ARMY_TYPE, army_id))?;
                    let parent = read_u64_stable(army + ARMY_OFF_AG_PARENT)
                        .ok_or(Rejection::SourceUnreadable)?
                        as usize;
                    if parent == 0 {
                        members.push(MemberRow {
                            kind: ObjectKind::Army,
                            object_id: army_id,
                            parent_kind: ObjectKind::LandTheatre,
                            parent_id: theatre_id,
                            engine_order: standalone_order,
                            reserve_navy: false,
                            display_name: None,
                            name_provenance: None,
                            priority_ordinal: lane::PriorityOrdinal::Unavailable,
                            orders_visibility: Some(army_orders_visibility(
                                army,
                                border_conflict_by_army[&army],
                            )?),
                        });
                        standalone_order = standalone_order.saturating_add(1);
                    } else if !grouped_armies.contains(&army) {
                        return Err(Rejection::OrphanRow);
                    }
                }
                // The LAST engine read of this row, so the witness sweep below
                // it genuinely closes every object's read window.
                let display_name = display_name(theater_group, THEATER_GROUP_OFF_NAME)?;
                for (&object, &(vtable_rva, type_tag, expected_id)) in &witnesses {
                    if validate_oob_body(self.state.base, object, vtable_rva, type_tag)
                        != Some(expected_id)
                    {
                        return Err(Rejection::IdentityAuthenticationFailed);
                    }
                }
                out.push(TheatreRow {
                    kind: ObjectKind::LandTheatre,
                    object_id: theatre_id,
                    owner_tag_db_id: country.tag_db_id,
                    engine_order: theatre_order,
                    display_name,
                    name_provenance: lane::NameProvenance::EngineRuntimeString,
                    priority_ordinal: lane::PriorityOrdinal::Measured(priority_ordinal),
                    delete_eligible: Some(delete_eligible),
                    members,
                    alert_categories: self
                        .alerts
                        .get(&theater_group)
                        .cloned()
                        .unwrap_or_default(),
                });
                theatre_order = theatre_order.saturating_add(1);
            }
        }
        Ok(())
    }

    fn navy(
        &self,
        country: Domain51CountryRoot,
        out: &mut Vec<lane::TheatreRow>,
    ) -> Result<(), Rejection> {
        use lane::{MemberRow, ObjectKind, TheatreRow};
        let manager = read_u64_stable(country.object_ptr + COUNTRY_NAVY_THEATRE_MANAGER)
            .ok_or(Rejection::SourceUnreadable)? as usize;
        if manager == 0 {
            return Ok(());
        }
        if !is_map_control_heap_pointer(manager) {
            return Err(Rejection::SourceUnreadable);
        }
        let mut seen = BTreeSet::new();
        let mut seen_fleets = BTreeSet::new();
        let mut theatre_order = 0u32;
        for theatre in ptr_vector(
            manager,
            NAVY_THEATRE_MANAGER_DATA,
            NAVY_THEATRE_MANAGER_CAPACITY,
            NAVY_THEATRE_MANAGER_COUNT,
        )? {
            let (theatre_id, _) =
                reference_id(theatre, self.state.base, NAVY_THEATRE_VTABLE_RVA, None)?;
            if !seen.insert(theatre_id) {
                return Err(Rejection::DuplicateIdentity);
            }
            let mut members = Vec::new();
            let mut normal_order = 0u32;
            let mut reserve_order = 0u32;
            for fleet in ptr_vector(
                theatre,
                NAVY_THEATRE_OFF_FLEET_DATA,
                NAVY_THEATRE_OFF_FLEET_CAPACITY,
                NAVY_THEATRE_OFF_FLEET_COUNT,
            )? {
                let fleet_id =
                    validate_oob_body(self.state.base, fleet, FLEET_VTABLE_RVA, OOB_TYPE_FLEET)
                        .ok_or(Rejection::IdentityAuthenticationFailed)?;
                if !seen_fleets.insert(fleet_id) {
                    return Err(Rejection::DuplicateIdentity);
                }
                let children = ptr_vector(
                    fleet,
                    FLEET_OFF_RESERVE_MEMBER_DATA,
                    FLEET_OFF_RESERVE_MEMBER_CAPACITY,
                    FLEET_OFF_RESERVE_MEMBER_COUNT,
                )?;
                let Some(first_task_force) = children.first().copied() else {
                    return Err(Rejection::IncompleteWalk);
                };
                let reserve = read_u32_stable(
                    first_task_force
                        .checked_add(TASK_FORCE_OFF_RESERVE_CLASSIFIER)
                        .ok_or(Rejection::SourceUnreadable)?,
                )
                .ok_or(Rejection::SourceUnreadable)?
                    == TASK_FORCE_RESERVE_CLASSIFIER_VALUE;
                let order = if reserve {
                    let order = reserve_order;
                    reserve_order = reserve_order.saturating_add(1);
                    order
                } else {
                    let order = normal_order;
                    normal_order = normal_order.saturating_add(1);
                    order
                };
                members.push(MemberRow {
                    kind: ObjectKind::Fleet,
                    object_id: fleet_id,
                    parent_kind: ObjectKind::NavyTheatre,
                    parent_id: theatre_id,
                    engine_order: order,
                    reserve_navy: reserve,
                    display_name: None,
                    name_provenance: None,
                    priority_ordinal: lane::PriorityOrdinal::Unavailable,
                    orders_visibility: None,
                });
            }
            let priority_ordinal = read_u8_stable(
                theatre
                    .checked_add(NAVY_THEATRE_OFF_PRIORITY_ORDINAL)
                    .ok_or(Rejection::SourceUnreadable)?,
            )
            .ok_or(Rejection::SourceUnreadable)?;
            out.push(TheatreRow {
                kind: ObjectKind::NavyTheatre,
                object_id: theatre_id,
                owner_tag_db_id: country.tag_db_id,
                engine_order: theatre_order,
                display_name: display_name(theatre, NAVY_THEATRE_OFF_NAME)?,
                name_provenance: lane::NameProvenance::EngineRuntimeString,
                priority_ordinal: lane::PriorityOrdinal::Measured(u32::from(priority_ordinal)),
                delete_eligible: None,
                members,
                alert_categories: Vec::new(),
            });
            theatre_order = theatre_order.saturating_add(1);
        }
        Ok(())
    }

    fn air(
        &self,
        country: Domain51CountryRoot,
        out: &mut Vec<lane::TheatreRow>,
        free: &mut Vec<lane::FreeAirWingRow>,
    ) -> Result<(), Rejection> {
        use lane::{FreeAirWingRow, MemberRow, ObjectKind, TheatreRow};
        let owned = self
            .wings_by_owner
            .get(&country.tag_db_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let mut wing_by_id = BTreeMap::new();
        for wing in owned {
            if wing_by_id.insert(wing.wing_id, wing).is_some() {
                return Err(Rejection::DuplicateIdentity);
            }
        }
        let mut grouped_wings = BTreeSet::new();
        let mut seen_theatres = BTreeSet::new();
        for (theatre_order, theatre) in ptr_vector(
            country.object_ptr,
            COUNTRY_AIR_THEATRE_DATA,
            COUNTRY_AIR_THEATRE_CAPACITY,
            COUNTRY_AIR_THEATRE_COUNT,
        )?
        .into_iter()
        .enumerate()
        {
            let (theatre_id, theatre_handle) = reference_id(
                theatre,
                self.state.base,
                AIR_THEATRE_VTABLE_RVA,
                Some(AIR_THEATRE_TYPE),
            )?;
            if !seen_theatres.insert(theatre_id) {
                return Err(Rejection::DuplicateIdentity);
            }
            let mut members = Vec::new();
            for (group_order, group) in ptr_vector(
                theatre,
                AIR_THEATRE_OFF_GROUP_DATA,
                AIR_THEATRE_OFF_GROUP_CAPACITY,
                AIR_THEATRE_OFF_GROUP_COUNT,
            )?
            .into_iter()
            .enumerate()
            {
                let (group_id, group_handle) = reference_id(
                    group,
                    self.state.base,
                    AIR_GROUP_VTABLE_RVA,
                    Some(AIR_GROUP_TYPE),
                )?;
                if read_u64_stable(group + AIR_GROUP_OFF_THEATRE_HANDLE) != Some(theatre_handle) {
                    return Err(Rejection::OrphanRow);
                }
                members.push(MemberRow {
                    kind: ObjectKind::AirGroup,
                    object_id: group_id,
                    parent_kind: ObjectKind::AirTheatre,
                    parent_id: theatre_id,
                    engine_order: group_order as u32,
                    reserve_navy: false,
                    display_name: Some(display_name(group, AIR_GROUP_OFF_NAME)?),
                    name_provenance: Some(lane::NameProvenance::EngineRuntimeString),
                    priority_ordinal: lane::PriorityOrdinal::Unavailable,
                    orders_visibility: None,
                });
                for (wing_order, wing_handle) in raw_vector(
                    group,
                    AIR_GROUP_OFF_WING_DATA,
                    AIR_GROUP_OFF_WING_CAPACITY,
                    AIR_GROUP_OFF_WING_COUNT,
                )?
                .into_iter()
                .enumerate()
                {
                    if wing_handle as u32 != AIR_WING_TYPE {
                        return Err(Rejection::IdentityAuthenticationFailed);
                    }
                    let wing_id = (wing_handle >> 32) as u32;
                    let wing = wing_by_id.get(&wing_id).ok_or(Rejection::OrphanRow)?;
                    if read_u64_stable(wing.body_addr + AIR_WING_OFF_GROUP_HANDLE)
                        != Some(group_handle)
                    {
                        return Err(Rejection::OrphanRow);
                    }
                    if !grouped_wings.insert(wing_id) {
                        return Err(Rejection::DuplicateIdentity);
                    }
                    members.push(MemberRow {
                        kind: ObjectKind::AirWing,
                        object_id: wing_id,
                        parent_kind: ObjectKind::AirGroup,
                        parent_id: group_id,
                        engine_order: wing_order as u32,
                        reserve_navy: false,
                        display_name: None,
                        name_provenance: None,
                        priority_ordinal: lane::PriorityOrdinal::Unavailable,
                        orders_visibility: None,
                    });
                }
            }
            out.push(TheatreRow {
                kind: ObjectKind::AirTheatre,
                object_id: theatre_id,
                owner_tag_db_id: country.tag_db_id,
                engine_order: theatre_order as u32,
                display_name: display_name(theatre, AIR_THEATRE_OFF_NAME)?,
                name_provenance: lane::NameProvenance::EngineRuntimeString,
                priority_ordinal: lane::PriorityOrdinal::Unavailable,
                delete_eligible: None,
                members,
                alert_categories: Vec::new(),
            });
        }
        for wing in owned {
            let group_handle = read_u64_stable(wing.body_addr + AIR_WING_OFF_GROUP_HANDLE)
                .ok_or(Rejection::SourceUnreadable)?;
            if group_handle == 0 {
                if grouped_wings.contains(&wing.wing_id) {
                    return Err(Rejection::DuplicateIdentity);
                }
                free.push(FreeAirWingRow {
                    wing_id: wing.wing_id,
                    engine_order: free.len() as u32,
                    priority_ordinal: lane::PriorityOrdinal::Unavailable,
                });
            } else if !grouped_wings.contains(&wing.wing_id) {
                return Err(Rejection::OrphanRow);
            }
        }
        Ok(())
    }
}

impl lane::ExactTheatreSelectorSource for AllCountrySource<'_> {
    fn is_exact_a729_theatre_selector_source(&self) -> bool {
        self.state.base != 0
            && self.state.daily_match
            && self.state.long_string_match
            && self.state.date_ptr != 0
            && self.state.daily_addr
                == self
                    .state
                    .base
                    .saturating_add(C_GAME_STATE_DAILY_UPDATE_OFFSET)
            && self.state.long_string_addr == self.state.base.saturating_add(GET_LONG_STRING_OFFSET)
            && sentinel_matches(self.state.daily_addr, &DAILY_UPDATE_SENTINEL)
            && sentinel_matches(self.state.long_string_addr, &GET_LONG_STRING_SENTINEL)
    }

    fn read_country_census(&self) -> Result<Vec<u32>, lane::CarrierError> {
        let mut census = Vec::with_capacity(self.roster.len());
        for row in &self.roster {
            if row.tag_db_id == 0 {
                return Err(frame_error("country roster carries a zero tag db id"));
            }
            census.push(row.tag_db_id);
        }
        census.sort_unstable();
        let before = census.len();
        census.dedup();
        if census.len() != before {
            return Err(frame_error("country roster carries a duplicate tag db id"));
        }
        if census.is_empty() {
            return Err(frame_error("country roster is empty"));
        }
        Ok(census)
    }

    fn read_content_identity(&self) -> Result<lane::ContentIdentity, lane::CarrierError> {
        Ok(self.identity.clone())
    }

    fn read_country_section(&self, owner_tag_db_id: u32) -> Result<lane::SectionBody, Rejection> {
        let country = self.country_root(owner_tag_db_id)?;
        let mut theatres = Vec::new();
        let mut free_air_wings = Vec::new();
        self.land(country, &mut theatres)?;
        self.navy(country, &mut theatres)?;
        self.air(country, &mut theatres, &mut free_air_wings)?;
        Ok(lane::SectionBody {
            theatres,
            free_air_wings,
        })
    }
}

#[cfg(not(windows))]
pub(super) fn compact_theatre_selector_worker_panic_read(
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> CompactTheatreSelectorRead {
    let failed = worker_panic_status(status_flags) | COMPACT_STATUS_SOURCE_UNAVAILABLE;
    let frame_hour = compact_contract::calendar::hour_index_from_raw(game_hour_raw)
        .and_then(|hour| u64::try_from(hour).ok())
        .unwrap_or(0);
    let payload = lane::encode_source_unavailable(
        frame_hour,
        frame_hour.saturating_add(1),
        elapsed_wall_ms,
        failed,
    )
    .expect("bounded XUI07 worker-panic unavailable payload");
    CompactTheatreSelectorRead {
        payload,
        observed_sections: 0,
        observed_theatres: 0,
        observed_members: 0,
        read_time_us: 0,
        status_flags: failed,
        diagnostic_units: vec![CompactDomainPerfDiagnosticUnit {
            name: "d98_fail_worker_panic",
            value: 1,
        }],
    }
}

fn unavailable_read(
    frame_hour: u64,
    source_sequence: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
    started: Instant,
    diagnostic_units: Vec<CompactDomainPerfDiagnosticUnit>,
) -> CompactTheatreSelectorRead {
    let payload =
        lane::encode_source_unavailable(frame_hour, source_sequence, elapsed_wall_ms, status_flags)
            .expect("bounded XUI07 unavailable payload");
    CompactTheatreSelectorRead {
        payload,
        observed_sections: 0,
        observed_theatres: 0,
        observed_members: 0,
        read_time_us: duration_micros_u64(started.elapsed()),
        status_flags,
        diagnostic_units,
    }
}

pub(super) fn read_compact_theatre_selector(
    state: &CollectorState,
    control: &CollectorControl,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> CompactTheatreSelectorRead {
    let started = Instant::now();
    let frame_hour = compact_contract::calendar::hour_index_from_raw(game_hour_raw)
        .and_then(|hour| u64::try_from(hour).ok());
    if compact_contract::compact_status_is_failure(status_flags) || frame_hour.is_none() {
        let failed = status_flags
            | COMPACT_STATUS_SOURCE_UNAVAILABLE
            | if frame_hour.is_none() {
                COMPACT_STATUS_NO_CLOCK
            } else {
                0
            };
        let hour = frame_hour.unwrap_or(0);
        return unavailable_read(
            hour,
            hour.saturating_add(1),
            elapsed_wall_ms,
            failed,
            started,
            vec![CompactDomainPerfDiagnosticUnit {
                name: "d98_fail_outer_frame",
                value: 1,
            }],
        );
    }
    let frame_hour = frame_hour.expect("checked above");
    let source_sequence = frame_hour.saturating_add(1);

    let build = (|| -> Result<AllCountrySource<'_>, lane::CarrierError> {
        let identity = content_identity(control)?;
        let roster = domain51_read_complete_country_roster(state)
            .ok_or_else(|| frame_error("authenticated country roster unavailable"))?;
        let native = sample_compact_air_wings_native(state)
            .ok_or_else(|| frame_error("strategic-air wing walk unavailable"))?;
        if native.invalid_air_wing_rows > 0 {
            return Err(frame_error("strategic-air wing walk reported invalid rows"));
        }
        let mut wings_by_owner: BTreeMap<u32, Vec<CompactAirWingRow>> = BTreeMap::new();
        for row in native.rows {
            if row.deleted {
                continue;
            }
            wings_by_owner
                .entry(row.owner_tag_db_id)
                .or_default()
                .push(row);
        }
        let alerts = alert_categories_by_theatre(state.base)?;
        Ok(AllCountrySource {
            state,
            roster,
            wings_by_owner,
            alerts,
            identity,
        })
    })();

    let source = match build {
        Ok(source) => source,
        Err(error) => {
            let failed = status_flags | COMPACT_STATUS_SOURCE_UNAVAILABLE;
            return unavailable_read(
                frame_hour,
                source_sequence,
                elapsed_wall_ms,
                failed,
                started,
                match &error {
                    lane::CarrierError::Source(message) if message == NO_CONTENT_IDENTITY => {
                        vec![CompactDomainPerfDiagnosticUnit {
                            name: "d98_fail_no_content_identity",
                            value: 1,
                        }]
                    }
                    _ => vec![
                        CompactDomainPerfDiagnosticUnit {
                            name: "d98_fail_frame_source",
                            value: 1,
                        },
                        CompactDomainPerfDiagnosticUnit {
                            name: "d98_fail_reason_hash",
                            value: fnv1a_64(format!("{error:?}").as_bytes()),
                        },
                    ],
                },
            );
        }
    };

    match lane::collect_and_encode(&source, frame_hour, source_sequence, elapsed_wall_ms) {
        Ok(payload) => {
            let decoded = lane::decode_payload(&payload).expect("fresh XUI07 payload must decode");
            let snapshot = decoded
                .snapshot
                .expect("clean XUI07 payload must contain sections");
            let mut observed_theatres = 0usize;
            let mut observed_members = 0usize;
            let mut refused_sections = 0u64;
            for section in &snapshot.sections {
                match &section.admission {
                    lane::SectionAdmission::Unavailable(_) => {
                        refused_sections = refused_sections.saturating_add(1)
                    }
                    lane::SectionAdmission::Admitted(body) => {
                        observed_theatres = observed_theatres.saturating_add(body.theatres.len());
                        observed_members = observed_members.saturating_add(
                            body.theatres
                                .iter()
                                .map(|row| {
                                    row.members.len().saturating_add(row.alert_categories.len())
                                })
                                .sum::<usize>()
                                .saturating_add(body.free_air_wings.len()),
                        );
                    }
                }
            }
            let mut diagnostic_units = Vec::new();
            if refused_sections > 0 {
                diagnostic_units.push(CompactDomainPerfDiagnosticUnit {
                    name: "d98_refused_sections",
                    value: refused_sections,
                });
            }
            CompactTheatreSelectorRead {
                payload,
                observed_sections: snapshot.sections.len().min(u32::MAX as usize) as u32,
                observed_theatres: observed_theatres.min(u32::MAX as usize) as u32,
                observed_members: observed_members.min(u32::MAX as usize) as u32,
                read_time_us: duration_micros_u64(started.elapsed()),
                status_flags: 0,
                diagnostic_units,
            }
        }
        Err(error) => {
            let failed = status_flags | COMPACT_STATUS_SOURCE_UNAVAILABLE;
            unavailable_read(
                frame_hour,
                source_sequence,
                elapsed_wall_ms,
                failed,
                started,
                vec![
                    CompactDomainPerfDiagnosticUnit {
                        name: "d98_fail_source",
                        value: 1,
                    },
                    CompactDomainPerfDiagnosticUnit {
                        name: "d98_fail_reason_hash",
                        value: fnv1a_64(format!("{error:?}").as_bytes()),
                    },
                ],
            )
        }
    }
}
