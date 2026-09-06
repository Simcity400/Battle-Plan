use crate::{PreparedXmap15Frame, PreparedXmap16Frame, Xmap15LifecycleUpdate};
use crate::production_features::xmap15::prepared_row_key;
use portable_render_stream::Xmap15PreparedRow;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ArrowSortKey {
    pub primary_bits: u32,
    pub secondary_bits: u32,
}

impl ArrowSortKey {
    pub fn from_operands(width: [f32; 2], scale: f32, object_width: f32, secondary: f32) -> Self {
        // MAXSS returns its second operand for equal or unordered operands.
        let maximum = if width[1] > width[0] { width[1] } else { width[0] };
        let scaled = object_width * scale;
        Self { primary_bits: (maximum + scaled).to_bits(), secondary_bits: secondary.to_bits() }
    }

    pub fn precedes(self, other: Self) -> bool {
        let a = f32::from_bits(self.primary_bits);
        let b = f32::from_bits(other.primary_bits);
        a > b || (a == b && f32::from_bits(self.secondary_bits) > f32::from_bits(other.secondary_bits))
    }

    fn finite(self) -> bool {
        f32::from_bits(self.primary_bits).is_finite() && f32::from_bits(self.secondary_bits).is_finite()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum ArrowFamily { Orders, Supply }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum SupplySortInput { Definition, EngineRegistrationOrdinal }

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SupplyArrowSortInputs {
    pub mesh_row_ordinal: u32,
    pub definition_name: String,
    pub definition_index: Option<u32>,
    pub body_width_bits: Option<[u32; 2]>,
    pub width_scale_bits: Option<u32>,
    pub effective_width_scale_bits: Option<u32>,
    pub object_width_bits: Option<u32>,
    pub key: Option<ArrowSortKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum ArrowOrderRefusal {
    MissingRegistrationOrdinal { family: ArrowFamily, row: usize },
    MissingDefinition { family: ArrowFamily, row: usize },
    DefinitionIndexConflict { family: ArrowFamily, row: usize, index: u32 },
    DuplicateRegistration { family: ArrowFamily, row: usize, index: u32, ordinal: u32 },
    DuplicateRow { family: ArrowFamily, row: usize },
    NonFiniteSortKey { family: ArrowFamily, row: usize },
    MissingViewerArrowLifecycle { family: ArrowFamily, row: usize },
    ViewerInteractionSortKeyUnproven { family: ArrowFamily, row: usize },
    MissingSupplySortInputs { row: usize, inputs: SupplyArrowSortInputs, missing: Vec<SupplySortInput> },
    MissingNativePlanDrawInputs { row: usize },
    MissingDerivedPlanInput(portable_render_stream::Xmap15ViewerPlanRefusal),
    MissingOrderSemanticCarrier,
    MissingSupplySemanticCarrier,
}

impl std::fmt::Display for ArrowOrderRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "arrow-order:{self:?}") }
}
impl std::error::Error for ArrowOrderRefusal {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ArrowSubmission {
    pub family: ArrowFamily,
    pub row: usize,
    pub definition_index: u32,
    pub definition_name: String,
    pub registration_ordinal: Option<u32>,
    pub key: ArrowSortKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ArrowSubmissionBatch {
    pub definition_index: Option<u32>,
    pub outer_pass: bool,
    pub family: ArrowFamily,
    pub rows: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ArrowSubmissionOrder {
    pub submissions: Vec<ArrowSubmission>,
    pub refusals: Vec<ArrowOrderRefusal>,
}

fn resolve_arrow(family: ArrowFamily, row: usize,
    definition: Option<(&str, u32, [f32; 2], f32)>, registration_ordinal: Option<u32>,
    object_width: f32, secondary: f32) -> Result<ArrowSubmission, ArrowOrderRefusal> {
    let (name, definition_index, width, scale) = definition
        .ok_or(ArrowOrderRefusal::MissingDefinition { family, row })?;
    Ok(ArrowSubmission { family, row, definition_index, definition_name: name.to_owned(),
        registration_ordinal, key: ArrowSortKey::from_operands(width, scale, object_width, secondary) })
}

pub enum ArrowFrameInput<'a, T> {
    Present(&'a T),
    Unavailable,
    ViewInactive,
}

impl<T> ArrowFrameInput<'_, T> {
    fn candidates(self, resolve: impl FnOnce(&T) -> Vec<Result<ArrowSubmission, ArrowOrderRefusal>>)
        -> Option<Vec<Result<ArrowSubmission, ArrowOrderRefusal>>> {
        match self {
            Self::Present(frame) => Some(resolve(frame)),
            Self::Unavailable => None,
            Self::ViewInactive => Some(Vec::new()),
        }
    }
}

#[derive(Debug, Default)]
pub struct ArrowViewerSortState {
    retained: BTreeMap<String, bool>,
    selected: Option<String>,
    last_frame: Option<(String, String, i64, u64, Xmap15LifecycleUpdate)>,
}

impl ArrowViewerSortState {
    fn observe(&mut self, replay_frame: &str, prepared_frame: &str, hour: i64, sequence: u64,
        keys: BTreeSet<String>, selected: Option<String>, witness: &Xmap15LifecycleUpdate) -> bool {
        let mut unsigned = witness.clone();
        unsigned.deterministic_sha256.clear();
        let digest = serde_json::to_vec(&unsigned)
            .map(|bytes| snapshot::sha256::Sha256::hex(&bytes));
        let hash = |value: &str| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
        if !hash(replay_frame) || !hash(prepared_frame)
            || witness.hour_index != hour || witness.observation_sequence != sequence
            || digest.as_ref().ok() != Some(&witness.deterministic_sha256)
            || witness.authenticated_empty != keys.is_empty()
            || selected.as_ref().is_some_and(|key| !keys.contains(key)) {
            return false;
        }
        let stamp = (replay_frame.to_owned(), prepared_frame.to_owned(), hour, sequence, witness.clone());
        if self.last_frame.as_ref() == Some(&stamp) {
            return self.selected == selected && self.retained.keys().cloned().collect::<BTreeSet<_>>() == keys;
        }
        let unique = |values: &[String]| values.iter().cloned().collect::<BTreeSet<_>>();
        let created = unique(&witness.created);
        let deleted = unique(&witness.deleted);
        let edited = unique(&witness.edited);
        let prior = self.retained.keys().cloned().collect::<BTreeSet<_>>();
        if created.len() != witness.created.len() || deleted.len() != witness.deleted.len()
            || edited.len() != witness.edited.len()
            || created != keys.difference(&prior).cloned().collect::<BTreeSet<_>>()
            || deleted != prior.difference(&keys).cloned().collect::<BTreeSet<_>>()
            || !edited.is_subset(&keys.intersection(&prior).cloned().collect::<BTreeSet<_>>())
            || witness.selected_plan_changed != (self.selected != selected) {
            return false;
        }
        self.retained.retain(|key, _| keys.contains(key));
        // Only the joined lifecycle's creation event establishes constructor +0.
        for key in created { self.retained.insert(key, false); }
        if let Some(key) = &selected { self.retained.insert(key.clone(), true); }
        self.selected = selected;
        self.last_frame = Some(stamp);
        true
    }

    fn observe_frame(&mut self, replay_frame: &str, frame: &PreparedXmap15Frame,
        witness: Option<&Xmap15LifecycleUpdate>) -> bool {
        let Some(witness) = witness else { return false };
        if frame.validate().is_err() { return false; }
        let mut keys = frame.portable.ordered_rows.iter().map(prepared_row_key).collect::<BTreeSet<_>>();
        if let Some(runtime) = &frame.portable.viewer_runtime {
            keys.extend(runtime.retained_plans.iter().map(crate::production_features::xmap15::prepared_plan_key));
        }
        let selected = frame.portable.ordered_rows.iter().find(|row| matches!(row,
            Xmap15PreparedRow::Plan { semantic, .. }
                if Some(&semantic.key) == frame.portable.selected_plan.as_ref())).map(prepared_row_key);
        self.observe(replay_frame, &frame.portable.deterministic_sha256,
            frame.portable.hour_index, frame.portable.observation_sequence,
            keys, selected, witness)
    }

