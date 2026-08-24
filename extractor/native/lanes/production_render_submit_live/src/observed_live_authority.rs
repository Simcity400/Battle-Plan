use crate::{
    ExactLiveRenderCatalogSemantics, ObservedStaticDrawAuthority, ProductionStaticCaptureAuthority,
    RawD3dDrawState, RawParticleSubmit, RawRenderSubmitCapture,
};
use production_render_catalog_capture_contract::{
    materialize_d77_draw_authority, materialize_d80_static_authority, RawDrawContentIdentity,
    RawDrawIdentity, RawDrawSemantics, RawEffectiveVfsOpenObservation,
    RawEffectiveVfsWinnerIndexBinding, RawLiveDrawAuthorityObservation, RawProgramIdentity,
    RawRenderTargetRegistrationReceipt, RawStaticAuthorityMaterialization,
    RawStaticAuthorityObservation, RawStaticD3dDrawState,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionAuthorityRegistryLimits {
    pub max_pending_groups: usize,
    pub max_rows_per_group: usize,
    pub max_completed_groups: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnselectedAuthorityGroup {
    pub submit_index: u64,
    pub ordered_live_observation_sequences: Vec<u64>,
    pub frame_sequence: u64,
    pub ordered_static_capture_sequences: Vec<u64>,
}

impl ProductionAuthorityRegistryLimits {
    fn validate(self) -> Result<Self, String> {
        if self.max_pending_groups == 0
            || self.max_rows_per_group == 0
            || self.max_completed_groups == 0
        {
            return Err("production authority registry limits must be nonzero".into());
        }
        Ok(self)
    }
}

#[derive(Clone, Debug)]
struct MaterializedLiveDrawAuthority {
    observation_sequence: u64,
    identity: RawDrawIdentity,
    shader_program_ptr: usize,
    source_open: RawEffectiveVfsOpenObservation,
    program_identity: RawProgramIdentity,
    draw_semantics: RawDrawSemantics,
}

#[derive(Clone, Debug)]
pub struct ProductionExactLiveRenderCatalogSemantics {
    submit_index: u64,
    ordered_rows: Vec<MaterializedLiveDrawAuthority>,
}

impl ProductionExactLiveRenderCatalogSemantics {
    pub fn submit_index(&self) -> u64 {
        self.submit_index
    }

    pub fn draw_count(&self) -> usize {
        self.ordered_rows.len()
    }

    fn exact_row(
        &self,
        submit: &RawParticleSubmit,
        draw: &RawD3dDrawState,
    ) -> Result<&MaterializedLiveDrawAuthority, String> {
        if submit.submit_index != self.submit_index || draw.submit_index != self.submit_index {
            return Err("live authority adapter was requested for another submit".into());
        }
        let mut matches = self.ordered_rows.iter().filter(|row| {
            row.observation_sequence == draw.final_draw_sequence
                && row.identity == draw.identity
                && row.shader_program_ptr == draw.shader_program_ptr
        });
        let row = matches
            .next()
            .ok_or_else(|| "live authority adapter has no exact draw row".to_owned())?;
        if matches.next().is_some() {
            return Err("live authority adapter contains ambiguous draw rows".into());
        }
        Ok(row)
    }
}

impl ExactLiveRenderCatalogSemantics for ProductionExactLiveRenderCatalogSemantics {
    fn resolve_program_identity(
        &self,
        submit: &RawParticleSubmit,
        draw: &RawD3dDrawState,
    ) -> Result<RawProgramIdentity, String> {
        Ok(self.exact_row(submit, draw)?.program_identity.clone())
    }

    fn resolve_draw_semantics(
        &self,
        submit: &RawParticleSubmit,
        draw: &RawD3dDrawState,
    ) -> Result<RawDrawSemantics, String> {
        Ok(self.exact_row(submit, draw)?.draw_semantics.clone())
    }
}

#[derive(Debug, Default)]
struct LiveRegistryState {
    winner_index: Option<RawEffectiveVfsWinnerIndexBinding>,
    pending: BTreeMap<u64, Vec<MaterializedLiveDrawAuthority>>,
    completed: VecDeque<ProductionExactLiveRenderCatalogSemantics>,
    last_finalized_submit_index: Option<u64>,
}

#[derive(Debug)]
pub struct ProductionExactLiveRenderCatalogSemanticsRegistry {
    limits: ProductionAuthorityRegistryLimits,
    state: Mutex<LiveRegistryState>,
}

impl ProductionExactLiveRenderCatalogSemanticsRegistry {
    pub fn new(limits: ProductionAuthorityRegistryLimits) -> Result<Self, String> {
        Ok(Self {
            limits: limits.validate()?,
            state: Mutex::new(LiveRegistryState::default()),
        })
    }

    pub fn bind_effective_vfs_winners(
        &self,
        binding: RawEffectiveVfsWinnerIndexBinding,
    ) -> Result<(), String> {
        binding.validate()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "live authority registry lock poisoned".to_owned())?;
        if state.winner_index.is_some() || !state.pending.is_empty() || !state.completed.is_empty()
        {
            return Err(
                "live authority winner index is already bound or observations exist".into(),
            );
        }
        state.winner_index = Some(binding);
        Ok(())
    }

    pub fn observe_live_draw_authority(
        &self,
        submit_index: u64,
        observed: RawLiveDrawAuthorityObservation,
    ) -> Result<(), String> {
        let (program_identity, draw_semantics) = materialize_d77_draw_authority(&observed)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "live authority registry lock poisoned".to_owned())?;
        state.winner_index.as_ref().ok_or_else(|| {
            "live authority registry has no authenticated winner index".to_owned()
        })?;
        let row = MaterializedLiveDrawAuthority {
            observation_sequence: observed.observation_sequence,
            identity: observed.identity,
            shader_program_ptr: program_identity.shader_program_ptr,
            source_open: observed.source_open,
            program_identity,
            draw_semantics,
        };
        if state
            .last_finalized_submit_index
            .is_some_and(|last| submit_index <= last)
        {
            return Err("live authority observation belongs to a stale or reordered submit".into());
        }
        if !state.pending.contains_key(&submit_index)
            && state.pending.len() >= self.limits.max_pending_groups
        {
            return Err("live authority pending-submit bound reached".into());
        }
        let rows = state.pending.entry(submit_index).or_default();
        if rows.len() >= self.limits.max_rows_per_group {
            return Err("live authority per-submit row bound reached".into());
        }
        if rows.iter().any(|existing| {
            existing.observation_sequence == row.observation_sequence
                || existing.identity == row.identity
        }) {
            return Err("live authority observation is ambiguous or repeated".into());
        }
        rows.push(row);
        Ok(())
    }

    pub fn finalize_live_submit_authority(
        &self,
        capture: &RawRenderSubmitCapture,
    ) -> Result<(), String> {
        self.finalize_selected_live_submit_authority(capture, &[])
    }

    pub fn discard_unselected_live_authority_ids(
        &self,
        submit_index: u64,
        unselected_observation_ids: &[u64],
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "live authority registry lock poisoned".to_owned())?;
        let rows = state
            .pending
            .get(&submit_index)
            .ok_or_else(|| "live authority discard names an absent submit".to_owned())?;
        let discard = exact_discard_ids(
            rows.iter().map(|row| row.observation_sequence),
            unselected_observation_ids,
            "live authority",
        )?;
        let retained = rows
            .iter()
            .filter(|row| !discard.contains(&row.observation_sequence))
            .cloned()
            .collect::<Vec<_>>();
        if retained.is_empty() {
            state.pending.remove(&submit_index);
        } else {
            state.pending.insert(submit_index, retained);
        }
        Ok(())
    }

    pub fn finalize_selected_live_submit_authority(
        &self,
        capture: &RawRenderSubmitCapture,
        unselected_observation_ids: &[u64],
    ) -> Result<(), String> {
        if capture.submit.submit_index != capture.catalog.submit_index
            || !capture.catalog.program_identities.is_empty()
            || !capture.catalog.draw_semantics.is_empty()
        {
            return Err(
                "live authority finalize received a mismatched or materialized capture".into(),
            );
        }
        let expected = capture
            .submit
            .packets
            .len()
            .checked_add(capture.submit.entity_submissions.len())
            .ok_or("live authority expected draw count overflow")?;
        if capture.catalog.draw_states.len() != expected {
            return Err("live authority draw-state cardinality differs from the submit".into());
        }

        let submit_index = capture.submit.submit_index;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "live authority registry lock poisoned".to_owned())?;
        let winner_index = state.winner_index.as_ref().ok_or_else(|| {
            "live authority registry has no authenticated winner index".to_owned()
        })?;
        if capture.catalog.process.process_start_nonce != winner_index.process_start_nonce {
            return Err("live authority capture uses another process nonce".into());
        }
        if state.completed.len() >= self.limits.max_completed_groups {
            return Err("live authority completed-submit bound reached".into());
        }
        if state
            .last_finalized_submit_index
            .is_some_and(|last| submit_index <= last)
        {
            return Err("live authority submit finalization is stale or reordered".into());
        }
        let pending_rows = state
            .pending
            .get(&submit_index)
            .ok_or_else(|| "live authority submit has no observed rows".to_owned())?;
        let discard = exact_discard_ids(
            pending_rows.iter().map(|row| row.observation_sequence),
            unselected_observation_ids,
            "live authority",
        )?;
        let rows = pending_rows
            .iter()
            .filter(|row| !discard.contains(&row.observation_sequence))
            .cloned()
            .collect::<Vec<_>>();
        if rows.len() != expected {
            return Err("live authority submit contains unmatched rows".into());
        }
        let mut prior_sequence = 0u64;
        let mut identities = BTreeSet::new();
        for (row, draw) in rows.iter().zip(&capture.catalog.draw_states) {
            if draw.final_draw_sequence == 0
                || draw.final_draw_sequence <= prior_sequence
                || draw.submit_index != submit_index
                || !identities.insert(draw.identity.clone())
                || row.observation_sequence != draw.final_draw_sequence
                || row.identity != draw.identity
                || row.shader_program_ptr != draw.shader_program_ptr
            {
                return Err("live authority rows are unmatched, ambiguous, or reordered".into());
            }
            winner_index.authorize_open(&row.source_open)?;
            winner_index.authorize_content(&row.program_identity.source)?;
            authorize_draw_contents(winner_index, &row.draw_semantics)?;
            prior_sequence = draw.final_draw_sequence;
        }
        state.pending.remove(&submit_index);
        state
            .completed
            .push_back(ProductionExactLiveRenderCatalogSemantics {
                submit_index,
                ordered_rows: rows,
            });
        state.last_finalized_submit_index = Some(submit_index);
        Ok(())
    }

    pub fn drain_complete_live_render_catalog_semantics(
        &self,
    ) -> Result<Vec<ProductionExactLiveRenderCatalogSemantics>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "live authority registry lock poisoned".to_owned())?;
        Ok(state.completed.drain(..).collect())
    }

    pub fn reset(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "live authority registry lock poisoned".to_owned())?;
        *state = LiveRegistryState::default();
        Ok(())
    }
}

