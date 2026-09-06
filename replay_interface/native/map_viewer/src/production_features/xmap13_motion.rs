use super::derived::derived_frame_digest;
use crate::production_features::xmap13_model::DivisionModelFrame;
use crate::Result;
use replay_core::{Leaf, ViewStatus, WorldFrame};
use serde::Serialize;
use snapshot::sha256::Sha256;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub struct Xmap13MotionClock {
    pub seconds: f64,
    pub speed_step: usize,
    pub discontinuity: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum SplineState {
    Empty,
    DerivationUnavailable,
    SemanticUnavailable,
}

#[derive(Clone, Debug, Default)]
pub struct Xmap13MotionState {
    previous_hour: Option<i64>,
    previous_seconds: f64,
    previous_frame: String,
    content: String,
    history: String,
    failed_update: bool,
    helpers: BTreeMap<String, SplineState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap13MotionSample {
    frame: String,
    content: String,
    pub(crate) history_sha256: String,
    helpers: BTreeMap<String, SplineState>,
}

impl Xmap13MotionSample {
    pub(super) fn content_digest(&self) -> &str { &self.content }
    pub(crate) fn validate_frame(&self, world: &WorldFrame, content: &str) -> Result<()> {
        if self.frame != derived_frame_digest(world)? || self.content != content {
            return Err("XMAP13 helper history belongs to another frame or content admission".into());
        }
        Ok(())
    }

    pub(super) fn require_empty(&self, identity: &crate::Xmap13UnitIdentity) -> Result<()> {
        match self.helpers.get(&identity.canonical_id()) {
            Some(SplineState::Empty) => Ok(()),
            Some(SplineState::DerivationUnavailable) => Err("XMAP13 retained spline geometry is not derived; an empty semantic route cannot clear it".into()),
            Some(SplineState::SemanticUnavailable) => Err("XMAP13 helper semantic input is unavailable".into()),
            None => Err("XMAP13 helper has no current unambiguous division identity".into()),
        }
    }
}

impl Xmap13MotionState {
    pub fn sample(&mut self, world: &WorldFrame, content: &str, clock: Xmap13MotionClock) -> Result<Xmap13MotionSample> {
        let prior_failed = self.failed_update;
        self.failed_update = true;
        if prior_failed && !clock.discontinuity {
            return Err("XMAP13 failed helper update requires explicit viewer discontinuity".into());
        }
        if !clock.seconds.is_finite() || clock.seconds < 0.0 || clock.speed_step > 4 {
            return Err("XMAP13 helper clock or speed step is invalid".into());
        }
        if content.len() != 64 || !content.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("XMAP13 helper content digest is invalid".into());
        }
        let frame = derived_frame_digest(world)?;
        let hour = world.hour_index();
        let reset = clock.discontinuity || self.content != content || self.previous_hour.is_none()
            || self.previous_hour.is_some_and(|previous| hour < previous || hour > previous.saturating_add(1));
        if !reset && (clock.seconds < self.previous_seconds
            || (self.previous_hour == Some(hour) && self.previous_frame != frame)) {
            return Err("XMAP13 helper history changed without a viewer discontinuity".into());
        }
        let models = DivisionModelFrame::new(world, content)?;
        let mut helpers = BTreeMap::new();
        let mut retained = if reset { BTreeMap::new() } else { self.helpers.clone() };
        let mut census_complete = models.division_status() == ViewStatus::Present;
        if models.division_status() == ViewStatus::Present {
            for ordinal in 0..world.division_units().len() {
                let Ok(identity) = models.identity_at(ordinal) else { census_complete = false; continue };
                let key = identity.canonical_id();
                let previous = (!reset).then(|| self.helpers.get(&key)).flatten();
                let state = match models.inputs(&identity) {
                    Ok(input) => match input.movement_route {
                        Leaf::Measured(route) if route.is_empty() => match previous {
                            Some(SplineState::DerivationUnavailable | SplineState::SemanticUnavailable) => SplineState::DerivationUnavailable,
                            Some(SplineState::Empty) | None => SplineState::Empty,
                        },
                        Leaf::Measured(_) => SplineState::DerivationUnavailable,
                        _ => SplineState::SemanticUnavailable,
                    },
                    Err(_) => SplineState::SemanticUnavailable,
                };
                helpers.insert(key, state);
            }
        }
        if census_complete {
            retained = helpers.clone();
        } else {
            for state in retained.values_mut() { *state = SplineState::DerivationUnavailable; }
            retained.extend(helpers.iter().map(|(identity, state)| (identity.clone(), *state)));
        }
        let history_sha256 = Sha256::hex(&serde_json::to_vec(&(
            "XMAP13_HELPER_HISTORY_V1", &self.history, &frame, content, reset,
            clock.seconds.to_bits(), clock.speed_step, &helpers, &retained,
        ))?);
        *self = Self {
            previous_hour: Some(hour), previous_seconds: clock.seconds,
            previous_frame: frame.clone(), content: content.into(),
            history: history_sha256.clone(), failed_update: false, helpers: retained,
        };
        Ok(Xmap13MotionSample { frame, content: content.into(), history_sha256, helpers })
    }
}