    fn secondary(&self, key: &str, family: ArrowFamily, row: usize) -> Result<f32, ArrowOrderRefusal> {
        match self.retained.get(key) {
            Some(false) => Ok(0.0),
            Some(true) => Err(ArrowOrderRefusal::ViewerInteractionSortKeyUnproven { family, row }),
            None => Err(ArrowOrderRefusal::MissingViewerArrowLifecycle { family, row }),
        }
    }
}

impl ArrowSubmissionOrder {
    fn resolve_candidates(orders: Option<Vec<Result<ArrowSubmission, ArrowOrderRefusal>>>,
        supply: Option<Vec<Result<ArrowSubmission, ArrowOrderRefusal>>>) -> Self {
        let mut rows = Vec::new();
        let mut refusals = Vec::new();
        let mut supply_diagnostics = BTreeMap::new();
        for (candidates, missing) in [(orders, ArrowOrderRefusal::MissingOrderSemanticCarrier),
            (supply, ArrowOrderRefusal::MissingSupplySemanticCarrier)] {
            match candidates {
                None => refusals.push(missing),
                Some(candidates) => for row in candidates {
                    match row {
                        Ok(row) => rows.push(row),
                        Err(ArrowOrderRefusal::MissingSupplySortInputs { row, inputs, missing })
                            if missing == [SupplySortInput::EngineRegistrationOrdinal]
                                && inputs.key.is_some_and(ArrowSortKey::finite)
                                && inputs.definition_index.is_some() => {
                            rows.push(ArrowSubmission {
                                family: ArrowFamily::Supply, row,
                                definition_index: inputs.definition_index.unwrap(),
                                definition_name: inputs.definition_name.clone(),
                                registration_ordinal: None, key: inputs.key.unwrap(),
                            });
                            supply_diagnostics.insert(row, (inputs, missing));
                        },
                        Err(reason) => refusals.push(reason),
                    }
                }
            }
        }
        let mut result = Self::compose(rows);
        for refusal in &mut result.refusals {
            if let ArrowOrderRefusal::MissingRegistrationOrdinal { family: ArrowFamily::Supply, row } = refusal {
                if let Some((inputs, missing)) = supply_diagnostics.remove(row) {
                    *refusal = ArrowOrderRefusal::MissingSupplySortInputs { row: *row, inputs, missing };
                }
            }
        }
        result.refusals.extend(refusals);
        result
    }

    pub fn compose(rows: Vec<ArrowSubmission>) -> Self {
        let mut buckets = BTreeMap::<u32, Vec<ArrowSubmission>>::new();
        let mut refusals = Vec::new();
        let mut identities = BTreeMap::<(ArrowFamily, usize), usize>::new();
        for row in &rows { *identities.entry((row.family, row.row)).or_default() += 1; }
        for row in rows {
            if identities[&(row.family, row.row)] > 1 {
                refusals.push(ArrowOrderRefusal::DuplicateRow { family: row.family, row: row.row });
            } else if !row.key.finite() {
                // NaN is not a strict weak ordering. Do not substitute total_cmp.
                refusals.push(ArrowOrderRefusal::NonFiniteSortKey { family: row.family, row: row.row });
            } else {
                buckets.entry(row.definition_index).or_default().push(row);
            }
        }
        let mut submissions = Vec::new();
        for (index, mut bucket) in buckets {
            if bucket.iter().any(|row| row.definition_name != bucket[0].definition_name) {
                refusals.extend(bucket.iter().map(|r| ArrowOrderRefusal::DefinitionIndexConflict { family: r.family, row: r.row, index }));
                continue;
            }
            let mut registrations = BTreeMap::<u32, usize>::new();
            for ordinal in bucket.iter().filter_map(|row| row.registration_ordinal) {
                *registrations.entry(ordinal).or_default() += 1;
            }
            bucket.retain(|row| {
                let Some(ordinal) = row.registration_ordinal else { return true };
                if registrations[&ordinal] == 1 { return true; }
                refusals.push(ArrowOrderRefusal::DuplicateRegistration { family: row.family, row: row.row,
                    index, ordinal });
                false
            });
            bucket.sort_by_key(|row| row.registration_ordinal);
            stable_arrow_sort(&mut bucket);
            // Registration affects only comparator-equivalent rows. Inspect the
            // complete sorted run before removing any row with an unknown slot.
            let mut ambiguous = BTreeSet::new();
            for pair in bucket.windows(2) {
                if !pair[0].key.precedes(pair[1].key) && !pair[1].key.precedes(pair[0].key) {
                    for row in pair {
                        if row.registration_ordinal.is_none() { ambiguous.insert((row.family, row.row)); }
                    }
                }
            }
            bucket.retain(|row| {
                if !ambiguous.contains(&(row.family, row.row)) { return true; }
                refusals.push(ArrowOrderRefusal::MissingRegistrationOrdinal { family: row.family, row: row.row });
                false
            });
            submissions.extend(bucket);
        }
        Self { submissions, refusals }
    }

    pub fn from_frames(orders: ArrowFrameInput<'_, PreparedXmap15Frame>,
        supply: ArrowFrameInput<'_, PreparedXmap16Frame>, viewer: &mut ArrowViewerSortState,
        lifecycle: Option<&Xmap15LifecycleUpdate>, replay_frame: &str) -> Self {
        let order_rows = orders.candidates(|frame| {
            let joined = viewer.observe_frame(replay_frame, frame, lifecycle);
            let mut candidates = frame.portable.ordered_rows.iter().enumerate().filter_map(|(row, prepared)| {
                let Xmap15PreparedRow::Plan { semantic, .. } = prepared else { return None };
                if !joined {
                    return Some(Err(ArrowOrderRefusal::MissingViewerArrowLifecycle {
                        family: ArrowFamily::Orders, row }));
                }
                let secondary = match viewer.secondary(&prepared_row_key(prepared), ArrowFamily::Orders, row) {
                    Ok(value) => value,
                    Err(reason) => return Some(Err(reason)),
                };
                if frame.plan_draw_inputs(&semantic.key).is_none() {
                    return Some(Err(ArrowOrderRefusal::MissingNativePlanDrawInputs { row }));
                }
                let definition = frame.plan_definitions.iter().find(|d| d.name == semantic.resolved_definition_name);
                Some(resolve_arrow(ArrowFamily::Orders, row, definition.map(|d| (d.name.as_str(), d.definition_index,
                    d.body_width_f32_bits.map(f32::from_bits), f32::from_bits(d.width_scale_f32_bits))),
                    semantic.manager_insertion_ordinal, f32::from_bits(semantic.object_width_multiplier_f32_bits), secondary))
            }).collect::<Vec<_>>();
            if let Some(runtime) = &frame.portable.viewer_runtime {
                candidates.extend(runtime.refusals.iter().cloned()
                    .map(|refusal| Err(ArrowOrderRefusal::MissingDerivedPlanInput(refusal))));
            }
            candidates
        });
        let supply_rows = supply.candidates(|frame| Self::supply_candidates(
            &frame.supply_mesh.ordered_draws, &frame.supply_definitions));
        Self::resolve_candidates(order_rows, supply_rows)
    }

    fn supply_candidates(draws: &[portable_render_stream::RailSupplyMapArrowMeshDraw],
        definitions: &[crate::PreparedXmap16SupplyArrowDefinition])
        -> Vec<Result<ArrowSubmission, ArrowOrderRefusal>> {
        draws.iter().enumerate().map(|(row, draw)| {
            let name = match draw.definition {
                portable_render_stream::RailSupplyArrowDefinition::RailwayMapArrow => "railway_map_arrow",
                portable_render_stream::RailSupplyArrowDefinition::RailwayMapArrowFlowing => "railway_map_arrow_flowing",
                portable_render_stream::RailSupplyArrowDefinition::RailwayMapArrowFlowingReverse => "railway_map_arrow_flowing_reverse",
            };
            let definition = definitions.iter().find(|definition| definition.name == name);
            let mut missing = Vec::new();
            if definition.is_none() { missing.push(SupplySortInput::Definition); }
            // CMapArrowDefinition ctor 14124ff40 initializes getter field +0xc8 to +0.
            let scale = definition.map(|d| match d.width_scale { Some(scale) => scale, None => 0.0 });
            // Mesh enumeration has no admitted join to native pool registration order.
            missing.push(SupplySortInput::EngineRegistrationOrdinal);
            let inputs = SupplyArrowSortInputs {
                mesh_row_ordinal: draw.row_ordinal, definition_name: name.into(),
                definition_index: definition.map(|d| d.definition_index),
                body_width_bits: definition.map(|d| d.body_width.map(f32::to_bits)),
                width_scale_bits: definition.and_then(|d| d.width_scale).map(f32::to_bits),
                effective_width_scale_bits: scale.map(f32::to_bits),
                object_width_bits: definition.map(|d| d.object_width_multiplier_bits),
                key: definition.zip(scale).map(|(d, scale)| ArrowSortKey::from_operands(
                    d.body_width, scale, f32::from_bits(d.object_width_multiplier_bits), 0.0)),
            };
            Err(ArrowOrderRefusal::MissingSupplySortInputs { row, inputs, missing })
        }).collect()
    }