fn authorize_draw_contents(
    winner_index: &RawEffectiveVfsWinnerIndexBinding,
    semantics: &RawDrawSemantics,
) -> Result<(), String> {
    match &semantics.content_identity {
        RawDrawContentIdentity::Particle {
            definition_content,
            texture_content,
            ..
        } => {
            winner_index.authorize_content(definition_content)?;
            winner_index.authorize_content(texture_content)
        }
        RawDrawContentIdentity::Entity {
            resolved_entity_asset,
            mesh,
            ..
        } => {
            winner_index.authorize_content(resolved_entity_asset)?;
            winner_index.authorize_content(mesh)
        }
    }
}

fn exact_discard_ids(
    available_ids: impl IntoIterator<Item = u64>,
    requested_ids: &[u64],
    label: &str,
) -> Result<BTreeSet<u64>, String> {
    let available = available_ids.into_iter().collect::<BTreeSet<_>>();
    let mut requested = BTreeSet::new();
    if requested_ids
        .iter()
        .any(|id| *id == 0 || !requested.insert(*id))
    {
        return Err(format!("{label} discard IDs are zero or repeated"));
    }
    if requested.iter().any(|id| !available.contains(id)) {
        return Err(format!(
            "{label} discard names a foreign, future, or absent observation ID"
        ));
    }
    Ok(requested)
}

#[derive(Clone, Debug)]
pub struct FinalizedProductionStaticAuthority {
    frame_sequence: u64,
    ordered_rows: Vec<RawStaticAuthorityMaterialization>,
}

impl FinalizedProductionStaticAuthority {
    pub fn frame_sequence(&self) -> u64 {
        self.frame_sequence
    }

    pub fn draw_count(&self) -> usize {
        self.ordered_rows.len()
    }

    pub fn apply_to(self, authority: &mut ProductionStaticCaptureAuthority) -> Result<(), String> {
        let mut candidate = authority.clone();
        for row in self.ordered_rows {
            candidate.observe_materialized_static_authority(row)?;
        }
        *authority = candidate;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct StaticRegistryState {
    winner_index: Option<RawEffectiveVfsWinnerIndexBinding>,
    pending: BTreeMap<u64, Vec<PendingStaticAuthorityRow>>,
    completed: VecDeque<FinalizedProductionStaticAuthority>,
    last_finalized_frame_sequence: Option<u64>,
}

#[derive(Clone, Debug)]
struct PendingStaticAuthorityRow {
    source_open: RawEffectiveVfsOpenObservation,
    materialized: RawStaticAuthorityMaterialization,
}

#[derive(Debug)]
pub struct ProductionStaticAuthorityRegistry {
    limits: ProductionAuthorityRegistryLimits,
    state: Mutex<StaticRegistryState>,
}

impl ProductionStaticAuthorityRegistry {
    pub fn new(limits: ProductionAuthorityRegistryLimits) -> Result<Self, String> {
        Ok(Self {
            limits: limits.validate()?,
            state: Mutex::new(StaticRegistryState::default()),
        })
    }

    pub fn bind_effective_vfs_winners(
        &self,
        binding: RawEffectiveVfsWinnerIndexBinding,
    ) -> Result<(), String> {
        binding.validate()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "static authority registry lock poisoned".to_owned())?;
        if state.winner_index.is_some() || !state.pending.is_empty() || !state.completed.is_empty()
        {
            return Err(
                "static authority winner index is already bound or observations exist".into(),
            );
        }
        state.winner_index = Some(binding);
        Ok(())
    }

    pub fn observe_static_authority(
        &self,
        frame_sequence: u64,
        observed: RawStaticAuthorityObservation,
    ) -> Result<(), String> {
        if frame_sequence == 0 {
            return Err("static authority frame sequence is zero".into());
        }
        let row = materialize_d80_static_authority(&observed)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "static authority registry lock poisoned".to_owned())?;
        state.winner_index.as_ref().ok_or_else(|| {
            "static authority registry has no authenticated winner index".to_owned()
        })?;
        if state
            .last_finalized_frame_sequence
            .is_some_and(|last| frame_sequence <= last)
        {
            return Err(
                "static authority observation belongs to a stale or reordered frame".into(),
            );
        }
        if !state.pending.contains_key(&frame_sequence)
            && state.pending.len() >= self.limits.max_pending_groups
        {
            return Err("static authority pending-frame bound reached".into());
        }
        let rows = state.pending.entry(frame_sequence).or_default();
        if rows.len() >= self.limits.max_rows_per_group {
            return Err("static authority per-frame row bound reached".into());
        }
        if rows.iter().any(|existing| {
            existing.materialized.capture_sequence == row.capture_sequence
                || existing.materialized.program_identity == row.program_identity
        }) {
            return Err("static authority observation is ambiguous or repeated".into());
        }
        rows.push(PendingStaticAuthorityRow {
            source_open: observed.source_open,
            materialized: row,
        });
        Ok(())
    }

