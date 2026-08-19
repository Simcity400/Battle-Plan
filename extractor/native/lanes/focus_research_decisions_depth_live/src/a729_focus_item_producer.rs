use compact_contract::national_focus_presentation::{
    valid_focus_id, validate_semantic_snapshot, ExactNationalFocusPresentationSource,
    FocusPresentationItem, Snapshot, EXACT20_FUNCTION_MANIFEST_SHA256,
};
use std::collections::BTreeSet;

pub const IMAGE_BASE: usize = 0x1400_0000_0;
pub const VIEW_CONSTRUCTOR_RVA: usize = 0x0136_aec0;
pub const REBUILD_OWNER_RVA: usize = 0x0137_1b70;
pub const VECTOR_PUBLISHER_RVA: usize = 0x0136_f750;
pub const ITEM_CONSTRUCTOR_RVA: usize = 0x0136_a5c0;
pub const DYNAMIC_REFRESH_RVA: usize = 0x0137_a780;
pub const DETAIL_REFRESH_RVA: usize = 0x0137_a600;
pub const ITEM_STATE_PRODUCER_RVA: usize = 0x0137_9a50;
pub const ITEM_STATE_WRITER_RVA: usize = 0x0137_78f0;
pub const HIGHLIGHT_PRODUCER_RVA: usize = 0x0137_7430;
pub const DISPLAY_PROGRESS_PRODUCER_RVA: usize = 0x01bb_5df0;