    pub fn execution_batches(&self, movement_rows: Option<&[usize]>, supply_present: bool) -> Vec<ArrowSubmissionBatch> {
        let mut batches = self.batches();
        if let Some(rows) = movement_rows {
            if !rows.is_empty() || !self.submissions.iter().any(|r| r.family == ArrowFamily::Orders) {
                batches.push(ArrowSubmissionBatch { definition_index: None, outer_pass: false,
                    family: ArrowFamily::Orders, rows: rows.to_vec() });
            }
        }
        if supply_present && !self.submissions.iter().any(|r| r.family == ArrowFamily::Supply) {
            batches.push(ArrowSubmissionBatch { definition_index: None, outer_pass: false,
                family: ArrowFamily::Supply, rows: Vec::new() });
        }
        batches
    }

    pub fn batches(&self) -> Vec<ArrowSubmissionBatch> {
        let mut batches = Vec::<ArrowSubmissionBatch>::new();
        let mut start = 0;
        while start < self.submissions.len() {
            let index = self.submissions[start].definition_index;
            let end = start + self.submissions[start..].iter().take_while(|r| r.definition_index == index).count();
            for outer_pass in [false, true] {
                for row in &self.submissions[start..end] {
                    if let Some(last) = batches.last_mut().filter(|last| last.definition_index == Some(index)
                        && last.outer_pass == outer_pass && last.family == row.family) {
                        last.rows.push(row.row);
                    } else {
                        batches.push(ArrowSubmissionBatch { definition_index: Some(index), outer_pass,
                            family: row.family, rows: vec![row.row] });
                    }
                }
            }
            start = end;
        }
        batches
    }
}

fn stable_arrow_sort(rows: &mut [ArrowSubmission]) {
    if rows.len() <= 32 {
        for i in 1..rows.len() {
            let mut j = i;
            while j > 0 && rows[j].key.precedes(rows[j - 1].key) {
                rows.swap(j, j - 1);
                j -= 1;
            }
        }
        return;
    }
    let middle = rows.len().div_ceil(2);
    stable_arrow_sort(&mut rows[..middle]);
    stable_arrow_sort(&mut rows[middle..]);
    let left = rows[..middle].to_vec();
    let (mut a, mut b, mut output) = (0, middle, 0);
    while a < left.len() && b < rows.len() {
        if rows[b].key.precedes(left[a].key) {
            rows[output] = rows[b].clone();
            b += 1;
        } else {
            rows[output] = left[a].clone();
            a += 1;
        }
        output += 1;
    }
    rows[output..output + left.len() - a].clone_from_slice(&left[a..]);
}

pub(crate) fn validate_arrow_row_selection(rows: &[usize], length: usize) -> crate::Result<()> {
    let mut seen = BTreeSet::new();
    if rows.iter().any(|row| *row >= length || !seen.insert(*row)) {
        return Err("arrow-order:invalid-native-row-selection".into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn row(family: ArrowFamily, ordinal: u32, bucket: u32, primary: f32, secondary: f32) -> ArrowSubmission {
        ArrowSubmission { family, row: ordinal as usize, definition_index: bucket,
            definition_name: format!("definition-{bucket}"), registration_ordinal: Some(ordinal),
            key: ArrowSortKey { primary_bits: primary.to_bits(), secondary_bits: secondary.to_bits() } }
    }

    pub(crate) fn prepared_orders(sequence: u64, populated: bool, selected: Option<usize>) -> PreparedXmap15Frame {
        use crate::production_features::xmap15::*;
        use portable_render_stream::*;
        let content = |path: &str| UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            stored_game_relative_path: path.to_ascii_lowercase(),
            normalized_game_relative_path: path.to_ascii_lowercase(),
            winning_layer_ordinal: 0,
            layer_kind: portable_content_manifest::ContentLayerKind::Base,
            layer_id: "base".into(), source_sha256: "b".repeat(64),
            payload_sha256: "c".repeat(64), payload_bytes: 1,
        };
        let zero = 0.0f32.to_bits();
        let one = 1.0f32.to_bits();
        let mut definition = PreparedXmap15PlanDefinition {
            name: "offence_line".into(), definition_index: 3, technique: "MapArrowNoHead".into(),
            body_width_f32_bits: [one; 2], body_uv_scale_f32_bits: one,
            body_uv_scale_2_f32_bits: None, mask_inner_f32_bits: [one; 3],
            mask_outer_f32_bits: [one; 3], width_scale_f32_bits: zero,
            width_scale_max_f32_bits: None, head_length_f32_bits: None,
            texture_loop_f32_bits: None, fade_in_f32_bits: zero, fade_out_f32_bits: zero,
            global_scale_min_size_f32_bits: one, global_scale_max_points: 1,
            texture_anim_speed_f32_bits: None, texture_variant_count: Some(1),
            culling_direction: None, draw_last: false, draw_shadows: false,
            ordered_texture_masks: vec!["gfx/maparrows/m.dds".into()],
            ordered_texture_patterns: vec!["gfx/maparrows/p.dds".into()],
            ordered_shader_translation_unit_paths: vec![XMAP15_MAPARROW_SHADER_PATH.into()],
            ordered_used_content: [XMAP15_MAPARROWS_PATH, XMAP15_MAPARROW_SHADER_PATH,
                "gfx/maparrows/m.dds", "gfx/maparrows/p.dds"].map(content).to_vec(),
            deterministic_sha256: String::new(),
        };
        definition.deterministic_sha256 = snapshot::sha256::Sha256::hex(
            &serde_json::to_vec(&serde_json::to_value(&definition).unwrap()).unwrap());
        let semantic = Xmap15PlanSemanticRow {
            key: Xmap15PlanKey {
                owner_kind: Xmap15OwnerKind::Army,
                owner_id: 7,
                engine_order_instance_id: 9,
                generation: 2,
                slot_index: 0,
            },
            owner_country: "GER".into(),
            selector: Xmap15PlanDefinitionSelector {
                order_type: 2,
                move_special_118: false,
                withdraw_119: false,
                fallback_298: false,
                virtual_299: false,
                blitz_118: false,
                army_group_style_39_and_378: false,
            },
            resolved_definition_name: "offence_line".into(),
            ordered_assigned_division_ids: vec![11],
            assigned: true,
            active: true,
            route: Xmap15PreparedRoute {
                ordered_province_ids: vec![1, 2],
                builder_input_points_fixed5: vec![[0, 0], [100_000, 0]],
                engine_pre_smoother_points_fixed5: None,
                point_width_classes: vec![0, 0],
                explicit_height_f32_bits: None,
                wrap_offset_world: 0,
            },
            manager_insertion_ordinal: Some(0),
            definition_group_ordinal: Some(0),
            object_width_multiplier_f32_bits: 0.0f32.to_bits(),
            comparator_tie_17c_f32_bits: 0.0f32.to_bits(),
            fade_in_f32_bits: 0.0f32.to_bits(),
            fade_out_f32_bits: 0.0f32.to_bits(),
            culling_offset_f32_bits: 0.0f32.to_bits(),
            inner_rgba_f32_bits: [0.0f32.to_bits(); 4],
            outer_rgba_f32_bits: [0.0f32.to_bits(); 4],
            texture_variant: 0,
            see_through: false,
            visible: true,
            final_engine_draw_ordinal: Some(0),
            exact_head_vertices_f32_bits: None,
            lifecycle_tombstone_sha256: Some("a".repeat(64)),
        };
        let ordered_rows = if populated { (0..2).map(|index| {
            let mut semantic = semantic.clone();
            semantic.key.slot_index = index;
            semantic.manager_insertion_ordinal = Some(index);
            semantic.final_engine_draw_ordinal = Some(index);
            Xmap15PreparedRow::Plan { semantic, definition_sha256: definition.deterministic_sha256.clone() }
        }).collect::<Vec<_>>() } else { Vec::new() };
        let selected_plan = selected.map(|index| match &ordered_rows[index] {
            Xmap15PreparedRow::Plan { semantic, .. } => semantic.key.clone(),
            _ => unreachable!(),
        });
        let heightmap = populated.then(|| content(XMAP15_HEIGHTMAP_PATH));
        let map_dimensions_content = populated.then(|| content(XMAP15_MAP_DIMENSIONS_PATH));
        let plan_definitions = if populated { vec![definition] } else { Vec::new() };
        let ordered_used_content = plan_definitions.iter().flat_map(|d| d.ordered_used_content.iter().cloned())
            .chain(heightmap.iter().cloned()).chain(map_dimensions_content.iter().cloned())
            .collect::<BTreeSet<_>>().into_iter().collect();
        let viewer_runtime = Some(portable_render_stream::Xmap15ViewerRuntimeFrame {
            retained_plans: ordered_rows.iter().filter_map(|row| match row {
                Xmap15PreparedRow::Plan { semantic, .. } => Some(semantic.key.clone()), _ => None }).collect(),
            draw_inputs: ordered_rows.iter().filter_map(|row| match row {
                Xmap15PreparedRow::Plan { semantic, .. } => Some(portable_render_stream::Xmap15ViewerPlanDrawInputs {
                    key: semantic.key.clone(), base_rgba_f32_bits: semantic.inner_rgba_f32_bits,
                    inner_highlight: false, animation_speed_f32_bits: 0 }), _ => None }).collect(),
            refusals: Vec::new(),
        });
        let frame = PreparedXmap15Frame {
            portable: Xmap15PreparedFrame {
                schema: XMAP15_STATE_FIRST_SCHEMA.into(), hour_index: 7, observation_sequence: sequence,
                semantic_input_sha256: "a".repeat(64), enumerator_witness_sha256: "b".repeat(64),
                xmap13_census_sha256: "c".repeat(64), xmap13_admission_sha256: "d".repeat(64),
                selected_plan, viewer_runtime, ordered_rows, ordered_used_content, deterministic_sha256: String::new(),
            }.seal().unwrap(),
            assigned_stage: AssignedStage::seal("f".repeat(64), XMAP15_ASSIGNED_STAGE_NAME,
                XMAP15_ASSIGNED_OPERATION_NAME, 4).unwrap(),
            foundation_target_sha256: "e".repeat(64), plan_definitions, movement_definitions: Vec::new(),
            paradrop_profile: None,
            heightmap, map_dimensions_content, map_width: if populated { 2 } else { 0 },
            map_height: if populated { 2 } else { 0 },
        };
        frame.validate().unwrap();
        frame
    }

    #[test]
    fn prepared_frame_admits_derived_terrain_heads_and_refuses_missing_callback_inputs() {
        let mut frame = prepared_orders(1, true, None);
        let definition = &mut frame.plan_definitions[0];
        definition.technique = "MapArrowDefault".into();
        definition.head_length_f32_bits = Some(0.25f32.to_bits());
        definition.deterministic_sha256.clear();
        definition.deterministic_sha256 = snapshot::sha256::Sha256::hex(
            &serde_json::to_vec(&serde_json::to_value(&definition).unwrap()).unwrap());
        let digest = definition.deterministic_sha256.clone();
        for row in &mut frame.portable.ordered_rows {
            let Xmap15PreparedRow::Plan { semantic, definition_sha256 } = row else { unreachable!() };
            assert!(semantic.exact_head_vertices_f32_bits.is_none());
            *definition_sha256 = digest.clone();
        }
        frame.portable = frame.portable.seal().unwrap();
        frame.validate().unwrap();
        let profile = frame.plan_definitions[0].geometry_profile().unwrap();
        let Xmap15PreparedRow::Plan { semantic, .. } = &frame.portable.ordered_rows[0] else { unreachable!() };
        let mesh = semantic.build_map_arrow_mesh(profile, frame.map_width, |_,_| Ok(0.0)).unwrap();
        assert_eq!(mesh.head_first_vertex, Some(0));
        assert_eq!(mesh.body_vertex_count, 4);
        assert_eq!(mesh.vertices.len(), 8);
        assert!(mesh.vertices[..4].iter().all(|vertex| vertex.uv_is_head_variant[2] == 1.0));
        let mut missing_map = frame.clone();
        missing_map.heightmap = None;
        assert!(missing_map.validate().is_err());
        let mut missing_width = frame.clone();
        missing_width.map_width = 0;
        assert!(missing_width.validate().is_err());
        let Xmap15PreparedRow::Plan { semantic, .. } = &mut frame.portable.ordered_rows[0] else { unreachable!() };
        semantic.route.explicit_height_f32_bits = Some(vec![0; semantic.route.builder_input_points_fixed5.len()]);
        frame.portable = frame.portable.seal().unwrap();
        assert!(frame.validate().unwrap_err().to_string().contains("callback lacks exact head inputs"));
    }

    #[test]
    fn prepared_paradrop_profile_admits_mesh_and_lifecycle_submission() {
        let mut frame = prepared_orders(1, true, None);
        let definition = &mut frame.plan_definitions[0];
        definition.name = "paradrop".into();
        definition.head_length_f32_bits = Some(0.25f32.to_bits());
        definition.deterministic_sha256.clear();
        definition.deterministic_sha256 = snapshot::sha256::Sha256::hex(
            &serde_json::to_vec(&serde_json::to_value(&definition).unwrap()).unwrap());
        let digest = definition.deterministic_sha256.clone();
        for row in &mut frame.portable.ordered_rows {
            let Xmap15PreparedRow::Plan { semantic, definition_sha256 } = row else { unreachable!() };
            semantic.selector.order_type = 4;
            semantic.resolved_definition_name = "paradrop".into();
            *definition_sha256 = digest.clone();
        }
        frame.portable = frame.portable.seal().unwrap();
        assert!(frame.validate().unwrap_err().to_string().contains("paradrop profile closure"));
        let mut content = frame.heightmap.as_ref().unwrap().clone();
        content.normalized_game_relative_path = "common/defines/00_graphics.lua".into();
        content.stored_game_relative_path = content.normalized_game_relative_path.clone();
        let parameters = portable_render_stream::Xmap15ParadropProfile {
            height_to_length_f32_bits: 0.5f32.to_bits(), max_height_f32_bits: 10.0f32.to_bits(),
        };
        frame.paradrop_profile = Some(crate::production_features::xmap15::PreparedXmap15ParadropProfile {
            parameters, ordered_used_content: vec![content.clone()],
        });
        frame.portable.ordered_used_content.push(content);
        frame.portable.ordered_used_content.sort();
        frame.portable = frame.portable.seal().unwrap();
        frame.validate().unwrap();
        let profile = frame.plan_definitions[0].geometry_profile().unwrap();
        let Xmap15PreparedRow::Plan { semantic, .. } = &frame.portable.ordered_rows[0] else { unreachable!() };
        let mesh = semantic.build_paradrop_map_arrow_mesh(profile, frame.map_width, parameters, |_, _| Ok(0.0)).unwrap();
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.head_first_vertex, Some(0));
        assert!(mesh.vertices.iter().any(|vertex| vertex.position[1] > 0.0));
        let mut lifecycle = crate::Xmap15LifecycleState::default();
        let witness = lifecycle.apply(&frame).unwrap();
        let order = ArrowSubmissionOrder::from_frames(ArrowFrameInput::Present(&frame),
            ArrowFrameInput::ViewInactive, &mut ArrowViewerSortState::default(), Some(&witness), &"a".repeat(64));
        assert_eq!(order.submissions.len(), 2);
        assert!(order.refusals.is_empty());
        frame.paradrop_profile.as_mut().unwrap().ordered_used_content.clear();
        assert!(frame.validate().unwrap_err().to_string().contains("authenticated defines closure"));
    }

    #[test]
    fn prepared_viewer_rows_without_native_ordinals_submit_unique_keys_and_refuse_ties() {
        let mut frame = prepared_orders(1, true, None);
        let definition = &mut frame.plan_definitions[0];
        definition.width_scale_f32_bits = 1.0f32.to_bits();
        definition.deterministic_sha256.clear();
        definition.deterministic_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(&serde_json::to_value(&*definition).unwrap()).unwrap());
        for (index, row) in frame.portable.ordered_rows.iter_mut().enumerate() {
            let Xmap15PreparedRow::Plan { semantic, definition_sha256 } = row else { unreachable!() };
            semantic.manager_insertion_ordinal = None;
            semantic.definition_group_ordinal = None;
            semantic.final_engine_draw_ordinal = None;
            semantic.object_width_multiplier_f32_bits = (index as f32).to_bits();
            *definition_sha256 = frame.plan_definitions[0].deterministic_sha256.clone();
        }
        frame.portable = frame.portable.seal().unwrap();
        frame.validate().unwrap();
        let mut lifecycle = crate::Xmap15LifecycleState::default();
        let mut viewer = ArrowViewerSortState::default();
        let witness = lifecycle.apply(&frame).unwrap();
        let result = ArrowSubmissionOrder::from_frames(ArrowFrameInput::Present(&frame),
            ArrowFrameInput::ViewInactive, &mut viewer, Some(&witness), &"a".repeat(64));
        assert_eq!(result.submissions.len(), 2);
        assert!(result.refusals.is_empty());
        assert!(result.submissions.iter().all(|row| row.registration_ordinal.is_none()));
        assert_eq!(result.submissions.iter().map(|row| row.row).collect::<BTreeSet<_>>(), [0, 1].into_iter().collect());
        for row in &mut frame.portable.ordered_rows {
            let Xmap15PreparedRow::Plan { semantic, .. } = row else { unreachable!() };
            semantic.object_width_multiplier_f32_bits = 0.0f32.to_bits();
        }
        frame.portable.observation_sequence += 1;
        frame.portable = frame.portable.seal().unwrap();
        let witness = lifecycle.apply(&frame).unwrap();
        let result = ArrowSubmissionOrder::from_frames(ArrowFrameInput::Present(&frame),
            ArrowFrameInput::ViewInactive, &mut viewer, Some(&witness), &"a".repeat(64));
        assert!(result.submissions.is_empty());
        assert_eq!(result.refusals.iter().map(|reason| match reason {
            ArrowOrderRefusal::MissingRegistrationOrdinal { family: ArrowFamily::Orders, row } => *row,
            other => panic!("unexpected refusal: {other:?}"),
        }).collect::<BTreeSet<_>>(), [0, 1].into_iter().collect());
    }

