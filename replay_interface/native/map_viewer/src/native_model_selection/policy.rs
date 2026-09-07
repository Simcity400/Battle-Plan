#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativePickerClass {
    Division,
    TaskForce,
    RailwayGun,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativePickerCandidate<I> {
    pub identity: I,
    pub class: NativePickerClass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativePickerRefusal<E> {
    CandidateCount,
    DuplicateIdentity { first: usize, repeated: usize },
    Candidate { ordinal: usize, reason: E },
}

/// One camera's direct model pass. A miss still requires the owning province fallback.
/// The caller supplies a complete current candidate projection, never drawable row order.
pub fn pick_ordered_model_pass<'a, I: Ord, E>(
    candidates: &'a [NativePickerCandidate<I>],
    selected: &[I],
    cycle: bool,
    mut hit: impl FnMut(&NativePickerCandidate<I>) -> Result<bool, E>,
) -> Result<Option<&'a I>, NativePickerRefusal<E>> {
    if candidates.len() > i32::MAX as usize {
        return Err(NativePickerRefusal::CandidateCount);
    }
    let mut identities = std::collections::BTreeMap::new();
    for (ordinal, candidate) in candidates.iter().enumerate() {
        if let Some(first) = identities.insert(&candidate.identity, ordinal) {
            return Err(NativePickerRefusal::DuplicateIdentity { first, repeated: ordinal });
        }
    }
    let start = if cycle && selected.len() == 1 {
        candidates.iter().position(|candidate| candidate.identity == selected[0])
            .map_or(0, |ordinal| ordinal + 1)
    } else { 0 };
    for step in 0..candidates.len() {
        let ordinal = (start + step) % candidates.len();
        let candidate = &candidates[ordinal];
        // Class 1 participates in the cycle anchor, but 140b450e0 skips its model test.
        if candidate.class == NativePickerClass::TaskForce { continue; }
        if hit(candidate).map_err(|reason| NativePickerRefusal::Candidate { ordinal, reason })? {
            return Ok(Some(&candidate.identity));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(identity: u32, class: NativePickerClass) -> NativePickerCandidate<u32> {
        NativePickerCandidate { identity, class }
    }

    #[test]
    fn ordinary_pass_uses_candidate_order_instead_of_identity_or_nearest_hit() {
        let rows = [candidate(90, NativePickerClass::Division), candidate(2, NativePickerClass::Division)];
        let mut visited = Vec::new();
        assert_eq!(pick_ordered_model_pass(&rows, &[], false, |row| {
            visited.push(row.identity); Ok::<_, ()>(true)
        }), Ok(Some(&90)));
        assert_eq!(visited, [90]);
    }

    #[test]
    fn task_force_is_a_cycle_anchor_without_a_model_query() {
        let rows = [candidate(90, NativePickerClass::Division),
            candidate(8, NativePickerClass::TaskForce), candidate(2, NativePickerClass::Division)];
        let mut visited = Vec::new();
        assert_eq!(pick_ordered_model_pass(&rows, &[8], true, |row| {
            visited.push(row.identity); Ok::<_, ()>(true)
        }), Ok(Some(&2)));
        assert_eq!(visited, [2]);
    }

    #[test]
    fn selected_last_candidate_wraps_to_first_and_selected_first_is_tested_last() {
        let rows = [candidate(1, NativePickerClass::Division), candidate(2, NativePickerClass::RailwayGun)];
        assert_eq!(pick_ordered_model_pass(&rows, &[2], true, |_| Ok::<_, ()>(true)), Ok(Some(&1)));
        let mut visited = Vec::new();
        assert_eq!(pick_ordered_model_pass(&rows, &[1], true, |row| {
            visited.push(row.identity); Ok::<_, ()>(row.identity == 1)
        }), Ok(Some(&1)));
        assert_eq!(visited, [2, 1]);
    }

    #[test]
    fn no_cycle_multiple_selection_and_absent_anchor_begin_at_first() {
        let rows = [candidate(1, NativePickerClass::Division), candidate(2, NativePickerClass::Division)];
        for (selected, cycle) in [(vec![1], false), (vec![1, 2], true), (vec![99], true), (vec![], true)] {
            assert_eq!(pick_ordered_model_pass(&rows, &selected, cycle, |_| Ok::<_, ()>(true)), Ok(Some(&1)));
        }
    }

    #[test]
    fn earlier_unavailable_candidate_cannot_be_filtered_to_admit_a_later_hit() {
        let rows = [candidate(1, NativePickerClass::Division), candidate(2, NativePickerClass::Division)];
        let mut visited = Vec::new();
        assert_eq!(pick_ordered_model_pass(&rows, &[], false, |row| {
            visited.push(row.identity);
            if row.identity == 1 { Err("province unavailable") } else { Ok(true) }
        }), Err(NativePickerRefusal::Candidate { ordinal: 0, reason: "province unavailable" }));
        assert_eq!(visited, [1]);
    }

    #[test]
    fn a_known_hit_does_not_consume_a_later_unavailable_candidate() {
        let rows = [candidate(1, NativePickerClass::Division), candidate(2, NativePickerClass::Division)];
        assert_eq!(pick_ordered_model_pass(&rows, &[], false, |row|
            if row.identity == 1 { Ok(true) } else { Err("later unavailable") }), Ok(Some(&1)));
    }

    #[test]
    fn duplicate_identity_refuses_before_any_geometry_query_even_across_classes() {
        let rows = [candidate(1, NativePickerClass::Division), candidate(1, NativePickerClass::RailwayGun)];
        assert_eq!(pick_ordered_model_pass(&rows, &[], false, |_| -> Result<bool, ()> {
            panic!("ambiguous identity reached geometry")
        }), Err(NativePickerRefusal::DuplicateIdentity { first: 0, repeated: 1 }));
    }

    #[test]
    fn empty_and_task_force_only_passes_miss_without_querying_geometry() {
        for rows in [vec![], vec![candidate(1, NativePickerClass::TaskForce)]] {
            assert_eq!(pick_ordered_model_pass(&rows, &[1], true, |_| -> Result<bool, ()> {
                panic!("non-model candidate reached geometry")
            }), Ok(None));
        }
    }
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelPickerInputRefusal {
    NativeCandidateContextUnavailable,
    CameraUnavailable,
    Camera(String),
    Publication(crate::Xmap13ModelObjectRefusal),
}

#[derive(Clone)]
pub struct ModelPickerFrame {
    prepared: crate::PreparedXmap13Frame,
    camera: Option<crate::ViewerMapCameraState>,
    publication: Result<crate::Xmap13ModelHitPublication, crate::Xmap13ModelObjectRefusal>,
}

impl ModelPickerFrame {
    pub(crate) fn from_preparation(
        prepared: crate::PreparedXmap13Frame,
        camera: Option<crate::ViewerMapCameraState>,
        publication: Result<crate::Xmap13ModelHitPublication, crate::Xmap13ModelObjectRefusal>,
    ) -> Self {
        Self { prepared, camera, publication }
    }

    pub fn prepared(&self) -> &crate::PreparedXmap13Frame { &self.prepared }

    pub fn publication_refusal(&self) -> Option<&crate::Xmap13ModelObjectRefusal> {
        self.publication.as_ref().err()
    }


    // Candidate category/province are supplied by the selection owner, never inferred from draws.
    pub fn geometry_hit(
        &self,
        cursor: [i32; 2],
        identity: &crate::Xmap13UnitIdentity,
        province: u32,
        category: u32,
    ) -> Result<bool, ModelPickerInputRefusal> {
        let publication = self.publication.as_ref()
            .map_err(|reason| ModelPickerInputRefusal::Publication(reason.clone()))?;
        let camera = self.camera.as_ref().ok_or(ModelPickerInputRefusal::CameraUnavailable)?;
        let (origin, direction) = camera.native_screen_ray(cursor)
            .map_err(|error| ModelPickerInputRefusal::Camera(error.to_string()))?;
        publication.geometry_hit(&self.prepared, identity, province, category, origin, direction)
            .map_err(ModelPickerInputRefusal::Publication)
    }
}