    pub fn finalize_static_authority(
        &self,
        frame_sequence: u64,
        process_start_nonce: u64,
        ordered_draws: &[RawStaticD3dDrawState],
    ) -> Result<(), String> {
        self.finalize_selected_static_authority(
            frame_sequence,
            process_start_nonce,
            ordered_draws,
            &[],
        )
    }

    pub fn discard_unselected_static_authority_ids(
        &self,
        frame_sequence: u64,
        unselected_capture_ids: &[u64],
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "static authority registry lock poisoned".to_owned())?;
        let rows = state
            .pending
            .get(&frame_sequence)
            .ok_or_else(|| "static authority discard names an absent frame".to_owned())?;
        let discard = exact_discard_ids(
            rows.iter().map(|row| row.materialized.capture_sequence),
            unselected_capture_ids,
            "static authority",
        )?;
        let retained = rows
            .iter()
            .filter(|row| !discard.contains(&row.materialized.capture_sequence))
            .cloned()
            .collect::<Vec<_>>();
        if retained.is_empty() {
            state.pending.remove(&frame_sequence);
        } else {
            state.pending.insert(frame_sequence, retained);
        }
        Ok(())
    }

    pub fn finalize_selected_static_authority(
        &self,
        frame_sequence: u64,
        process_start_nonce: u64,
        ordered_draws: &[RawStaticD3dDrawState],
        unselected_capture_ids: &[u64],
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "static authority registry lock poisoned".to_owned())?;
        let winner_index = state.winner_index.as_ref().ok_or_else(|| {
            "static authority registry has no authenticated winner index".to_owned()
        })?;
        if process_start_nonce != winner_index.process_start_nonce {
            return Err("static authority frame uses another process nonce".into());
        }
        if state.completed.len() >= self.limits.max_completed_groups {
            return Err("static authority completed-frame bound reached".into());
        }
        if state
            .last_finalized_frame_sequence
            .is_some_and(|last| frame_sequence <= last)
        {
            return Err("static authority frame finalization is stale or reordered".into());
        }
        let pending_rows = state
            .pending
            .get(&frame_sequence)
            .ok_or_else(|| "static authority frame has no observed rows".to_owned())?;
        let discard = exact_discard_ids(
            pending_rows
                .iter()
                .map(|row| row.materialized.capture_sequence),
            unselected_capture_ids,
            "static authority",
        )?;
        let rows = pending_rows
            .iter()
            .filter(|row| !discard.contains(&row.materialized.capture_sequence))
            .cloned()
            .collect::<Vec<_>>();
        if rows.len() != ordered_draws.len() || rows.is_empty() {
            return Err("static authority frame contains unmatched rows".into());
        }
        let mut prior_sequence = 0u64;
        let mut seen = BTreeSet::new();
        for (pending, draw) in rows.iter().zip(ordered_draws) {
            let row = &pending.materialized;
            if draw.capture_sequence == 0
                || draw.capture_sequence <= prior_sequence
                || !seen.insert(draw.capture_sequence)
                || row.capture_sequence != draw.capture_sequence
                || row.shader_program_ptr != draw.shader_program_ptr
            {
                return Err("static authority rows are unmatched, ambiguous, or reordered".into());
            }
            winner_index.authorize_open(&pending.source_open)?;
            winner_index.authorize_content(&row.program_identity.source)?;
            for content in &row.ordered_content {
                winner_index.authorize_content(&content.content)?;
            }
            prior_sequence = draw.capture_sequence;
        }
        let ordered_rows = rows
            .into_iter()
            .map(|row| row.materialized)
            .collect::<Vec<_>>();
        state.pending.remove(&frame_sequence);
        state
            .completed
            .push_back(FinalizedProductionStaticAuthority {
                frame_sequence,
                ordered_rows,
            });
        state.last_finalized_frame_sequence = Some(frame_sequence);
        Ok(())
    }

    pub fn drain_complete_static_authorities(
        &self,
    ) -> Result<Vec<FinalizedProductionStaticAuthority>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "static authority registry lock poisoned".to_owned())?;
        Ok(state.completed.drain(..).collect())
    }

    pub fn reset(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "static authority registry lock poisoned".to_owned())?;
        *state = StaticRegistryState::default();
        Ok(())
    }
}

static LIVE_AUTHORITY_REGISTRY: OnceLock<
    Mutex<Option<ProductionExactLiveRenderCatalogSemanticsRegistry>>,
> = OnceLock::new();
static STATIC_AUTHORITY_REGISTRY: OnceLock<Mutex<Option<ProductionStaticAuthorityRegistry>>> =
    OnceLock::new();

fn global_live_registry(
) -> &'static Mutex<Option<ProductionExactLiveRenderCatalogSemanticsRegistry>> {
    LIVE_AUTHORITY_REGISTRY.get_or_init(|| Mutex::new(None))
}

fn global_static_registry() -> &'static Mutex<Option<ProductionStaticAuthorityRegistry>> {
    STATIC_AUTHORITY_REGISTRY.get_or_init(|| Mutex::new(None))
}

pub fn install_live_authority_registry(
    limits: ProductionAuthorityRegistryLimits,
) -> Result<(), String> {
    let mut slot = global_live_registry()
        .lock()
        .map_err(|_| "global live authority registry lock poisoned".to_owned())?;
    if slot.is_some() {
        return Err("global live authority registry is already installed".into());
    }
    *slot = Some(ProductionExactLiveRenderCatalogSemanticsRegistry::new(
        limits,
    )?);
    Ok(())
}

pub fn uninstall_live_authority_registry() -> Result<(), String> {
    let mut slot = global_live_registry()
        .lock()
        .map_err(|_| "global live authority registry lock poisoned".to_owned())?;
    if slot.take().is_none() {
        return Err("global live authority registry is not installed".into());
    }
    Ok(())
}

pub fn reset_live_authority_registry() -> Result<(), String> {
    let slot = global_live_registry()
        .lock()
        .map_err(|_| "global live authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global live authority registry is not installed".to_owned())?
        .reset()
}

pub fn observe_live_draw_authority(
    submit_index: u64,
    observed: RawLiveDrawAuthorityObservation,
) -> Result<(), String> {
    let slot = global_live_registry()
        .lock()
        .map_err(|_| "global live authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global live authority registry is not installed".to_owned())?
        .observe_live_draw_authority(submit_index, observed)
}