    #[test]
    fn prepared_frame_lifecycle_join_preserves_selection_until_witnessed_deletion() {
        let mut lifecycle = crate::Xmap15LifecycleState::default();
        let mut viewer = ArrowViewerSortState::default();
        let submit = |viewer: &mut ArrowViewerSortState, frame: &PreparedXmap15Frame,
            witness: Option<&Xmap15LifecycleUpdate>| ArrowSubmissionOrder::from_frames(
                ArrowFrameInput::Present(frame), ArrowFrameInput::ViewInactive,
                viewer, witness, &"a".repeat(64));
        let fresh = prepared_orders(1, true, None);
        let created = lifecycle.apply(&fresh).unwrap();
        assert_eq!(created.created.len(), 2);
        let missing = submit(&mut viewer, &fresh, None);
        assert_eq!(missing.refusals, (0..2).map(|row|
            ArrowOrderRefusal::MissingViewerArrowLifecycle { family: ArrowFamily::Orders, row }).collect::<Vec<_>>());
        assert!(missing.submissions.is_empty());
        let admitted = submit(&mut viewer, &fresh, Some(&created));
        assert!(admitted.refusals.is_empty());
        assert_eq!(admitted.submissions.len(), 2);
        assert!(admitted.submissions.iter().all(|row| row.key.secondary_bits == 0.0f32.to_bits()));

        let selected = prepared_orders(2, true, Some(0));
        let selection = lifecycle.apply(&selected).unwrap();
        assert!(selection.selected_plan_changed);
        let assert_selected = |result: ArrowSubmissionOrder| {
            assert_eq!(result.submissions.iter().map(|row| row.row).collect::<Vec<_>>(), [1]);
            assert_eq!(result.refusals, [ArrowOrderRefusal::ViewerInteractionSortKeyUnproven {
                family: ArrowFamily::Orders, row: 0 }]);
            assert_eq!(result.submissions.len() + result.refusals.len(), 2);
        };
        assert_selected(submit(&mut viewer, &selected, Some(&selection)));
        assert_selected(submit(&mut viewer, &selected, Some(&selection)));
        let unavailable = ArrowSubmissionOrder::from_frames(ArrowFrameInput::Unavailable,
            ArrowFrameInput::ViewInactive, &mut viewer, None, &"a".repeat(64));
        assert_eq!(unavailable.refusals, [ArrowOrderRefusal::MissingOrderSemanticCarrier]);
        let inactive = ArrowSubmissionOrder::from_frames(ArrowFrameInput::ViewInactive,
            ArrowFrameInput::ViewInactive, &mut viewer, None, &"a".repeat(64));
        assert!(inactive.refusals.is_empty() && inactive.submissions.is_empty());
        assert_selected(submit(&mut viewer, &selected, Some(&selection)));

        let deselected = prepared_orders(3, true, None);
        let deselection = lifecycle.apply(&deselected).unwrap();
        assert_selected(submit(&mut viewer, &deselected, Some(&deselection)));
        let mut corrupt = deselected.clone();
        corrupt.portable.deterministic_sha256 = "0".repeat(64);
        let refused = submit(&mut viewer, &corrupt, Some(&deselection));
        assert!(refused.submissions.is_empty());
        assert_eq!(refused.refusals.len(), 2);
        assert_selected(submit(&mut viewer, &deselected, Some(&deselection)));

        let mut other_session = ArrowViewerSortState::default();
        let unwitnessed = submit(&mut other_session, &deselected, Some(&deselection));
        assert!(unwitnessed.submissions.is_empty());
        assert_eq!(unwitnessed.refusals.len(), 2);
        let empty = prepared_orders(4, false, None);
        let deletion = lifecycle.apply(&empty).unwrap();
        assert_eq!(deletion.deleted.len(), 2);
        assert!(deletion.authenticated_empty);
        let removed = submit(&mut viewer, &empty, Some(&deletion));
        assert!(removed.submissions.is_empty() && removed.refusals.is_empty());
        let recreated = prepared_orders(5, true, None);
        let creation = lifecycle.apply(&recreated).unwrap();
        assert_eq!(creation.created.len(), 2);
        let recreated = submit(&mut viewer, &recreated, Some(&creation));
        assert_eq!(recreated.submissions.len(), 2);
        assert!(recreated.refusals.is_empty());
        assert!(recreated.submissions.iter().all(|row| row.key.secondary_bits == 0.0f32.to_bits()));
    }

