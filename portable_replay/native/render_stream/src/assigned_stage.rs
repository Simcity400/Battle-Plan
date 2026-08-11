//! Authenticated assignment of named production-map stages.
//!
//! Packet implementations declare names and dependency edges. This neutral
//! provider authenticates that topology and derives the only accepted
//! contiguous operation ordinals. Packets never choose a number locally, and
//! the final compositor remains free to supply and validate the complete graph
//! without becoming a compile dependency of every packet.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const ASSIGNED_STAGE_SCHEMA: &str = "hoi4.replay-map-assigned-stage.v1";
pub const ASSIGNED_STAGE_SCHEDULE_SCHEMA: &str =
    "hoi4.replay-map-assigned-stage-schedule.v1";
pub const ASSIGNED_STAGE_TOPOLOGY_SCHEMA: &str =
    "hoi4.replay-map-assigned-stage-topology.v1";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignedStageIdentity {
    pub stage_name: String,
    pub operation_name: String,
}

impl AssignedStageIdentity {
    pub fn new(stage_name: impl Into<String>, operation_name: impl Into<String>) -> Self {
        Self {
            stage_name: stage_name.into(),
            operation_name: operation_name.into(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_name(&self.stage_name, "assigned stage")?;
        validate_name(&self.operation_name, "assigned operation")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignedStageEdge {
    pub before: AssignedStageIdentity,
    pub after: AssignedStageIdentity,
}

impl AssignedStageEdge {
    pub fn new(before: AssignedStageIdentity, after: AssignedStageIdentity) -> Self {
        Self { before, after }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignedStageTopology {
    pub schema: String,
    pub stages: Vec<AssignedStageIdentity>,
    pub edges: Vec<AssignedStageEdge>,
    pub topology_sha256: String,
}

impl AssignedStageTopology {
    pub fn seal(
        stages: Vec<AssignedStageIdentity>,
        edges: Vec<AssignedStageEdge>,
    ) -> Result<Self, String> {
        let mut topology = Self {
            schema: ASSIGNED_STAGE_TOPOLOGY_SCHEMA.into(),
            stages,
            edges,
            topology_sha256: String::new(),
        };
        topology.validate_body()?;
        topology.topology_sha256 = canonical_sha256(&topology)?;
        topology.validate()?;
        Ok(topology)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_body()?;
        validate_sha256(&self.topology_sha256, "assigned-stage topology")?;
        let mut unsigned = self.clone();
        unsigned.topology_sha256.clear();
        if self.topology_sha256 != canonical_sha256(&unsigned)? {
            return Err("assigned-stage topology digest mismatch".into());
        }
        Ok(())
    }

    pub fn ordered_stages(&self) -> &[AssignedStageIdentity] {
        &self.stages
    }

    pub fn ordered_edges(&self) -> &[AssignedStageEdge] {
        &self.edges
    }

    fn validate_body(&self) -> Result<(), String> {
        if self.schema != ASSIGNED_STAGE_TOPOLOGY_SCHEMA {
            return Err("assigned-stage topology schema mismatch".into());
        }
        if self.stages.is_empty() {
            return Err("assigned-stage topology has no stages".into());
        }
        let mut stages = BTreeSet::new();
        for stage in &self.stages {
            stage.validate()?;
            if !stages.insert(stage.clone()) {
                return Err(format!(
                    "assigned-stage topology repeats {}/{}",
                    stage.stage_name, stage.operation_name
                ));
            }
        }
        let mut edges = BTreeSet::new();
        for edge in &self.edges {
            edge.before.validate()?;
            edge.after.validate()?;
            if edge.before == edge.after {
                return Err("assigned-stage topology contains a self edge".into());
            }
            if !stages.contains(&edge.before) || !stages.contains(&edge.after) {
                return Err("assigned-stage topology edge names an undeclared stage".into());
            }
            if !edges.insert(edge.clone()) {
                return Err("assigned-stage topology repeats a dependency edge".into());
            }
        }
        deterministic_topological_order(&self.stages, &self.edges).map(|_| ())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignedStage {
    pub schema: String,
    pub topology_sha256: String,
    pub stage_name: String,
    pub operation_name: String,
    pub assigned_ordinal: u32,
    pub assignment_sha256: String,
}

impl AssignedStage {
    pub fn seal(
        topology_sha256: impl Into<String>,
        stage_name: impl Into<String>,
        operation_name: impl Into<String>,
        assigned_ordinal: u32,
    ) -> Result<Self, String> {
        let mut assignment = Self {
            schema: ASSIGNED_STAGE_SCHEMA.into(),
            topology_sha256: topology_sha256.into(),
            stage_name: stage_name.into(),
            operation_name: operation_name.into(),
            assigned_ordinal,
            assignment_sha256: String::new(),
        };
        assignment.validate_body()?;
        assignment.assignment_sha256 = canonical_sha256(&assignment)?;
        assignment.validate()?;
        Ok(assignment)
    }

    pub fn identity(&self) -> AssignedStageIdentity {
        AssignedStageIdentity::new(self.stage_name.clone(), self.operation_name.clone())
    }

    /// Construct the aggregate foundation identity for this exact assigned
    /// node. Packet adapters supply their authenticated program/resource/target
    /// digests, but cannot reconstruct or replace the provider-owned ordinal.
    pub fn foundation_operation_identity(
        &self,
        program_sha256: impl Into<String>,
        ordered_resources_sha256: impl Into<String>,
        target_sha256: impl Into<String>,
    ) -> Result<crate::FoundationOperationIdentity, String> {
        self.validate()?;
        let program_sha256 = program_sha256.into();
        let ordered_resources_sha256 = ordered_resources_sha256.into();
        let target_sha256 = target_sha256.into();
        validate_sha256(&program_sha256, "assigned-stage operation program")?;
        validate_sha256(
            &ordered_resources_sha256,
            "assigned-stage operation resource order",
        )?;
        validate_sha256(&target_sha256, "assigned-stage operation target")?;
        Ok(crate::FoundationOperationIdentity {
            operation_id: self.operation_name.clone(),
            global_order: self.assigned_ordinal,
            program_sha256,
            ordered_resources_sha256,
            target_sha256,
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_body()?;
        validate_sha256(&self.assignment_sha256, "assigned-stage assignment")?;
        let mut unsigned = self.clone();
        unsigned.assignment_sha256.clear();
        if self.assignment_sha256 != canonical_sha256(&unsigned)? {
            return Err("assigned-stage assignment digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<(), String> {
        if self.schema != ASSIGNED_STAGE_SCHEMA {
            return Err("assigned-stage assignment schema mismatch".into());
        }
        validate_sha256(&self.topology_sha256, "assigned-stage topology")?;
        validate_name(&self.stage_name, "assigned stage")?;
        validate_name(&self.operation_name, "assigned operation")?;
        if self.assigned_ordinal == 0 {
            return Err("assigned-stage ordinal is zero".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignedStageSchedule {
    pub schema: String,
    pub topology_sha256: String,
    pub assignments: Vec<AssignedStage>,
    pub schedule_sha256: String,
}

impl AssignedStageSchedule {
    pub fn from_topology(topology: &AssignedStageTopology) -> Result<Self, String> {
        topology.validate()?;
        let order = deterministic_topological_order(&topology.stages, &topology.edges)?;
        let assignments = order
            .into_iter()
            .enumerate()
            .map(|(index, identity)| {
                let assigned_ordinal = u32::try_from(index + 1)
                    .map_err(|_| "assigned-stage topology exceeds u32 ordinals".to_owned())?;
                AssignedStage::seal(
                    topology.topology_sha256.clone(),
                    identity.stage_name,
                    identity.operation_name,
                    assigned_ordinal,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::seal(topology, assignments)
    }

    /// Authenticate a transported schedule against the exact topology that
    /// owns its census and dependency edges.
    pub fn seal(
        topology: &AssignedStageTopology,
        assignments: Vec<AssignedStage>,
    ) -> Result<Self, String> {
        topology.validate()?;
        let mut schedule = Self {
            schema: ASSIGNED_STAGE_SCHEDULE_SCHEMA.into(),
            topology_sha256: topology.topology_sha256.clone(),
            assignments,
            schedule_sha256: String::new(),
        };
        schedule.validate_body()?;
        schedule.validate_assignments_against(topology)?;
        schedule.schedule_sha256 = canonical_sha256(&schedule)?;
        schedule.validate_against(topology)?;
        Ok(schedule)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_body()?;
        validate_sha256(&self.schedule_sha256, "assigned-stage schedule")?;
        let mut unsigned = self.clone();
        unsigned.schedule_sha256.clear();
        if self.schedule_sha256 != canonical_sha256(&unsigned)? {
            return Err("assigned-stage schedule digest mismatch".into());
        }
        Ok(())
    }

    pub fn validate_against(&self, topology: &AssignedStageTopology) -> Result<(), String> {
        self.validate()?;
        topology.validate()?;
        self.validate_assignments_against(topology)
    }

    pub fn require(
        &self,
        stage_name: &str,
        operation_name: &str,
    ) -> Result<&AssignedStage, String> {
        self.validate()?;
        self.assignments
            .iter()
            .find(|assignment| {
                assignment.stage_name == stage_name
                    && assignment.operation_name == operation_name
            })
            .ok_or_else(|| {
                format!(
                    "assigned-stage schedule omits required {stage_name}/{operation_name}"
                )
            })
    }

    pub fn ordered_assignments(&self) -> &[AssignedStage] {
        &self.assignments
    }

    fn validate_body(&self) -> Result<(), String> {
        if self.schema != ASSIGNED_STAGE_SCHEDULE_SCHEMA {
            return Err("assigned-stage schedule schema mismatch".into());
        }
        validate_sha256(&self.topology_sha256, "assigned-stage topology")?;
        if self.assignments.is_empty() {
            return Err("assigned-stage schedule has no assignments".into());
        }
        let mut identities = BTreeSet::new();
        let mut ordinals = BTreeSet::new();
        for (index, assignment) in self.assignments.iter().enumerate() {
            assignment.validate()?;
            if assignment.topology_sha256 != self.topology_sha256 {
                return Err("assigned-stage assignment belongs to another topology".into());
            }
            let expected_ordinal = u32::try_from(index + 1)
                .map_err(|_| "assigned-stage schedule exceeds u32 ordinals".to_owned())?;
            if assignment.assigned_ordinal != expected_ordinal {
                return Err(
                    "assigned-stage ordinals are not unique, contiguous, and nonzero".into(),
                );
            }
            if !identities.insert(assignment.identity())
                || !ordinals.insert(assignment.assigned_ordinal)
            {
                return Err("assigned-stage schedule repeats an identity or ordinal".into());
            }
        }
        Ok(())
    }

    fn validate_assignments_against(
        &self,
        topology: &AssignedStageTopology,
    ) -> Result<(), String> {
        if self.topology_sha256 != topology.topology_sha256 {
            return Err("assigned-stage schedule belongs to another topology".into());
        }
        let expected = deterministic_topological_order(&topology.stages, &topology.edges)?;
        if expected.len() != self.assignments.len()
            || expected.iter().zip(&self.assignments).any(|(identity, assignment)| {
                identity.stage_name != assignment.stage_name
                    || identity.operation_name != assignment.operation_name
            })
        {
            return Err(
                "assigned-stage schedule differs from deterministic topology order".into(),
            );
        }
        Ok(())
    }
}

fn deterministic_topological_order(
    stages: &[AssignedStageIdentity],
    edges: &[AssignedStageEdge],
) -> Result<Vec<AssignedStageIdentity>, String> {
    let stage_indexes = stages
        .iter()
        .enumerate()
        .map(|(index, stage)| (stage.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut indegrees = vec![0usize; stages.len()];
    let mut outgoing = vec![Vec::new(); stages.len()];
    for edge in edges {
        let before = *stage_indexes
            .get(&edge.before)
            .ok_or("assigned-stage edge has no declared source")?;
        let after = *stage_indexes
            .get(&edge.after)
            .ok_or("assigned-stage edge has no declared target")?;
        indegrees[after] = indegrees[after]
            .checked_add(1)
            .ok_or("assigned-stage indegree overflow")?;
        outgoing[before].push(after);
    }

    let mut emitted = vec![false; stages.len()];
    let mut ordered = Vec::with_capacity(stages.len());
    while ordered.len() < stages.len() {
        let next = (0..stages.len())
            .find(|index| !emitted[*index] && indegrees[*index] == 0)
            .ok_or("assigned-stage topology contains a dependency cycle")?;
        emitted[next] = true;
        ordered.push(stages[next].clone());
        for target in &outgoing[next] {
            indegrees[*target] = indegrees[*target]
                .checked_sub(1)
                .ok_or("assigned-stage topology indegree underflow")?;
        }
    }
    Ok(ordered)
}

fn canonical_sha256<T: Serialize>(value: &T) -> Result<String, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("assigned-stage canonical JSON: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase SHA-256"));
    }
    Ok(())
}

fn validate_name(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte))
    {
        return Err(format!(
            "{label} is not nonempty lowercase ASCII [a-z0-9_.-]"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(stage: &str, operation: &str) -> AssignedStageIdentity {
        AssignedStageIdentity::new(stage, operation)
    }

    fn topology() -> AssignedStageTopology {
        let early = identity("early_borders", "thin-borders.early.draw");
        let rivers = identity("rivers", "rivers.prepare-and-draw");
        let late = identity("late_borders", "thin-borders.late.draw");
        let straits = identity("straits", "straits.prepare-and-draw");
        AssignedStageTopology::seal(
            vec![early.clone(), rivers.clone(), late.clone(), straits.clone()],
            vec![
                AssignedStageEdge::new(early, late),
                AssignedStageEdge::new(rivers, straits),
            ],
        )
        .unwrap()
    }

    #[test]
    fn topology_derives_authenticated_contiguous_assignments() {
        let topology = topology();
        let schedule = AssignedStageSchedule::from_topology(&topology).unwrap();
        schedule.validate_against(&topology).unwrap();
        assert_eq!(
            schedule
                .ordered_assignments()
                .iter()
                .map(|assignment| (
                    assignment.stage_name.as_str(),
                    assignment.assigned_ordinal
                ))
                .collect::<Vec<_>>(),
            vec![
                ("early_borders", 1),
                ("rivers", 2),
                ("late_borders", 3),
                ("straits", 4)
            ]
        );
        assert_eq!(
            schedule
                .require("straits", "straits.prepare-and-draw")
                .unwrap()
                .assigned_ordinal,
            4
        );
    }

    #[test]
    fn assignments_fail_closed_on_tampering_or_wrong_topology() {
        let topology = topology();
        let schedule = AssignedStageSchedule::from_topology(&topology).unwrap();

        let mut zero = schedule.clone();
        zero.assignments[0].assigned_ordinal = 0;
        assert!(zero.validate().unwrap_err().contains("ordinal"));

        let mut reordered = schedule.clone();
        reordered.assignments.swap(0, 1);
        assert!(reordered.validate().is_err());

        let other = AssignedStageTopology::seal(
            vec![identity("water", "water.draw")],
            Vec::new(),
        )
        .unwrap();
        assert!(schedule.validate_against(&other).is_err());
    }

    #[test]
    fn foundation_identity_uses_the_authenticated_operation_and_ordinal() {
        let topology = topology();
        let schedule = AssignedStageSchedule::from_topology(&topology).unwrap();
        let assignment = schedule
            .require("rivers", "rivers.prepare-and-draw")
            .unwrap();
        let identity = assignment
            .foundation_operation_identity("a".repeat(64), "b".repeat(64), "c".repeat(64))
            .unwrap();
        assert_eq!(identity.operation_id, assignment.operation_name);
        assert_eq!(identity.global_order, assignment.assigned_ordinal);

        assert!(assignment
            .foundation_operation_identity("not-a-hash", "b".repeat(64), "c".repeat(64))
            .is_err());
    }

    #[test]
    fn invalid_names_edges_and_cycles_are_rejected() {
        assert!(AssignedStageTopology::seal(
            vec![identity("Bad Name", "draw")],
            Vec::new()
        )
        .is_err());

        let a = identity("a", "draw");
        let b = identity("b", "draw");
        assert!(AssignedStageTopology::seal(
            vec![a.clone(), b.clone()],
            vec![
                AssignedStageEdge::new(a.clone(), b.clone()),
                AssignedStageEdge::new(b, a),
            ],
        )
        .unwrap_err()
        .contains("cycle"));
    }
}