pub fn bind_live_effective_vfs_winners(
    binding: RawEffectiveVfsWinnerIndexBinding,
) -> Result<(), String> {
    let slot = global_live_registry()
        .lock()
        .map_err(|_| "global live authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global live authority registry is not installed".to_owned())?
        .bind_effective_vfs_winners(binding)
}

pub fn bind_effective_vfs_winners(
    binding: RawEffectiveVfsWinnerIndexBinding,
) -> Result<(), String> {
    binding.validate()?;
    let live_slot = global_live_registry()
        .lock()
        .map_err(|_| "global live authority registry lock poisoned".to_owned())?;
    let static_slot = global_static_registry()
        .lock()
        .map_err(|_| "global static authority registry lock poisoned".to_owned())?;
    let live = live_slot
        .as_ref()
        .ok_or_else(|| "global live authority registry is not installed".to_owned())?;
    let static_registry = static_slot
        .as_ref()
        .ok_or_else(|| "global static authority registry is not installed".to_owned())?;
    live.bind_effective_vfs_winners(binding.clone())?;
    if let Err(error) = static_registry.bind_effective_vfs_winners(binding) {
        live.reset()?;
        return Err(error);
    }
    Ok(())
}

pub fn finalize_live_submit_authority(capture: &RawRenderSubmitCapture) -> Result<(), String> {
    let slot = global_live_registry()
        .lock()
        .map_err(|_| "global live authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global live authority registry is not installed".to_owned())?
        .finalize_live_submit_authority(capture)
}

pub fn discard_unselected_live_authority_ids(
    submit_index: u64,
    unselected_observation_ids: &[u64],
) -> Result<(), String> {
    let slot = global_live_registry()
        .lock()
        .map_err(|_| "global live authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global live authority registry is not installed".to_owned())?
        .discard_unselected_live_authority_ids(submit_index, unselected_observation_ids)
}

pub fn finalize_selected_live_submit_authority(
    capture: &RawRenderSubmitCapture,
    unselected_observation_ids: &[u64],
) -> Result<(), String> {
    let slot = global_live_registry()
        .lock()
        .map_err(|_| "global live authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global live authority registry is not installed".to_owned())?
        .finalize_selected_live_submit_authority(capture, unselected_observation_ids)
}

pub fn drain_complete_live_render_catalog_semantics(
) -> Result<Vec<ProductionExactLiveRenderCatalogSemantics>, String> {
    let slot = global_live_registry()
        .lock()
        .map_err(|_| "global live authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global live authority registry is not installed".to_owned())?
        .drain_complete_live_render_catalog_semantics()
}

pub fn install_static_authority_registry(
    limits: ProductionAuthorityRegistryLimits,
) -> Result<(), String> {
    let mut slot = global_static_registry()
        .lock()
        .map_err(|_| "global static authority registry lock poisoned".to_owned())?;
    if slot.is_some() {
        return Err("global static authority registry is already installed".into());
    }
    *slot = Some(ProductionStaticAuthorityRegistry::new(limits)?);
    Ok(())
}

pub fn uninstall_static_authority_registry() -> Result<(), String> {
    let mut slot = global_static_registry()
        .lock()
        .map_err(|_| "global static authority registry lock poisoned".to_owned())?;
    if slot.take().is_none() {
        return Err("global static authority registry is not installed".into());
    }
    Ok(())
}

pub fn reset_static_authority_registry() -> Result<(), String> {
    let slot = global_static_registry()
        .lock()
        .map_err(|_| "global static authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global static authority registry is not installed".to_owned())?
        .reset()
}

pub fn observe_static_authority(
    frame_sequence: u64,
    observed: RawStaticAuthorityObservation,
) -> Result<(), String> {
    let slot = global_static_registry()
        .lock()
        .map_err(|_| "global static authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global static authority registry is not installed".to_owned())?
        .observe_static_authority(frame_sequence, observed)
}

pub fn bind_static_effective_vfs_winners(
    binding: RawEffectiveVfsWinnerIndexBinding,
) -> Result<(), String> {
    let slot = global_static_registry()
        .lock()
        .map_err(|_| "global static authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global static authority registry is not installed".to_owned())?
        .bind_effective_vfs_winners(binding)
}

pub fn finalize_static_authority(
    frame_sequence: u64,
    process_start_nonce: u64,
    ordered_draws: &[RawStaticD3dDrawState],
) -> Result<(), String> {
    let slot = global_static_registry()
        .lock()
        .map_err(|_| "global static authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global static authority registry is not installed".to_owned())?
        .finalize_static_authority(frame_sequence, process_start_nonce, ordered_draws)
}

pub fn discard_unselected_static_authority_ids(
    frame_sequence: u64,
    unselected_capture_ids: &[u64],
) -> Result<(), String> {
    let slot = global_static_registry()
        .lock()
        .map_err(|_| "global static authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global static authority registry is not installed".to_owned())?
        .discard_unselected_static_authority_ids(frame_sequence, unselected_capture_ids)
}

pub fn finalize_selected_static_authority(
    frame_sequence: u64,
    process_start_nonce: u64,
    ordered_draws: &[RawStaticD3dDrawState],
    unselected_capture_ids: &[u64],
) -> Result<(), String> {
    let slot = global_static_registry()
        .lock()
        .map_err(|_| "global static authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global static authority registry is not installed".to_owned())?
        .finalize_selected_static_authority(
            frame_sequence,
            process_start_nonce,
            ordered_draws,
            unselected_capture_ids,
        )
}

pub fn drain_complete_static_authorities() -> Result<Vec<FinalizedProductionStaticAuthority>, String>
{
    let slot = global_static_registry()
        .lock()
        .map_err(|_| "global static authority registry lock poisoned".to_owned())?;
    slot.as_ref()
        .ok_or_else(|| "global static authority registry is not installed".to_owned())?
        .drain_complete_static_authorities()
}

pub fn discard_unselected_authority_groups(
    groups: &[UnselectedAuthorityGroup],
) -> Result<(), String> {
    let mut submit_ids = BTreeSet::new();
    let mut frame_ids = BTreeSet::new();
    for group in groups {
        if group.submit_index == 0
            || group.frame_sequence == 0
            || !submit_ids.insert(group.submit_index)
            || !frame_ids.insert(group.frame_sequence)
        {
            return Err("unselected authority discard contains null or duplicate ids".into());
        }
    }
    let live_slot = global_live_registry()
        .lock()
        .map_err(|_| "global live authority registry lock poisoned".to_owned())?;
    let static_slot = global_static_registry()
        .lock()
        .map_err(|_| "global static authority registry lock poisoned".to_owned())?;
    let all_groups_empty = groups.iter().all(|group| {
        group.ordered_live_observation_sequences.is_empty()
            && group.ordered_static_capture_sequences.is_empty()
    });
    let (live, static_registry) = match (live_slot.as_ref(), static_slot.as_ref()) {
        (None, None) if all_groups_empty => return Ok(()),
        (None, None) => {
            return Err("global authority registries are not installed".into());
        }
        (Some(live), Some(static_registry)) => (live, static_registry),
        _ => {
            return Err("global authority registries are only partially installed".into());
        }
    };
    let mut live_state = live
        .state
        .lock()
        .map_err(|_| "live authority registry lock poisoned".to_owned())?;
    let mut static_state = static_registry
        .state
        .lock()
        .map_err(|_| "static authority registry lock poisoned".to_owned())?;

    validate_unselected_authority_groups(groups, &live_state, &static_state)?;
    for group in groups {
        if !group.ordered_live_observation_sequences.is_empty() {
            live_state.pending.remove(&group.submit_index);
        }
        if !group.ordered_static_capture_sequences.is_empty() {
            static_state.pending.remove(&group.frame_sequence);
        }
    }
    Ok(())
}