    #[test]
    fn omitted_supply_scale_is_the_pinned_definition_constructor_value() {
        let definition = portable_engine_evidence::map_scene_residuals::DECOMPILES.iter()
            .find(|(address, _)| *address == "14124ff40").unwrap().1;
        assert!(definition.contains("*(undefined4 *)(param_1 + 0x19) = 0;"));
        let parser = portable_engine_evidence::map_scene_residuals::DECOMPILES.iter()
            .find(|(address, _)| *address == "141255760").unwrap().1;
        assert!(parser.contains("case 0x2efb:"));
        assert!(parser.contains("param_1 = param_1 + 200;"));
        let getter = portable_engine_evidence::arrow_manager_buckets::DECOMPILES.iter()
            .find(|(address, _)| *address == "141253890").unwrap().1;
        assert!(getter.contains("*(float *)(lVar1 + 200)"));
    }

    #[test]
    fn supply_refusals_preserve_available_operands_and_account_for_each_draw() {
        use portable_render_stream::{RailSupplyArrowDefinition, RailSupplyMapArrowMeshDraw};
        let mut definition = crate::PreparedXmap16SupplyArrowDefinition {
            name: "railway_map_arrow".into(), definition_index: 4, technique: "MapArrowNoHead".into(),
            body_width: [2.0, 3.0], body_uv_scale: 1.0, body_uv_scale_2: None,
            width_scale: None, width_scale_max: None, texture_variant_count: None,
            global_scale_min_size: 1.0, global_scale_max_points: 1,
            texture_loop: 1.0, texture_anim_speed: 1.0, culling_direction: 0, draw_last: false, draw_shadows: false,
            mask_inner: [0.0; 4], mask_outer: [0.0; 4], fade_in: 0.0, fade_out: 0.0,
            texture_mask_paths: std::array::from_fn(|index| format!("gfx/m{index}.dds")),
            texture_pattern_paths: std::array::from_fn(|index| format!("gfx/p{index}.dds")),
            ordered_used_content: Vec::new(), object_width_multiplier_bits: 0.0f32.to_bits(),
            vertex_texture_variant_bits: 0.0f32.to_bits(), deterministic_sha256: String::new(),
        };
        let draw = |row_ordinal, definition| RailSupplyMapArrowMeshDraw {
            row_ordinal, first_vertex: 0, vertex_count: 4, definition, texture_slot: 0,
            selected_rgba_f32_bits: [0; 4], wrap_offset_world: 0, total_length_f32_bits: 1.0f32.to_bits(),
        };
        for mesh_ordinals in [[0, 1, 2], [99, 7, 0]] {
            let draws = [draw(mesh_ordinals[0], RailSupplyArrowDefinition::RailwayMapArrow),
                draw(mesh_ordinals[1], RailSupplyArrowDefinition::RailwayMapArrowFlowing),
                draw(mesh_ordinals[2], RailSupplyArrowDefinition::RailwayMapArrowFlowingReverse),
                draw(mesh_ordinals[0], RailSupplyArrowDefinition::RailwayMapArrow),
                draw(mesh_ordinals[1], RailSupplyArrowDefinition::RailwayMapArrowFlowing),
                draw(mesh_ordinals[2], RailSupplyArrowDefinition::RailwayMapArrowFlowingReverse)];
            for width_scale in [None, Some(0.0), Some(-0.0), Some(2.0)] {
                definition.width_scale = width_scale;
                let mut flowing = definition.clone();
                flowing.name = "railway_map_arrow_flowing".into();
                flowing.definition_index = 5;
                flowing.width_scale = Some(-2.0);
                let definitions = [definition.clone(), flowing];
                let candidates = ArrowSubmissionOrder::supply_candidates(&draws, &definitions);
                let order = row(ArrowFamily::Orders, 0, 3, 4.0, 0.0);
                let result = ArrowSubmissionOrder::resolve_candidates(Some(vec![Ok(order.clone())]), Some(candidates));
                assert_eq!(result.submissions, [order]);
                assert_eq!(result.refusals.len(), draws.len());
                assert_eq!(result.refusals.iter().map(|refusal| match refusal {
                    ArrowOrderRefusal::MissingSupplySortInputs { row, .. } => *row,
                    other => panic!("unexpected supply refusal: {other:?}"),
                }).collect::<BTreeSet<_>>(), (0..draws.len()).collect());
                assert_eq!(result.submissions.len() + result.refusals.len(), draws.len() + 1);
                let singleton = ArrowSubmissionOrder::resolve_candidates(Some(vec![]),
                    Some(ArrowSubmissionOrder::supply_candidates(&draws[..3], &definitions)));
                assert_eq!(singleton.submissions.iter().map(|row| (row.family, row.row,
                    row.definition_index, row.registration_ordinal)).collect::<Vec<_>>(),
                    [(ArrowFamily::Supply, 0, 4, None), (ArrowFamily::Supply, 1, 5, None)]);
                assert_eq!(singleton.refusals.len(), 1);
                assert!(matches!(&singleton.refusals[0],
                    ArrowOrderRefusal::MissingSupplySortInputs { row: 2, .. }));
                for refusal in &result.refusals {
                    let ArrowOrderRefusal::MissingSupplySortInputs { row, inputs, missing } = refusal else {
                        panic!("supply refusal lost its diagnostic inputs: {refusal:?}");
                    };
                    let index = *row % 3;
                    assert_eq!(inputs.mesh_row_ordinal, mesh_ordinals[index]);
                    if index == 2 {
                        assert_eq!(missing, &[SupplySortInput::Definition, SupplySortInput::EngineRegistrationOrdinal]);
                        assert_eq!(inputs.definition_name, "railway_map_arrow_flowing_reverse");
                        assert_eq!(inputs.definition_index, None);
                        assert_eq!(inputs.body_width_bits, None);
                        assert_eq!(inputs.width_scale_bits, None);
                        assert_eq!(inputs.effective_width_scale_bits, None);
                        assert_eq!(inputs.object_width_bits, None);
                        assert_eq!(inputs.key, None);
                    } else {
                        let d = &definitions[index];
                        assert_eq!(inputs.definition_name, d.name);
                        assert_eq!(inputs.definition_index, Some(d.definition_index));
                        assert_eq!(inputs.body_width_bits, Some([2.0f32.to_bits(), 3.0f32.to_bits()]));
                        assert_eq!(inputs.width_scale_bits, d.width_scale.map(f32::to_bits));
                        assert_eq!(inputs.object_width_bits, Some(0.0f32.to_bits()));
                        assert_eq!(inputs.effective_width_scale_bits,
                            Some(d.width_scale.map(f32::to_bits).unwrap_or(0)));
                        assert_eq!(missing, &[SupplySortInput::EngineRegistrationOrdinal]);
                        assert_eq!(inputs.key, Some(ArrowSortKey {
                            primary_bits: 3.0f32.to_bits(), secondary_bits: 0.0f32.to_bits() }));
                    }
                }
            }
        }
    }