pub const VIEW_VTABLE: usize = 0x1429_9d680;
pub const ITEM_PRIMARY_VTABLE: usize = 0x1429_9dd40;
pub const ITEM_SECONDARY_VTABLE: usize = 0x1429_9dde0;
pub const ITEM_ALLOCATION_SIZE: usize = 0x550;
pub const VIEW_VECTOR_DATA_OFFSET: usize = 0x2058;
pub const VIEW_VECTOR_CAPACITY_OFFSET: usize = 0x2060;
pub const VIEW_VECTOR_COUNT_OFFSET: usize = 0x2064;
pub const VIEW_VECTOR_ALLOCATOR_OFFSET: usize = 0x2068;
pub const ITEM_DEFINITION_OFFSET: usize = 0x20;
pub const ITEM_PRESENTATION_STATE_OFFSET: usize = 0x540;
pub const ITEM_NATIONAL_CAPABILITY_OFFSET: usize = 0x54c;
pub const ITEM_JOINT_CAPABILITY_OFFSET: usize = 0x54d;
pub const FOCUS_DEFINITION_ID_OFFSET: usize = 0x18;
pub const FOCUS_HISTORICAL_MARKER_OFFSET: usize = 0x5b8;
pub const DETAIL_DEFINITION_OFFSET: usize = 0xf50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VectorHeader {
    pub data: usize,
    pub capacity: u32,
    pub count: u32,
    pub allocator: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalItemObservation {
    pub item_address: usize,
    pub primary_vtable: usize,
    pub secondary_vtable: usize,
    pub definition_address: usize,
    pub presentation_state_code: u8,
    pub display_progress_percent: Option<u8>,
    pub highlight_glow_visible: Option<bool>,
    pub joint_viewing_country_tag: Option<Option<String>>,
    pub historical_marker_visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalFrameObservation {
    pub source_sequence: u64,
    pub frame_hour: u64,
    pub country_tag: String,
    pub view_address: usize,
    pub view_vtable: usize,
    pub opening_header: VectorHeader,
    pub opening_item_addresses: Vec<usize>,
    pub items: Vec<ProcessLocalItemObservation>,
    pub closing_item_addresses: Vec<usize>,
    pub closing_header: VectorHeader,
    pub rebuild_completed: bool,
    pub dynamic_refresh_completed: bool,
    pub transient_epoch_completed: bool,
}

pub trait ExactA729FocusObservationBoundary {
    fn read_complete_observation(
        &self,
        frame_hour: u64,
        country_tag: &str,
    ) -> Result<ProcessLocalFrameObservation, String>;
    fn stable_focus_id(&self, definition_address: usize) -> Result<String, String>;
}

pub struct A729FocusItemSource<B> {
    boundary: B,
}

impl<B> A729FocusItemSource<B> {
    pub fn new(boundary: B) -> Self {
        Self { boundary }
    }
}

impl<B: ExactA729FocusObservationBoundary> ExactNationalFocusPresentationSource
    for A729FocusItemSource<B>
{
    fn is_exact_a729_focus_view_source(&self) -> bool {
        true
    }

    fn producer_manifest_sha256(&self) -> [u8; 32] {
        EXACT20_FUNCTION_MANIFEST_SHA256
    }

    fn collect_snapshot(&self, frame_hour: u64, country_tag: &str) -> Result<Snapshot, String> {
        let observation = self
            .boundary
            .read_complete_observation(frame_hour, country_tag)?;
        if observation.frame_hour != frame_hour || observation.country_tag != country_tag {
            return Err("A31 requested and observed frame identity differ".into());
        }
        materialize_pointer_free_snapshot(&observation, &self.boundary)
    }
}

pub fn materialize_pointer_free_snapshot<B: ExactA729FocusObservationBoundary>(
    observation: &ProcessLocalFrameObservation,
    boundary: &B,
) -> Result<Snapshot, String> {
    if observation.source_sequence == 0
        || observation.view_address == 0
        || observation.view_vtable != VIEW_VTABLE
        || !observation.rebuild_completed
        || !observation.dynamic_refresh_completed
        || !observation.transient_epoch_completed
        || observation.opening_header != observation.closing_header
        || observation.opening_item_addresses != observation.closing_item_addresses
        || observation.opening_header.count as usize != observation.items.len()
        || observation.opening_header.count as usize != observation.opening_item_addresses.len()
        || observation.opening_header.count > observation.opening_header.capacity
        || (observation.opening_header.count != 0 && observation.opening_header.data == 0)
    {
        return Err("A31 owner/vector epoch was incomplete or unstable".into());
    }
    if observation.country_tag.len() != 3
        || !observation
            .country_tag
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return Err("A31 observation country tag was not canonical".into());
    }

    let mut process_items = BTreeSet::new();
    let mut definitions = BTreeSet::new();
    let mut visible_items = Vec::with_capacity(observation.items.len());
    for (ordinal, item) in observation.items.iter().enumerate() {
        if item.item_address == 0
            || item.definition_address == 0
            || item.primary_vtable != ITEM_PRIMARY_VTABLE
            || item.secondary_vtable != ITEM_SECONDARY_VTABLE
            || observation.opening_item_addresses[ordinal] != item.item_address
            || !process_items.insert(item.item_address)
            || !definitions.insert(item.definition_address)
        {
            return Err("A31 item identity/vtable/vector join failed".into());
        }
        let focus_id = boundary.stable_focus_id(item.definition_address)?;
        if !valid_focus_id(&focus_id) {
            return Err(
                "A31 definition lookup returned a nonportable or host-shaped focus id".into(),
            );
        }
        let display_progress_percent = item
            .display_progress_percent
            .ok_or_else(|| "A31 display-percent producer leaf was missed".to_string())?;
        let highlight_glow_visible = item
            .highlight_glow_visible
            .ok_or_else(|| "A31 highlight producer leaf was missed".to_string())?;
        let joint_viewing_country_tag = item
            .joint_viewing_country_tag
            .clone()
            .ok_or_else(|| "A31 joint-view helper leaf was missed".to_string())?;
        visible_items.push(FocusPresentationItem {
            focus_id,
            source_ordinal: ordinal as u32,
            presentation_state_code: item.presentation_state_code,
            display_progress_percent,
            highlight_glow_visible,
            historical_marker_visible: item.historical_marker_visible,
            joint_viewing_country_tag,
            selected_present_ui_operation_ordinals: Vec::new(),
            selected_present_screen_rect:
                compact_contract::national_focus_presentation::RectI32::EMPTY,
        });
    }

    let snapshot = Snapshot {
        source_sequence: observation.source_sequence,
        producer_complete: true,
        country_tag: observation.country_tag.clone(),
        view_id: format!("nationalfocusview:{}", observation.country_tag),
        producer_record_identity: format!(
            "a729:a31:{}:{}:{}",
            observation.frame_hour, observation.country_tag, observation.source_sequence
        ),
        expected_visible_item_count: observation.items.len() as u32,
        visible_items,
        selected_present_canvas_operation_ordinals: Vec::new(),
        selected_present_ui_operation_order: Vec::new(),
    };
    validate_semantic_snapshot(&snapshot).map_err(|error| format!("{error:?}"))?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Boundary {
        observation: ProcessLocalFrameObservation,
        ids: BTreeMap<usize, String>,
    }

    impl ExactA729FocusObservationBoundary for Boundary {
        fn read_complete_observation(
            &self,
            _: u64,
            _: &str,
        ) -> Result<ProcessLocalFrameObservation, String> {
            Ok(self.observation.clone())
        }
        fn stable_focus_id(&self, definition_address: usize) -> Result<String, String> {
            self.ids
                .get(&definition_address)
                .cloned()
                .ok_or_else(|| "missing stable focus id".into())
        }
    }

    fn boundary() -> Boundary {
        let header = VectorHeader {
            data: 0x2000,
            capacity: 2,
            count: 2,
            allocator: 0x3000,
        };
        Boundary {
            observation: ProcessLocalFrameObservation {
                source_sequence: 5,
                frame_hour: 42,
                country_tag: "GER".into(),
                view_address: 0x1000,
                view_vtable: VIEW_VTABLE,
                opening_header: header,
                opening_item_addresses: vec![0x4000, 0x5000],
                items: vec![
                    ProcessLocalItemObservation {
                        item_address: 0x4000,
                        primary_vtable: ITEM_PRIMARY_VTABLE,
                        secondary_vtable: ITEM_SECONDARY_VTABLE,
                        definition_address: 0x6000,
                        presentation_state_code: 1,
                        display_progress_percent: Some(100),
                        highlight_glow_visible: Some(false),
                        joint_viewing_country_tag: Some(None),
                        historical_marker_visible: true,
                    },
                    ProcessLocalItemObservation {
                        item_address: 0x5000,
                        primary_vtable: ITEM_PRIMARY_VTABLE,
                        secondary_vtable: ITEM_SECONDARY_VTABLE,
                        definition_address: 0x7000,
                        presentation_state_code: 3,
                        display_progress_percent: Some(25),
                        highlight_glow_visible: Some(true),
                        joint_viewing_country_tag: Some(Some("ITA".into())),
                        historical_marker_visible: false,
                    },
                ],
                closing_item_addresses: vec![0x4000, 0x5000],
                closing_header: header,
                rebuild_completed: true,
                dynamic_refresh_completed: true,
                transient_epoch_completed: true,
            },
            ids: BTreeMap::from([(0x6000, "ger_a".into()), (0x7000, "ger_b".into())]),
        }
    }

    #[test]
    fn process_addresses_are_replaced_by_stable_ids_before_transport() {
        let boundary = boundary();
        let snapshot = materialize_pointer_free_snapshot(&boundary.observation, &boundary).unwrap();
        assert_eq!(snapshot.visible_items[0].focus_id, "ger_a");
        assert_eq!(
            snapshot.visible_items[1]
                .joint_viewing_country_tag
                .as_deref(),
            Some("ITA")
        );
        assert!(!snapshot.producer_record_identity.contains("0x"));
    }

    #[test]
    fn torn_vector_or_missing_transient_leaf_rejects_whole_epoch() {
        let mut torn_boundary = boundary();
        torn_boundary.observation.closing_header.count = 1;
        assert!(
            materialize_pointer_free_snapshot(&torn_boundary.observation, &torn_boundary).is_err()
        );
        let mut missing_leaf_boundary = boundary();
        missing_leaf_boundary.observation.items[0].highlight_glow_visible = None;
        assert!(materialize_pointer_free_snapshot(
            &missing_leaf_boundary.observation,
            &missing_leaf_boundary,
        )
        .is_err());
    }

    #[test]
    fn producer_rejects_pointer_path_and_host_shaped_definition_ids() {
        for value in [
            "0x000001abcdef1234",
            r"C:\\hoi4\\common\\national_focus\\bad.txt",
            "/home/user/hoi4/focus",
            r"\\capture-host\\share\\focus",
            "focus/child",
        ] {
            let mut boundary = boundary();
            boundary.ids.insert(0x6000, value.into());
            assert!(
                materialize_pointer_free_snapshot(&boundary.observation, &boundary).is_err(),
                "{value}"
            );
        }
        let mut boundary = boundary();
        boundary.ids.insert(0x6000, "mod-focus.v2@shared".into());
        assert!(materialize_pointer_free_snapshot(&boundary.observation, &boundary).is_ok());
    }
}