fn validate_unselected_authority_groups(
    groups: &[UnselectedAuthorityGroup],
    live_state: &LiveRegistryState,
    static_state: &StaticRegistryState,
) -> Result<(), String> {
    for group in groups {
        if live_state
            .completed
            .iter()
            .any(|row| row.submit_index == group.submit_index)
        {
            return Err("unselected live authority group was already finalized".into());
        }
        match (
            group.ordered_live_observation_sequences.as_slice(),
            live_state.pending.get(&group.submit_index),
        ) {
            ([], None) => {}
            (expected, Some(rows))
                if rows
                    .iter()
                    .map(|row| row.observation_sequence)
                    .eq(expected.iter().copied()) => {}
            _ => return Err("unselected live authority group exact sequences differ".into()),
        }
        if static_state
            .completed
            .iter()
            .any(|row| row.frame_sequence == group.frame_sequence)
        {
            return Err("unselected static authority group was already finalized".into());
        }
        match (
            group.ordered_static_capture_sequences.as_slice(),
            static_state.pending.get(&group.frame_sequence),
        ) {
            ([], None) => {}
            (expected, Some(rows))
                if rows
                    .iter()
                    .map(|row| row.materialized.capture_sequence)
                    .eq(expected.iter().copied()) => {}
            _ => return Err("unselected static authority group exact sequences differ".into()),
        }
    }
    Ok(())
}

pub fn observed_static_draw_authority(
    raw: RawStaticAuthorityMaterialization,
) -> Result<ObservedStaticDrawAuthority, String> {
    if raw.capture_sequence == 0
        || raw.shader_program_ptr == 0
        || raw.program_identity.observation_sequence != raw.capture_sequence
        || raw.program_identity.shader_program_ptr != raw.shader_program_ptr
        || raw.ordered_color_targets.is_empty()
        || raw.ordered_content.is_empty()
    {
        return Err("neutral d80 authority materialization is incomplete".into());
    }
    Ok(ObservedStaticDrawAuthority {
        capture_sequence: raw.capture_sequence,
        shader_program_ptr: raw.shader_program_ptr,
        program_identity: raw.program_identity,
        ordered_texture_semantics: raw.ordered_texture_semantics,
        ordered_sampler_semantics: raw.ordered_sampler_semantics,
        ordered_constant_buffer_semantics: raw.ordered_constant_buffer_semantics,
        ordered_color_targets: raw.ordered_color_targets,
        depth_target: raw.depth_target,
        ordered_content: raw.ordered_content,
    })
}

pub trait ProductionStaticCaptureAuthorityLiveExt {
    fn observe_materialized_static_authority(
        &mut self,
        raw: RawStaticAuthorityMaterialization,
    ) -> Result<(), String>;

    fn observe_render_target_receipt(
        &mut self,
        receipt: RawRenderTargetRegistrationReceipt,
    ) -> Result<(), String>;
}

impl ProductionStaticCaptureAuthorityLiveExt for ProductionStaticCaptureAuthority {
    fn observe_materialized_static_authority(
        &mut self,
        raw: RawStaticAuthorityMaterialization,
    ) -> Result<(), String> {
        self.observe_draw(observed_static_draw_authority(raw)?)
    }