    fn lifecycle(created: &[&str], deleted: &[&str], selected_changed: bool, empty: bool) -> Xmap15LifecycleUpdate {
        let mut update = Xmap15LifecycleUpdate {
            hour_index: 7, observation_sequence: 9,
            created: created.iter().map(|key| (*key).into()).collect(),
            edited: Vec::new(), deleted: deleted.iter().map(|key| (*key).into()).collect(),
            selected_plan_changed: selected_changed, authenticated_empty: empty,
            deterministic_sha256: String::new(),
        };
        update.deterministic_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(&update).unwrap());
        update
    }

    fn observe(state: &mut ArrowViewerSortState, keys: &[&str], selected: Option<&str>,
        update: &Xmap15LifecycleUpdate) -> bool {
        let prepared = snapshot::sha256::Sha256::hex(&serde_json::to_vec(&(keys, selected)).unwrap());
        state.observe(&"a".repeat(64), &prepared, 7, 9,
            keys.iter().map(|key| (*key).into()).collect(), selected.map(str::to_owned), update)
    }

    #[test]
    fn viewer_secondary_requires_creation_and_retains_uncertainty_until_witnessed_deletion() {
        let mut state = ArrowViewerSortState::default();
        let secondary = |state: &ArrowViewerSortState, key: &str, row|
            state.secondary(key, ArrowFamily::Orders, row);
        assert_eq!(secondary(&state, "a", 0),
            Err(ArrowOrderRefusal::MissingViewerArrowLifecycle { family: ArrowFamily::Orders, row: 0 }));
        assert!(!observe(&mut state, &["a", "b"], None, &lifecycle(&[], &[], false, false)));
        let creation = lifecycle(&["a", "b"], &[], false, false);
        assert!(observe(&mut state, &["a", "b"], None, &creation));
        assert_eq!(secondary(&state, "a", 0), Ok(0.0));
        assert_eq!(secondary(&state, "b", 1), Ok(0.0));
        let selection = lifecycle(&[], &[], true, false);
        assert!(observe(&mut state, &["a", "b"], Some("a"), &selection));
        let refused = Err(ArrowOrderRefusal::ViewerInteractionSortKeyUnproven {
            family: ArrowFamily::Orders, row: 0 });
        assert_eq!(secondary(&state, "a", 0), refused);
        assert_eq!(secondary(&state, "b", 1), Ok(0.0));
        assert!(observe(&mut state, &["a", "b"], Some("a"), &selection));
        assert_eq!(secondary(&state, "a", 0), refused);
        let missing = ArrowFrameInput::<()>::Unavailable.candidates(|_| panic!("missing census resolved"));
        assert!(missing.is_none());
        assert_eq!(secondary(&state, "a", 0), refused);
        assert!(observe(&mut state, &["a", "b"], None, &lifecycle(&[], &[], true, false)));
        assert_eq!(secondary(&state, "a", 0), refused);
        assert!(observe(&mut state, &["b"], None, &lifecycle(&[], &["a"], false, false)));
        assert!(observe(&mut state, &["a", "b"], None, &lifecycle(&["a"], &[], false, false)));
        assert_eq!(secondary(&state, "a", 0), Ok(0.0));
        assert!(observe(&mut state, &[], None, &lifecycle(&[], &["a", "b"], false, true)));
        assert!(state.retained.is_empty());
        let mut other_session = ArrowViewerSortState::default();
        assert!(observe(&mut other_session, &["a"], None, &lifecycle(&["a"], &[], false, false)));
        assert_eq!(secondary(&other_session, "a", 0), Ok(0.0));
    }

    #[test]
    fn lifecycle_join_rejects_stale_or_inconsistent_witnesses_without_erasing_state() {
        let mut state = ArrowViewerSortState::default();
        let creation = lifecycle(&["a"], &[], true, false);
        assert!(observe(&mut state, &["a"], Some("a"), &creation));
        let before = format!("{state:?}");
        let mut stale = lifecycle(&[], &[], false, false);
        stale.hour_index = 8;
        assert!(!observe(&mut state, &["a"], Some("a"), &stale));
        assert!(!observe(&mut state, &[], None, &lifecycle(&[], &[], true, true)));
        assert!(!observe(&mut state, &["a"], None, &lifecycle(&["a"], &[], true, false)));
        assert!(!observe(&mut state, &["a"], Some("not-present"), &lifecycle(&[], &[], true, false)));
        assert!(!state.observe("", &"b".repeat(64), 7, 9, BTreeSet::new(), None,
            &lifecycle(&[], &["a"], true, true)));
        assert_eq!(format!("{state:?}"), before);
    }

    #[test]
    fn explicit_frame_availability_distinguishes_absence_from_inactive_and_measured_empty() {
        let absent = ArrowSubmissionOrder::resolve_candidates(
            ArrowFrameInput::<()>::Unavailable.candidates(|_| panic!("absent order resolved")),
            ArrowFrameInput::<()>::ViewInactive.candidates(|_| panic!("inactive supply resolved")));
        assert_eq!(absent.refusals, [ArrowOrderRefusal::MissingOrderSemanticCarrier]);
        let empty = ArrowSubmissionOrder::resolve_candidates(
            ArrowFrameInput::Present(&()).candidates(|_| vec![]),
            ArrowFrameInput::<()>::ViewInactive.candidates(|_| panic!("inactive supply resolved")));
        assert!(empty.refusals.is_empty() && empty.submissions.is_empty());
        let active = ArrowSubmissionOrder::resolve_candidates(
            ArrowFrameInput::Present(&()).candidates(|_| vec![Ok(row(ArrowFamily::Orders, 0, 0, 1.0, 0.0))]),
            ArrowFrameInput::<()>::Unavailable.candidates(|_| panic!("absent supply resolved")));
        assert_eq!(active.submissions.len(), 1);
        assert_eq!(active.refusals, [ArrowOrderRefusal::MissingSupplySemanticCarrier]);
    }

    #[test]
    fn definition_buckets_interleave_families_before_sorting_keys() {
        let order = ArrowSubmissionOrder::compose(vec![
            row(ArrowFamily::Orders, 0, 4, 100.0, 0.0),
            row(ArrowFamily::Supply, 1, 2, 1.0, 0.0),
            row(ArrowFamily::Orders, 2, 1, 1.0, 0.0),
            row(ArrowFamily::Orders, 3, 4, 100.0, 1.0),
        ]);
        assert!(order.refusals.is_empty());
        assert_eq!(order.submissions.iter().map(|r| r.row).collect::<Vec<_>>(), [2, 1, 3, 0]);
        let draws = order.batches().into_iter().flat_map(|b| b.rows.into_iter().map(move |r| (b.definition_index, b.outer_pass, r))).collect::<Vec<_>>();
        assert_eq!(draws, [(Some(1), false, 2), (Some(1), true, 2), (Some(2), false, 1),
            (Some(2), true, 1), (Some(4), false, 3), (Some(4), false, 0), (Some(4), true, 3), (Some(4), true, 0)]);
    }

    #[test]
    fn materials_cover_each_bucket_before_the_next_even_for_the_same_family() {
        let order = ArrowSubmissionOrder::compose(vec![row(ArrowFamily::Orders, 0, 0, 1.0, 0.0),
            row(ArrowFamily::Supply, 1, 0, 1.0, 0.0), row(ArrowFamily::Supply, 2, 1, 1.0, 0.0)]);
        let commands = order.batches().into_iter().flat_map(|b| b.rows.into_iter().map(move |r| (r, b.outer_pass))).collect::<Vec<_>>();
        assert_eq!(commands, [(0, false), (1, false), (0, true), (1, true), (2, false), (2, true)]);
    }

    #[test]
    fn refused_rows_keep_execution_batches_for_both_present_families() {
        let all_refused = ArrowSubmissionOrder::compose(vec![row(ArrowFamily::Orders, 0, 0, f32::NAN, 0.0),
            row(ArrowFamily::Supply, 1, 1, f32::NAN, 0.0)]);
        assert_eq!(all_refused.refusals.len(), 2);
        let batches = all_refused.execution_batches(Some(&[]), true);
        assert_eq!(batches.len(), 2);
        assert!(batches.iter().all(|b| b.rows.is_empty()));
        assert_eq!(batches.iter().map(|b| b.family).collect::<Vec<_>>(), [ArrowFamily::Orders, ArrowFamily::Supply]);
        let mixed = ArrowSubmissionOrder::compose(vec![row(ArrowFamily::Orders, 0, 0, 1.0, 0.0),
            row(ArrowFamily::Orders, 1, 0, f32::NAN, 0.0)]);
        assert_eq!(mixed.refusals.len(), 1);
        let batches = mixed.execution_batches(Some(&[2]), false);
        assert_eq!(batches.iter().map(|b| b.rows.clone()).collect::<Vec<_>>(), [vec![0], vec![0], vec![2]]);
        assert!(mixed.execution_batches(None, false).iter().all(|b| b.definition_index.is_some()));
    }

    #[test]
    fn ties_preserve_registration_across_insertion_and_merge_boundaries() {
        for count in [0, 1, 2, 31, 32, 33, 63, 64, 65, 512, 1025] {
            let rows = (0..count).rev().map(|i| row(ArrowFamily::Orders, i, 0, (i % 3) as f32, (i % 2) as f32)).collect();
            let order = ArrowSubmissionOrder::compose(rows);
            assert!(order.refusals.is_empty());
            assert_eq!(order.submissions.len(), count as usize);
            for pair in order.submissions.windows(2) {
                assert!(!pair[1].key.precedes(pair[0].key));
                if !pair[0].key.precedes(pair[1].key) {
                    assert!(pair[0].registration_ordinal < pair[1].registration_ordinal);
                }
            }
        }
    }

    #[test]
    fn unique_finite_keys_need_no_registration_and_ties_keep_the_native_slot_order() {
        let mut rows = vec![
            row(ArrowFamily::Supply, 0, 4, 4.0, 0.0),
            row(ArrowFamily::Orders, 1, 4, 2.0, 1.0),
            row(ArrowFamily::Supply, 2, 4, 2.0, -0.0),
            row(ArrowFamily::Orders, 3, 3, 9.0, 0.0),
        ];
        for row in &mut rows { row.registration_ordinal = None; }
        let expected = [(ArrowFamily::Orders, 3), (ArrowFamily::Supply, 0),
            (ArrowFamily::Orders, 1), (ArrowFamily::Supply, 2)];
        for a in 0..4 { for b in 0..4 { for c in 0..4 { for d in 0..4 {
            let indices = [a, b, c, d];
            if indices.into_iter().collect::<BTreeSet<_>>().len() != 4 { continue; }
            let result = ArrowSubmissionOrder::compose(indices.map(|i| rows[i].clone()).to_vec());
            assert!(result.refusals.is_empty());
            assert_eq!(result.submissions.iter().map(|row| (row.family, row.row)).collect::<Vec<_>>(), expected);
            assert!(result.submissions.iter().all(|row| row.registration_ordinal.is_none()));
        } } } }
        let mut equal = row(ArrowFamily::Orders, 9, 4, 2.0, 0.0);
        equal.registration_ordinal = Some(7);
        rows.push(equal);
        let mut second_known = row(ArrowFamily::Supply, 10, 4, 2.0, -0.0);
        second_known.registration_ordinal = Some(6);
        rows.push(second_known);
        for reverse in [false, true] {
            let mut input = rows.clone();
            if reverse { input.reverse(); }
            let result = ArrowSubmissionOrder::compose(input);
            assert_eq!(result.refusals, [ArrowOrderRefusal::MissingRegistrationOrdinal {
                family: ArrowFamily::Supply, row: 2 }]);
            assert_eq!(result.submissions.iter().map(|row| row.row).collect::<Vec<_>>(), [3, 0, 1, 10, 9]);
            assert_eq!(result.submissions.len() + result.refusals.len(), rows.len());
        }
        rows[4].registration_ordinal = None;
        rows[5].registration_ordinal = None;
        let result = ArrowSubmissionOrder::compose(rows);
        assert_eq!(result.submissions.len(), 3);
        assert_eq!(result.refusals.len(), 3);
        assert_eq!(result.refusals.iter().map(|refusal| match refusal {
            ArrowOrderRefusal::MissingRegistrationOrdinal { family, row } => (*family, *row),
            other => panic!("unexpected refusal: {other:?}"),
        }).collect::<BTreeSet<_>>(), BTreeSet::from([
            (ArrowFamily::Supply, 2), (ArrowFamily::Orders, 9), (ArrowFamily::Supply, 10),
        ]));
    }

    #[test]
    fn assembly_float_predicate_does_not_order_nans_or_signed_zero() {
        let key = |a: f32, b: f32| ArrowSortKey { primary_bits: a.to_bits(), secondary_bits: b.to_bits() };
        for nan in [f32::NAN, f32::from_bits(0x7f800001), f32::from_bits(0xffc00000)] {
            assert!(!key(nan, 1.0).precedes(key(0.0, 0.0)));
            assert!(!key(0.0, 1.0).precedes(key(nan, 0.0)));
            assert!(!key(1.0, nan).precedes(key(1.0, 0.0)));
            assert!(!key(1.0, 0.0).precedes(key(1.0, nan)));
        }
        assert!(!key(-0.0, -0.0).precedes(key(0.0, 0.0)));
        assert!(!key(0.0, 0.0).precedes(key(-0.0, -0.0)));
        assert!(key(-0.0, 1.0).precedes(key(0.0, 0.0)));
        assert!(key(f32::INFINITY, 0.0).precedes(key(1.0, 0.0)));
    }

    #[test]
    fn getter_uses_sse_second_operand_and_separate_multiply_add() {
        let key = ArrowSortKey::from_operands([2.0, 1.0], 3.0, 4.0, 0.0);
        assert_eq!(f32::from_bits(key.primary_bits), 14.0);
        assert!(f32::from_bits(ArrowSortKey::from_operands([f32::NAN, 2.0], 0.0, 0.0, 0.0).primary_bits).is_nan());
        assert_eq!(f32::from_bits(ArrowSortKey::from_operands([2.0, f32::NAN], 0.0, 0.0, 0.0).primary_bits), 2.0);
        assert_eq!(ArrowSortKey::from_operands([-0.0, 0.0], -0.0, 1.0, 0.0).primary_bits, (-0.0f32).to_bits());
    }