    fn observe_render_target_receipt(
        &mut self,
        receipt: RawRenderTargetRegistrationReceipt,
    ) -> Result<(), String> {
        if receipt.capture_sequence == 0
            || receipt.registration_sequence == 0
            || receipt.view_ptr == 0
            || receipt.resource_ptr == 0
            || receipt.target.view_ptr != receipt.view_ptr
        {
            return Err("render-target receipt is incomplete or mismatched".into());
        }
        self.observe_output_target(receipt.capture_sequence, receipt.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use production_render_catalog_capture_contract::*;

    fn content() -> RawVfsContentIdentity {
        RawVfsContentIdentity {
            logical_path: "gfx/fx/mod.shader".into(),
            winning_layer_ordinal: 9,
            layer_kind: "mod".into(),
            layer_id: "mod/graphics.mod".into(),
            source_sha256: "a".repeat(64),
            payload_sha256: "b".repeat(64),
            payload_bytes: 16,
        }
    }

    fn target() -> RawOutputTargetSemantic {
        RawOutputTargetSemantic {
            view_ptr: 0x500,
            prepared_target: RawPreparedTarget::OpaqueRenderTarget {
                attachment_ordinal: 0,
            },
            resource_token: Some(format!("sha256:{}", "1".repeat(64))),
            producer_pass_token: Some(format!("sha256:{}", "2".repeat(64))),
            persistent_content: None,
        }
    }

    fn limits() -> ProductionAuthorityRegistryLimits {
        ProductionAuthorityRegistryLimits {
            max_pending_groups: 2,
            max_rows_per_group: 4,
            max_completed_groups: 2,
        }
    }

    fn winner_binding() -> RawEffectiveVfsWinnerIndexBinding {
        let row = RawEffectiveVfsWinnerRow {
            content: content(),
            source_container_sha256: Some("a".repeat(64)),
            source_byte_offset: 32,
            source_byte_count: 8,
            source_range_sha256: "c".repeat(64),
        };
        RawEffectiveVfsWinnerIndexBinding {
            process_start_nonce: 2,
            authority_manifest_sha256: "d".repeat(64),
            declared_row_count: 1,
            ordered_rows_sha256: RawEffectiveVfsWinnerIndexBinding::canonical_rows_sha256(&[
                row.clone()
            ])
            .unwrap(),
            ordered_rows: vec![row],
        }
    }

    fn source_open() -> RawEffectiveVfsOpenObservation {
        RawEffectiveVfsOpenObservation {
            observation_sequence: 1,
            authority_manifest_sha256: "d".repeat(64),
            canonical_logical_path: content().logical_path,
            content: content(),
            stream_handle: 0x80,
            data_ptr: Some(0x90),
            opened_payload_sha256: "b".repeat(64),
            opened_payload_bytes: 16,
        }
    }

    fn shader_creation(
        stage: RawShaderStage,
        creation_sequence: u64,
        shader_ptr: usize,
        byte: u8,
    ) -> RawShaderCreation {
        RawShaderCreation {
            creation_sequence,
            shader_ptr,
            stage,
            compiler_identity: "observed-compiler".into(),
            target_profile: match stage {
                RawShaderStage::Vertex => "vs_5_0",
                RawShaderStage::Pixel => "ps_5_0",
                _ => unreachable!(),
            }
            .into(),
            compiler_flags_u64: 0,
            compiler_invocation: None,
            bytecode: RawPayloadSnapshot {
                captured_bytes: vec![byte],
                captured_sha256: if stage == RawShaderStage::Vertex {
                    "e".repeat(64)
                } else {
                    "f".repeat(64)
                },
                source: RawPayloadSource::RuntimeGenerated {
                    reason: "observed_compiler_output".into(),
                },
            },
        }
    }

    fn reflection(
        stage: RawShaderStage,
        reflection_sequence: u64,
        creation: &RawShaderCreation,
        entry: &str,
    ) -> RawShaderBytecodeReflectionObservation {
        RawShaderBytecodeReflectionObservation {
            reflection_sequence,
            creation_sequence: creation.creation_sequence,
            shader_ptr: creation.shader_ptr,
            shader_program_ptr: 0x100,
            stage,
            bytecode_sha256: creation.bytecode.captured_sha256.clone(),
            bytecode_bytes: 1,
            entry_token: entry.into(),
            target_profile: creation.target_profile.clone(),
            creator_token: "observed-D3DReflect".into(),
            ordered_inputs: Vec::new(),
            ordered_outputs: Vec::new(),
            ordered_resources: Vec::new(),
        }
    }

    fn target_receipt(capture_sequence: u64) -> RawRenderTargetRegistrationReceipt {
        RawRenderTargetRegistrationObservation {
            registration_sequence: capture_sequence - 1,
            capture_sequence,
            producer_rva: GENERIC_TEXTURE_RTV_PRODUCER_RVA,
            recreate_generation: 0,
            attachment_ordinal: 0,
            kind: RawRegisteredRenderTargetKind::OpaqueRenderTarget {
                source_flags_u32: 2,
            },
            view_ptr: 0x500,
            resource_ptr: 0x501,
            resource_desc: RawTexture2dDesc {
                width: 4,
                height: 4,
                mip_levels: 1,
                array_size: 1,
                dxgi_format_u32: 28,
                sample_count: 1,
                sample_quality: 0,
                usage_u32: 0,
                bind_flags_u32: 0x28,
                cpu_access_flags_u32: 0,
                misc_flags_u32: 0,
            },
            rtv_desc: RawResourceViewDesc {
                dxgi_format_u32: 28,
                view_dimension_u32: 4,
                flags_u32: 0,
                union_words_u32: [0; 4],
            },
        }
        .materialize()
        .unwrap()
    }

    fn draw_identity() -> RawDrawIdentity {
        RawDrawIdentity::Entity {
            submit_index: 1,
            pass_ordinal: 0,
            entity_instance_id: 4,
            draw_order: 0,
            entity_object_ptr: 0x10,
            entity_definition_ptr: 0x11,
            renderer_ptr: 0x12,
            render_context_ptr: 0x13,
            mesh_draw_ptr: 0x14,
            material_binding_ptr: 0x15,
            shader_program_ptr: 0x100,
        }
    }

    fn observations() -> (
        RawLiveDrawAuthorityObservation,
        RawStaticAuthorityObservation,
    ) {
        let vertex = shader_creation(RawShaderStage::Vertex, 2, 0x200, 1);
        let pixel = shader_creation(RawShaderStage::Pixel, 3, 0x300, 2);
        let vertex_reflection = reflection(RawShaderStage::Vertex, 4, &vertex, "VS");
        let pixel_reflection = reflection(RawShaderStage::Pixel, 5, &pixel, "PS");
        let registration = RawProgramRegistrationObservation {
            observation_sequence: 10,
            shader_program_ptr: 0x100,
            registration_root_rva: crate::MAP_EFFECT_REGISTRATION_ROOT_RVA,
            source_open_sequence: 1,
            effect_token: "observed-effect".into(),
            vertex_entry_token: "VS".into(),
            pixel_entry_token: "PS".into(),
            ordered_defines: Vec::new(),
        };
        let live = RawLiveDrawAuthorityObservation {
            observation_sequence: 10,
            identity: draw_identity(),
            content_identity: RawDrawContentIdentity::Entity {
                resolved_entity_asset: content(),
                mesh: content(),
                entity_registration_token: "observed-entity".into(),
                entity_group_index: 0,
                mesh_draw_list_index: 0,
                entity_variant_u8: 0,
                mesh_draw_variant_u8: 0,
                pass_selector_i32: 0,
                alternate_list_u8: 0,
                bind_renderer_target_u8: 0,
            },
            program_registration: registration.clone(),
            source_open: source_open(),
            vertex_creation: vertex.clone(),
            pixel_creation: pixel.clone(),
            vertex_reflection: vertex_reflection.clone(),
            pixel_reflection: pixel_reflection.clone(),
            ordered_color_targets: vec![target_receipt(10)],
            depth_target: None,
        };
        let static_observation = RawStaticAuthorityObservation {
            capture_sequence: 10,
            shader_program_ptr: 0x100,
            program_registration: registration,
            source_open: source_open(),
            vertex_creation: vertex,
            pixel_creation: pixel,
            vertex_reflection,
            pixel_reflection,
            ordered_texture_sources: Vec::new(),
            ordered_color_targets: vec![target_receipt(10)],
            depth_target: None,
            ordered_content: vec![RawStaticContentJoin {
                content_key: "observed-program".into(),
                content: content(),
            }],
        };
        (live, static_observation)
    }

    fn static_core() -> RawStaticD3dDrawState {
        RawStaticD3dDrawState {
            capture_sequence: 10,
            shader_program_ptr: 0x100,
            render_context_ptr: 1,
            device_context_ptr: 2,
            vertex_shader_ptr: 0x200,
            pixel_shader_ptr: 0x300,
            geometry_shader_ptr: None,
            hull_shader_ptr: None,
            domain_shader_ptr: None,
            compute_shader_ptr: None,
            geometry_entry_token: None,
            hull_entry_token: None,
            domain_entry_token: None,
            compute_entry_token: None,
            dynamic_shader_linkage_present: false,
            input_layout_ptr: None,
            blend_state_ptr: None,
            blend_factor_rgba_f32_bits: [0; 4],
            sample_mask_u32: u32::MAX,
            depth_stencil_state_ptr: None,
            stencil_reference_u32: 0,
            rasterizer_state_ptr: None,
            primitive_topology_u32: 4,
            ordered_texture_objects: Vec::new(),
            ordered_sampler_objects: Vec::new(),
            ordered_constant_buffer_objects: Vec::new(),
            ordered_vertex_buffer_objects: Vec::new(),
            index_buffer: None,
            ordered_stream_output_buffers: Vec::new(),
            ordered_color_target_view_ptrs: vec![0x500],
            depth_stencil_view_ptr: None,
            ordered_viewports_f32_bits: Vec::new(),
            ordered_scissor_rects_i32: Vec::new(),
            draw_call: RawStaticDrawCall::Draw {
                vertex_count: 3,
                start_vertex: 0,
            },
            ordered_texture_semantics: Vec::new(),
            ordered_sampler_semantics: Vec::new(),
            ordered_constant_buffer_semantics: Vec::new(),
            ordered_color_targets: Vec::new(),
            depth_target: None,
            ordered_content: Vec::new(),
        }
    }

    fn live_capture() -> RawRenderSubmitCapture {
        RawRenderSubmitCapture {
            submit: RawParticleSubmit {
                submit_index: 1,
                camera_carrier: None,
                packets: Vec::new(),
                entity_submissions: vec![crate::RawEntitySubmission {
                    submit_index: 1,
                    pass_ordinal: 0,
                    draw_order: 0,
                    entity_instance_id: 4,
                    entity_registration_token: Some("test_entity".into()),
                    entity_object_ptr: 0x10,
                    entity_definition_ptr: 0x11,
                    renderer_ptr: 0x12,
                    render_context_ptr: 0x13,
                    mesh_draw_ptr: 0x14,
                    material_binding_ptr: 0x15,
                    entity_group_index: 0,
                    mesh_draw_list_index: 0,
                    entity_variant_u8: 0,
                    mesh_draw_variant_u8: 0,
                    pass_selector_i32: 0,
                    bind_renderer_target_u8: 0,
                    alternate_list_u8: 0,
                    shader_program_ptr: 0x100,
                    additional_shader_stages_authenticated_null: true,
                    stream_output_targets_authenticated_null: true,
                    world_transform_4x4_f32_bits: [0; 16],
                    state_flags: 0,
                    skinning: crate::RawSkinning::NotSkinned,
                }],
            },
            shader_reflections: Vec::new(),
            catalog: RawRenderCatalogCapture {
                process: RawProcessEpoch {
                    process_id: 1,
                    process_start_nonce: 2,
                    executable_sha256: "1".repeat(64),
                },
                launch_epoch: RawCaptureLaunchEpoch {
                    process_id: 1,
                    process_start_nonce: 2,
                    observer_installed_qpc: 1,
                    d3d_device_created_qpc: 2,
                    first_creation_sequence: 1,
                    hook_epoch_nonce: 3,
                    hook_set_receipt_sha256: "2".repeat(64),
                    capture_receipt_sha256: "3".repeat(64),
                },
                submit_index: 1,
                frame_sequence: 1,
                draw_states: vec![RawD3dDrawState {
                    final_draw_sequence: 10,
                    identity: draw_identity(),
                    family: RawDrawFamily::Entity,
                    submit_index: 1,
                    draw_order: 0,
                    render_context_ptr: 0x13,
                    device_context_ptr: 2,
                    shader_program_ptr: 0x100,
                    vertex_shader_ptr: 0x200,
                    pixel_shader_ptr: 0x300,
                    geometry_shader_ptr: None,
                    hull_shader_ptr: None,
                    domain_shader_ptr: None,
                    compute_shader_ptr: None,
                    dynamic_shader_linkage_present: false,
                    input_layout_ptr: None,
                    blend_state_ptr: None,
                    blend_factor_rgba_f32_bits: [0; 4],
                    sample_mask_u32: u32::MAX,
                    depth_stencil_state_ptr: None,
                    stencil_reference_u32: 0,
                    rasterizer_state_ptr: None,
                    primitive_topology_u32: 4,
                    ordered_texture_objects: Vec::new(),
                    ordered_sampler_objects: Vec::new(),
                    ordered_constant_buffer_objects: Vec::new(),
                    ordered_vertex_buffer_objects: Vec::new(),
                    index_buffer: None,
                    ordered_stream_output_buffer_ptrs: Vec::new(),
                    ordered_color_target_view_ptrs: vec![0x500],
                    depth_stencil_view_ptr: None,
                    ordered_viewports_f32_bits: Vec::new(),
                    ordered_scissor_rects_i32: Vec::new(),
                    draw_call: RawDrawCall::Draw {
                        vertex_count: 3,
                        start_vertex: 0,
                    },
                }],
                draw_semantics: Vec::new(),
                program_identities: Vec::new(),
                shader_creations: Vec::new(),
                input_layout_creations: Vec::new(),
                blend_state_creations: Vec::new(),
                depth_stencil_state_creations: Vec::new(),
                rasterizer_state_creations: Vec::new(),
                sampler_state_creations: Vec::new(),
                buffer_snapshots: Vec::new(),
                texture_resource_snapshots: Vec::new(),
                shader_resource_view_snapshots: Vec::new(),
                render_target_view_snapshots: Vec::new(),
            },
        }
    }

    fn foreign_content(sequence: u64) -> RawVfsContentIdentity {
        RawVfsContentIdentity {
            logical_path: format!("gfx/fx/unselected_{sequence}.shader"),
            winning_layer_ordinal: 77,
            layer_kind: "mod".into(),
            layer_id: "mod/unselected.mod".into(),
            source_sha256: "4".repeat(64),
            payload_sha256: "5".repeat(64),
            payload_bytes: 16,
        }
    }

    fn retarget_observations(
        sequence: u64,
        shader_program_ptr: usize,
        foreign: bool,
    ) -> (
        RawLiveDrawAuthorityObservation,
        RawStaticAuthorityObservation,
    ) {
        let (mut live, mut static_observation) = observations();
        let content = foreign.then(|| foreign_content(sequence));
        live.observation_sequence = sequence;
        live.program_registration.observation_sequence = sequence;
        live.program_registration.shader_program_ptr = shader_program_ptr;
        live.vertex_reflection.shader_program_ptr = shader_program_ptr;
        live.pixel_reflection.shader_program_ptr = shader_program_ptr;
        live.ordered_color_targets[0].capture_sequence = sequence;
        if foreign {
            if let RawDrawIdentity::Entity {
                entity_instance_id,
                draw_order,
                shader_program_ptr: identity_program,
                ..
            } = &mut live.identity
            {
                *entity_instance_id = sequence;
                *draw_order = sequence as u32;
                *identity_program = shader_program_ptr;
            }
        }
        if let Some(content) = &content {
            live.source_open.canonical_logical_path = content.logical_path.clone();
            live.source_open.content = content.clone();
            live.source_open.opened_payload_sha256 = content.payload_sha256.clone();
            live.source_open.opened_payload_bytes = content.payload_bytes;
            live.content_identity = RawDrawContentIdentity::Entity {
                resolved_entity_asset: content.clone(),
                mesh: content.clone(),
                entity_registration_token: "observed-entity".into(),
                entity_group_index: 0,
                mesh_draw_list_index: 0,
                entity_variant_u8: 0,
                mesh_draw_variant_u8: 0,
                pass_selector_i32: 0,
                alternate_list_u8: 0,
                bind_renderer_target_u8: 0,
            };
        }

        static_observation.capture_sequence = sequence;
        static_observation.shader_program_ptr = shader_program_ptr;
        static_observation.program_registration.observation_sequence = sequence;
        static_observation.program_registration.shader_program_ptr = shader_program_ptr;
        static_observation.vertex_reflection.shader_program_ptr = shader_program_ptr;
        static_observation.pixel_reflection.shader_program_ptr = shader_program_ptr;
        static_observation.ordered_color_targets[0].capture_sequence = sequence;
        if let Some(content) = content {
            static_observation.source_open.canonical_logical_path = content.logical_path.clone();
            static_observation.source_open.content = content.clone();
            static_observation.source_open.opened_payload_sha256 = content.payload_sha256.clone();
            static_observation.source_open.opened_payload_bytes = content.payload_bytes;
            static_observation.ordered_content[0].content = content;
        }
        (live, static_observation)
    }

    #[test]
    fn neutral_receipts_feed_strict_d80_sink_without_role_inference() {
        let raw = RawStaticAuthorityMaterialization {
            capture_sequence: 7,
            shader_program_ptr: 0x100,
            program_identity: RawProgramIdentity {
                observation_sequence: 7,
                shader_program_ptr: 0x100,
                registration_root_rva: crate::MAP_EFFECT_REGISTRATION_ROOT_RVA,
                source: content(),
                effect_token: "observed-effect".into(),
                vertex_entry_token: "VS".into(),
                pixel_entry_token: "PS".into(),
                ordered_defines: Vec::new(),
            },
            ordered_texture_semantics: Vec::new(),
            ordered_sampler_semantics: Vec::new(),
            ordered_constant_buffer_semantics: Vec::new(),
            ordered_color_targets: vec![target()],
            depth_target: None,
            ordered_content: vec![RawStaticContentJoin {
                content_key: "observed-source".into(),
                content: content(),
            }],
        };
        let converted = observed_static_draw_authority(raw.clone()).unwrap();
        assert_eq!(converted.program_identity.source.winning_layer_ordinal, 9);

        let mut sink = ProductionStaticCaptureAuthority::default();
        sink.observe_render_target_receipt(target_receipt(7))
            .unwrap();
        sink.observe_materialized_static_authority(raw).unwrap();
    }

    #[test]
    fn concrete_live_registry_binds_winners_and_rejects_unmatched_or_reordered_rows() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ProductionExactLiveRenderCatalogSemanticsRegistry>();

        let registry = ProductionExactLiveRenderCatalogSemanticsRegistry::new(limits()).unwrap();
        let (live, _) = observations();
        assert!(registry
            .observe_live_draw_authority(1, live.clone())
            .is_err());
        registry
            .bind_effective_vfs_winners(winner_binding())
            .unwrap();
        registry
            .observe_live_draw_authority(1, live.clone())
            .unwrap();
        assert!(registry.observe_live_draw_authority(1, live).is_err());

        let mut mismatched = live_capture();
        mismatched.catalog.draw_states[0].final_draw_sequence = 11;
        assert!(registry
            .finalize_live_submit_authority(&mismatched)
            .is_err());
        let mut capture = live_capture();
        registry.finalize_live_submit_authority(&capture).unwrap();
        let adapters = registry
            .drain_complete_live_render_catalog_semantics()
            .unwrap();
        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].draw_count(), 1);
        crate::materialize_live_render_catalog_semantics(&mut capture, &adapters[0]).unwrap();
        assert_eq!(capture.catalog.program_identities.len(), 1);
        assert_eq!(capture.catalog.draw_semantics.len(), 1);
        assert_eq!(
            capture.catalog.draw_semantics[0].ordered_color_targets[0].view_ptr,
            0x500
        );
    }

    #[test]
    fn concrete_static_registry_finalizes_atomically_into_strict_sink() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ProductionStaticAuthorityRegistry>();

        let registry = ProductionStaticAuthorityRegistry::new(limits()).unwrap();
        let (_, observed) = observations();
        registry
            .bind_effective_vfs_winners(winner_binding())
            .unwrap();
        registry.observe_static_authority(7, observed).unwrap();
        let core = static_core();
        assert!(registry
            .finalize_static_authority(7, 99, &[core.clone()])
            .is_err());
        let mut reordered = core.clone();
        reordered.capture_sequence = 11;
        assert!(registry
            .finalize_static_authority(7, 2, &[reordered])
            .is_err());
        registry
            .finalize_static_authority(7, 2, &[core.clone()])
            .unwrap();
        let frames = registry.drain_complete_static_authorities().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].frame_sequence(), 7);
        let mut sink = ProductionStaticCaptureAuthority::default();
        frames
            .into_iter()
            .next()
            .unwrap()
            .apply_to(&mut sink)
            .unwrap();
        let materialized = sink.materialize_draw(core).unwrap();
        assert_eq!(materialized.ordered_color_targets[0].view_ptr, 0x500);
        assert_eq!(materialized.ordered_content[0].content, content());
    }

    #[test]
    fn empty_unselected_group_proves_exact_absence_when_registries_exist() {
        let live_registry =
            ProductionExactLiveRenderCatalogSemanticsRegistry::new(limits()).unwrap();
        let static_registry = ProductionStaticAuthorityRegistry::new(limits()).unwrap();
        live_registry
            .bind_effective_vfs_winners(winner_binding())
            .unwrap();
        static_registry
            .bind_effective_vfs_winners(winner_binding())
            .unwrap();
        let (live, static_observation) = observations();
        live_registry.observe_live_draw_authority(1, live).unwrap();
        static_registry
            .observe_static_authority(7, static_observation)
            .unwrap();
        let empty = UnselectedAuthorityGroup {
            submit_index: 1,
            ordered_live_observation_sequences: Vec::new(),
            frame_sequence: 7,
            ordered_static_capture_sequences: Vec::new(),
        };
        let live_state = live_registry.state.lock().unwrap();
        let static_state = static_registry.state.lock().unwrap();
        assert!(validate_unselected_authority_groups(
            std::slice::from_ref(&empty),
            &live_state,
            &static_state,
        )
        .is_err());
        let exact = UnselectedAuthorityGroup {
            ordered_live_observation_sequences: vec![10],
            ordered_static_capture_sequences: vec![10],
            ..empty
        };
        validate_unselected_authority_groups(
            std::slice::from_ref(&exact),
            &live_state,
            &static_state,
        )
        .unwrap();
    }

    #[test]
    fn live_selector_discards_exact_ids_and_admits_only_selected_content() {
        let registry = ProductionExactLiveRenderCatalogSemanticsRegistry::new(limits()).unwrap();
        registry
            .bind_effective_vfs_winners(winner_binding())
            .unwrap();
        let (selected, _) = retarget_observations(10, 0x100, false);
        registry.observe_live_draw_authority(1, selected).unwrap();
        for (sequence, program) in [(11, 0x101), (12, 0x102), (13, 0x103)] {
            let (unselected, _) = retarget_observations(sequence, program, true);
            registry.observe_live_draw_authority(1, unselected).unwrap();
        }
        let (overflow, _) = retarget_observations(14, 0x104, true);
        assert!(registry.observe_live_draw_authority(1, overflow).is_err());
        let (future, _) = retarget_observations(20, 0x120, true);
        registry.observe_live_draw_authority(2, future).unwrap();
        assert!(registry
            .finalize_selected_live_submit_authority(&live_capture(), &[999])
            .is_err());
        registry
            .finalize_selected_live_submit_authority(&live_capture(), &[11, 12, 13])
            .unwrap();
        assert_eq!(
            registry
                .drain_complete_live_render_catalog_semantics()
                .unwrap()[0]
                .draw_count(),
            1
        );
        assert!(registry
            .discard_unselected_live_authority_ids(2, &[11])
            .is_err());
        registry
            .discard_unselected_live_authority_ids(2, &[20])
            .unwrap();
    }

    #[test]
    fn static_selector_discards_exact_ids_and_preserves_future_rows() {
        let registry = ProductionStaticAuthorityRegistry::new(limits()).unwrap();
        registry
            .bind_effective_vfs_winners(winner_binding())
            .unwrap();
        let (_, selected) = retarget_observations(10, 0x100, false);
        registry.observe_static_authority(7, selected).unwrap();
        for (sequence, program) in [(11, 0x101), (12, 0x102), (13, 0x103)] {
            let (_, unselected) = retarget_observations(sequence, program, true);
            registry.observe_static_authority(7, unselected).unwrap();
        }
        let (_, overflow) = retarget_observations(14, 0x104, true);
        assert!(registry.observe_static_authority(7, overflow).is_err());
        let (_, future) = retarget_observations(20, 0x120, true);
        registry.observe_static_authority(8, future).unwrap();
        assert!(registry
            .finalize_selected_static_authority(7, 2, &[static_core()], &[999])
            .is_err());
        registry
            .finalize_selected_static_authority(7, 2, &[static_core()], &[11, 12, 13])
            .unwrap();
        assert_eq!(
            registry.drain_complete_static_authorities().unwrap().len(),
            1
        );
        assert!(registry
            .discard_unselected_static_authority_ids(8, &[11])
            .is_err());
        registry
            .discard_unselected_static_authority_ids(8, &[20])
            .unwrap();
    }
}