    #[test]
    fn malformed_rows_have_typed_refusals_and_never_reach_native_selection() {
        let result = ArrowSubmissionOrder::compose(vec![row(ArrowFamily::Orders, 0, 0, f32::NAN, 0.0)]);
        assert_eq!(result.refusals, [ArrowOrderRefusal::NonFiniteSortKey { family: ArrowFamily::Orders, row: 0 }]);
        assert!(result.submissions.is_empty());
        let result = ArrowSubmissionOrder::compose(vec![row(ArrowFamily::Orders, 0, 0, 1.0, 0.0), row(ArrowFamily::Supply, 0, 0, 1.0, 0.0)]);
        assert_eq!(result.refusals, [ArrowOrderRefusal::DuplicateRegistration { family: ArrowFamily::Orders, row: 0, index: 0, ordinal: 0 },
            ArrowOrderRefusal::DuplicateRegistration { family: ArrowFamily::Supply, row: 0, index: 0, ordinal: 0 }]);
        assert!(result.submissions.is_empty());
        assert!(validate_arrow_row_selection(&[0, 0], 2).is_err());
        assert!(validate_arrow_row_selection(&[2], 2).is_err());
        assert!(validate_arrow_row_selection(&[1, 0], 2).is_ok());
    }

    #[test]
    fn every_duplicate_row_candidate_is_refused_regardless_of_input_order() {
        let original = row(ArrowFamily::Orders, 0, 0, 1.0, 0.0);
        let mut conflicts = vec![original.clone(); 6];
        conflicts[1].definition_index = 2;
        conflicts[1].definition_name = "other-bucket".into();
        conflicts[2].definition_name = "conflicting-name".into();
        conflicts[3].key.primary_bits = 2.0f32.to_bits();
        conflicts[3].key.secondary_bits = 3.0f32.to_bits();
        conflicts[4].registration_ordinal = Some(99);
        conflicts[5].key.primary_bits = f32::NAN.to_bits();
        let independent = vec![
            row(ArrowFamily::Orders, 1, 0, 4.0, 0.0),
            row(ArrowFamily::Supply, 0, 1, 5.0, 0.0),
        ];
        for conflict in conflicts {
            for duplicate_count in [2, 3] {
                let mut input = vec![original.clone(); duplicate_count - 1];
                input.push(conflict.clone());
                input.extend(independent.clone());
                let expected_refusals = vec![
                    ArrowOrderRefusal::DuplicateRow { family: ArrowFamily::Orders, row: 0 };
                    duplicate_count
                ];
                let forward = ArrowSubmissionOrder::compose(input.clone());
                input.reverse();
                let reversed = ArrowSubmissionOrder::compose(input.clone());
                for result in [&forward, &reversed] {
                    assert_eq!(result.submissions, independent);
                    assert_eq!(result.refusals, expected_refusals);
                    assert_eq!(result.submissions.len() + result.refusals.len(), input.len());
                    assert!(!result.submissions.iter().any(|candidate|
                        candidate.family == ArrowFamily::Orders && candidate.row == 0));
                }
                assert_eq!(forward, reversed);
            }
        }
    }

    #[test]
    fn authored_definition_indices_include_unused_buckets_and_ignore_comments() {
        let source = "# arrowType = { name = fake }\narrowType = { name = z }\narrowType = { name = unused }\narrowType = { name = a }";
        let indices = crate::production_features::xmap15::arrow_definition_indices(source).unwrap();
        assert_eq!(indices["z"], 0);
        assert_eq!(indices["unused"], 1);
        assert_eq!(indices["a"], 2);
        assert_eq!(indices.len(), 3);
        assert!(crate::production_features::xmap15::arrow_definition_indices("arrowType = { name = a } arrowType = { name = a }").is_err());
    }

    #[test]
    fn production_resolution_keeps_valid_rows_and_names_each_missing_input() {
        let definition = Some(("arrow", 3, [1.0, 2.0], 4.0));
        let result = ArrowSubmissionOrder::resolve_candidates(Some(vec![
            resolve_arrow(ArrowFamily::Orders, 0, definition, Some(8), 3.0, 0.0),
            resolve_arrow(ArrowFamily::Orders, 1, None, Some(9), 0.0, 0.0),
            resolve_arrow(ArrowFamily::Orders, 2, definition, None, 0.0, 0.0),
        ]), None);
        assert_eq!(result.submissions.len(), 2);
        assert_eq!(result.submissions[0].definition_index, 3);
        assert_eq!(result.submissions[0].key.primary_bits, 14.0f32.to_bits());
        assert_eq!(result.submissions[1].row, 2);
        assert_eq!(result.submissions[1].key.primary_bits, 2.0f32.to_bits());
        assert_eq!(result.submissions[1].registration_ordinal, None);
        assert_eq!(result.refusals, [ArrowOrderRefusal::MissingDefinition { family: ArrowFamily::Orders, row: 1 },
            ArrowOrderRefusal::MissingSupplySemanticCarrier]);
        let empty = ArrowSubmissionOrder::resolve_candidates(Some(vec![]), Some(vec![]));
        assert!(empty.submissions.is_empty() && empty.refusals.is_empty());
    }

    #[test]
    #[ignore = "whole capture 07; opens the recording read-only and spools only to C: scratch"]
    fn capture07_every_hour_has_one_arrow_assignment_or_typed_refusal() {
        struct NeverFolds;
        impl snapshot::CertifiedPrefoldSpoolDecoder for NeverFolds {
            fn decoder_id(&self) -> &'static str { "arroword.packaged-cache.never-folds" }
            fn decode_certified_spool(&self, _: snapshot::CertifiedPrefoldSpoolRequest)
                -> Result<snapshot::PrefoldedSpool, snapshot::PrefoldDecodeError> {
                panic!("capture07 proof requires its certified packaged cache")
            }
        }
        let recording = std::path::Path::new(r"captures\e2e_07\native_session.battleplan");
        let scratch = std::env::temp_dir().join(format!("arroword_capture07_{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let source = replay_core::open_recording_with_spooled_decoder(recording, &scratch, &NeverFolds).unwrap();
        let timeline = replay_core::WorldTimeline::open_with_config(source, replay_core::TimelineConfig::default()).unwrap();
        let hours = timeline.hours().to_vec();
        assert!(!hours.is_empty());
        let mut counts = BTreeMap::<String, usize>::new();
        let mut records = Vec::new();
        let mut arrow_rows = 0usize;
        for hour in &hours {
            let world = timeline.frame(*hour).unwrap();
            let orders = world.xmap15_orders_fronts_movement_semantic().map(|value| {
                let frame: portable_render_stream::Xmap15SemanticFrame = serde_json::from_value(value.clone()).unwrap();
                frame.validate().unwrap();
                frame.plans.iter().enumerate().map(|(row, semantic)| resolve_arrow(ArrowFamily::Orders, row,
                    None, semantic.manager_insertion_ordinal, f32::from_bits(semantic.object_width_multiplier_f32_bits), 0.0))
                    .collect::<Vec<_>>()
            });
            let supply = world.rail_replay().supply_arrows.as_ref().map(|frame| {
                frame.validate().unwrap();
                frame.rows.iter().enumerate().map(|(row, _)| resolve_arrow(ArrowFamily::Supply, row,
                    None, Some(u32::try_from(row).unwrap()), 0.0, 0.0)).collect::<Vec<_>>()
            });
            let measured_rows = orders.as_ref().map_or(0, Vec::len) + supply.as_ref().map_or(0, Vec::len);
            let order = ArrowSubmissionOrder::resolve_candidates(orders, supply);
            let row_refusals = order.refusals.iter().filter(|r| matches!(r, ArrowOrderRefusal::MissingDefinition { .. })).count();
            assert_eq!(order.submissions.len() + row_refusals, measured_rows, "arrow coverage at hour {hour}");
            assert!(order.submissions.is_empty(), "no authenticated content was supplied");
            assert_eq!(order.refusals.iter().filter(|r| **r == ArrowOrderRefusal::MissingOrderSemanticCarrier).count(), usize::from(world.xmap15_orders_fronts_movement_semantic().is_none()));
            assert_eq!(order.refusals.iter().filter(|r| **r == ArrowOrderRefusal::MissingSupplySemanticCarrier).count(), usize::from(world.rail_replay().supply_arrows.is_none()));
            let refusals = order.refusals;
            for refusal in &refusals { *counts.entry(refusal.to_string()).or_default() += 1; }
            arrow_rows += measured_rows;
            records.push(serde_json::json!({"hour": hour, "measured_arrow_rows": measured_rows, "assigned": 0, "refusals": refusals}));
        }
        assert_eq!(records.len(), hours.len());
        let proof = serde_json::json!({"schema":"arroword.capture07.v1", "recording":recording,
            "hours":hours.len(), "measured_arrow_rows":arrow_rows, "assigned":0,
            "refusals":counts, "hour_records":records,
            "content_admission":"No effective content root supplied; a present semantic row requires authenticated definitions and is refused by identity."});
        if let Ok(path) = std::env::var("ARROWORD_PROOF_OUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&proof).unwrap()).unwrap();
        }
        println!("ARROWORD_CAPTURE07 {}", serde_json::json!({"hours":hours.len(), "measured_arrow_rows":arrow_rows, "assigned":0, "refusals":counts}));
        drop(timeline);
        std::fs::remove_dir_all(&scratch).unwrap();
    }
}
