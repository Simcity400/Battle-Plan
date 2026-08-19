use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use production_hook_receipt::ReleasedRvaHookReceipt;

use crate::a729_hook::{CombatBinding, SealedTerminalReservation};
use crate::x2_carriers::{
    LossSideBinding, LossTerminalReservation, PanelTerminalReservation, SideBinding,
    WidthTerminalReservation,
};
use crate::CombatSide;

static CAPTURE_ADMISSION_CLOSED: AtomicBool = AtomicBool::new(false);
static COMBINED_CUSTODY_ACTIVE: AtomicBool = AtomicBool::new(false);
static ACTIVE_INDIVIDUAL_API_MUTATIONS: AtomicUsize = AtomicUsize::new(0);
static NEXT_CUTOFF_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_COMBINED_RESERVATION_ID: AtomicU64 = AtomicU64::new(1);
static COORDINATOR: Mutex<CoordinatorState> = Mutex::new(CoordinatorState::new());

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalCommittedSide {
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
    pub committed: u32,
    pub counts_incomplete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalD55TerminalBindings {
    pub terminal: Vec<CombatBinding>,
    pub width: Vec<SideBinding>,
    pub current_loss: Vec<LossSideBinding>,
    pub panel: Vec<SideBinding>,
    pub committed_sides: Vec<FinalCommittedSide>,
}

pub struct CombinedTerminalCutoffGuard {
    cutoff_id: u64,
}

pub struct CombinedSealFailure {
    pub message: String,
    pub guard: CombinedTerminalCutoffGuard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CombinedTerminalRetryToken {
    cutoff_id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CombinedD55TerminalReservation {
    pub reservation_id: u64,
    pub cutoff_id: u64,
    pub terminal: SealedTerminalReservation,
    pub terminal_released_hooks: [ReleasedRvaHookReceipt; 3],
    pub width: WidthTerminalReservation,
    pub current_loss: LossTerminalReservation,
    pub panel: PanelTerminalReservation,
}

struct CoordinatorState {
    cutoff_id: Option<u64>,
    final_bindings: Option<FinalD55TerminalBindings>,
    terminal: Option<SealedTerminalReservation>,
    terminal_released_hooks: Option<[ReleasedRvaHookReceipt; 3]>,
    width: Option<WidthTerminalReservation>,
    current_loss: Option<LossTerminalReservation>,
    panel: Option<PanelTerminalReservation>,
    combined_reservation_id: Option<u64>,
}

impl CoordinatorState {
    const fn new() -> Self {
        Self {
            cutoff_id: None,
            final_bindings: None,
            terminal: None,
            terminal_released_hooks: None,
            width: None,
            current_loss: None,
            panel: None,
            combined_reservation_id: None,
        }
    }

    fn clear_lane_reservations(&mut self) {
        self.terminal = None;
        self.terminal_released_hooks = None;
        self.width = None;
        self.current_loss = None;
        self.panel = None;
        self.combined_reservation_id = None;
    }
}

pub(crate) fn try_admit_callback(admitted: &'static AtomicUsize) -> bool {
    loop {
        if CAPTURE_ADMISSION_CLOSED.load(Ordering::Acquire) {
            return false;
        }
        admitted.fetch_add(1, Ordering::AcqRel);
        if !CAPTURE_ADMISSION_CLOSED.load(Ordering::Acquire) {
            return true;
        }
        admitted.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) fn release_admitted_callback(admitted: &'static AtomicUsize) {
    admitted.fetch_sub(1, Ordering::AcqRel);
}

pub(crate) fn individual_terminal_api_blocked() -> bool {
    COMBINED_CUSTODY_ACTIVE.load(Ordering::Acquire)
}

pub(crate) struct IndividualApiMutationGuard;

impl Drop for IndividualApiMutationGuard {
    fn drop(&mut self) {
        ACTIVE_INDIVIDUAL_API_MUTATIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) fn try_admit_individual_api(label: &str) -> Result<IndividualApiMutationGuard, String> {
    loop {
        if individual_terminal_api_blocked() {
            return Err(format!(
                "{label} is blocked by the D55 combined terminal cutoff"
            ));
        }
        ACTIVE_INDIVIDUAL_API_MUTATIONS.fetch_add(1, Ordering::AcqRel);
        if !individual_terminal_api_blocked() {
            return Ok(IndividualApiMutationGuard);
        }
        ACTIVE_INDIVIDUAL_API_MUTATIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) fn wait_for_individual_api_quiescence() -> Result<(), String> {
    let mut yields = 0u32;
    while ACTIVE_INDIVIDUAL_API_MUTATIONS.load(Ordering::Acquire) != 0 {
        if yields == 1_000_000 {
            return Err("D55 individual APIs did not quiesce for combined cutoff".into());
        }
        yields += 1;
        std::thread::yield_now();
    }
    Ok(())
}

fn next_id(counter: &AtomicU64, label: &str) -> Result<u64, String> {
    let id = counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .map_err(|_| format!("{label} id exhausted"))?;
    (id != 0)
        .then_some(id)
        .ok_or_else(|| format!("{label} id is zero"))
}

pub fn begin_combined_terminal_cutoff() -> Result<CombinedTerminalCutoffGuard, String> {
    if COMBINED_CUSTODY_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("D55 combined terminal cutoff or custody is already active".into());
    }
    if CAPTURE_ADMISSION_CLOSED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        COMBINED_CUSTODY_ACTIVE.store(false, Ordering::Release);
        return Err("D55 combined capture admission was already closed".into());
    }
    let cutoff_id = match next_id(&NEXT_CUTOFF_ID, "D55 cutoff") {
        Ok(id) => id,
        Err(error) => {
            CAPTURE_ADMISSION_CLOSED.store(false, Ordering::Release);
            COMBINED_CUSTODY_ACTIVE.store(false, Ordering::Release);
            return Err(error);
        }
    };
    {
        let mut state = COORDINATOR
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.cutoff_id.is_some() {
            CAPTURE_ADMISSION_CLOSED.store(false, Ordering::Release);
            COMBINED_CUSTODY_ACTIVE.store(false, Ordering::Release);
            return Err("D55 combined coordinator retains prior cutoff custody".into());
        }
        state.cutoff_id = Some(cutoff_id);
    }
    if let Err(error) = wait_for_individual_api_quiescence()
        .and_then(|()| crate::a729_hook::wait_for_combined_capture_quiescence())
        .and_then(|()| crate::x2_carriers::wait_for_combined_capture_quiescence())
    {
        let mut state = COORDINATOR
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.cutoff_id == Some(cutoff_id) && state.final_bindings.is_none() {
            *state = CoordinatorState::new();
            CAPTURE_ADMISSION_CLOSED.store(false, Ordering::Release);
            COMBINED_CUSTODY_ACTIVE.store(false, Ordering::Release);
        }
        return Err(format!(
            "D55 combined cutoff quiescence failed without sealing custody: {error}"
        ));
    }
    Ok(CombinedTerminalCutoffGuard { cutoff_id })
}

fn validate_final_bindings(bindings: &FinalD55TerminalBindings) -> Result<(), String> {
    if bindings.terminal.is_empty()
        && (!bindings.width.is_empty()
            || !bindings.current_loss.is_empty()
            || !bindings.panel.is_empty()
            || !bindings.committed_sides.is_empty())
    {
        return Err("D55 final bindings contain side identities without terminal combats".into());
    }
    let mut committed = std::collections::BTreeMap::new();
    for side in &bindings.committed_sides {
        if side.combat_id == 0
            || side.generation == 0
            || committed
                .insert((side.combat_id, side.generation, side.side), side)
                .is_some()
        {
            return Err("D55 final committed-side roster is null or duplicated".into());
        }
    }
    let mut terminal_epochs = std::collections::BTreeSet::new();
    for combat in &bindings.terminal {
        if combat.combat_ptr == 0
            || combat.combat_id == 0
            || combat.generation == 0
            || !terminal_epochs.insert((combat.combat_id, combat.generation))
        {
            return Err("D55 final terminal-combat roster is null or duplicated".into());
        }
    }
    if committed
        .keys()
        .any(|(combat_id, generation, _)| !terminal_epochs.contains(&(*combat_id, *generation)))
    {
        return Err("D55 final committed-side roster lacks a terminal-combat authority".into());
    }
    for (label, combat_id, generation, side) in bindings
        .width
        .iter()
        .map(|binding| ("width", binding.combat_id, binding.generation, binding.side))
        .chain(bindings.current_loss.iter().map(|binding| {
            (
                "current-loss",
                binding.combat_id,
                binding.generation,
                binding.side,
            )
        }))
        .chain(
            bindings
                .panel
                .iter()
                .map(|binding| ("panel", binding.combat_id, binding.generation, binding.side)),
        )
    {
        if !terminal_epochs.contains(&(combat_id, generation)) {
            return Err(format!(
                "D55 final {label} binding lacks a terminal-combat authority"
            ));
        }
        if !committed.contains_key(&(combat_id, generation, side)) {
            return Err(format!(
                "D55 final {label} binding lacks a committed-side authority"
            ));
        }
    }
    Ok(())
}

fn validate_width_committed_counts(
    width: &crate::x2_carriers::WidthTerminalDrain,
    bindings: &FinalD55TerminalBindings,
) -> Result<(), String> {
    let committed: std::collections::BTreeMap<_, _> = bindings
        .committed_sides
        .iter()
        .map(|side| ((side.combat_id, side.generation, side.side), side))
        .collect();
    let mut groups: std::collections::BTreeMap<_, (u32, std::collections::BTreeSet<u32>)> =
        std::collections::BTreeMap::new();
    if width.completeness_poisoned || width.projection_failures != 0 {
        return Err("D55 terminal width projection is incomplete or poisoned".into());
    }
    for row in &width.rows {
        let side = committed
            .get(&(row.combat_id, row.generation, row.side))
            .ok_or_else(|| {
                "D55 terminal width row lacks final committed-side authority".to_string()
            })?;
        if side.counts_incomplete {
            return Err(format!(
                "D55 terminal width wrapper count has incomplete final participant authority for combat {}:{}",
                row.combat_id, row.generation
            ));
        }
        if side.committed != row.wrapper_count {
            return Err(format!(
                "D55 terminal width wrapper count {} disagrees with final committed count {} for combat {}:{}",
                row.wrapper_count, side.committed, row.combat_id, row.generation
            ));
        }
        let group = groups
            .entry((
                row.combat_id,
                row.generation,
                row.side,
                row.capture_sequence,
            ))
            .or_insert_with(|| (row.wrapper_count, std::collections::BTreeSet::new()));
        if group.0 != row.wrapper_count
            || row.ordinal >= row.wrapper_count
            || !group.1.insert(row.ordinal)
        {
            return Err("D55 terminal width group has inconsistent or duplicate ordinals".into());
        }
    }
    if groups
        .values()
        .any(|(wrapper_count, ordinals)| ordinals.len() != *wrapper_count as usize)
    {
        return Err("D55 terminal width group is missing one or more wrapper rows".into());
    }
    Ok(())
}

pub fn seal_combined_terminal(
    guard: CombinedTerminalCutoffGuard,
    bindings: FinalD55TerminalBindings,
) -> Result<CombinedD55TerminalReservation, CombinedSealFailure> {
    match seal_combined_terminal_inner(guard.cutoff_id, bindings) {
        Ok(reservation) => Ok(reservation),
        Err(message) => Err(CombinedSealFailure { message, guard }),
    }
}

fn seal_combined_terminal_inner(
    cutoff_id: u64,
    bindings: FinalD55TerminalBindings,
) -> Result<CombinedD55TerminalReservation, String> {
    validate_final_bindings(&bindings)?;
    let mut state = COORDINATOR
        .lock()
        .map_err(|_| "D55 combined coordinator mutex poisoned".to_string())?;
    if state.cutoff_id != Some(cutoff_id) || state.combined_reservation_id.is_some() {
        return Err("D55 cutoff guard is stale or already reserved".into());
    }
    match &state.final_bindings {
        Some(existing) if existing != &bindings => {
            return Err("D55 final binding snapshot changed across seal retry".into())
        }
        Some(_) => {}
        None => {
            crate::a729_hook::publish_combined_final_bindings(&bindings.terminal)?;
            let projected = crate::x2_carriers::publish_combined_final_bindings(
                &bindings.width,
                &bindings.current_loss,
                &bindings.panel,
            )?;
            validate_width_committed_counts(&projected.width, &bindings)?;
            if projected.current_loss.completeness_poisoned
                || projected.current_loss.projection_failures != 0
            {
                return Err(
                    "D55 terminal current-loss projection is incomplete or poisoned".into(),
                );
            }
            state.final_bindings = Some(bindings.clone());
        }
    }
    if state.terminal.is_none() {
        state.terminal = Some(crate::a729_hook::seal_and_reserve_all_for_combined()?);
        state.terminal_released_hooks = Some(crate::a729_hook::combined_released_hook_receipts()?);
    }
    if state.width.is_none() {
        state.width = Some(crate::x2_carriers::seal_and_reserve_width_for_combined()?);
    }
    if state.current_loss.is_none() {
        state.current_loss = Some(crate::x2_carriers::seal_and_reserve_loss_for_combined()?);
    }
    if state.panel.is_none() {
        state.panel = Some(crate::x2_carriers::seal_and_reserve_panel_for_combined()?);
    }
    validate_width_committed_counts(
        &state
            .width
            .as_ref()
            .expect("combined width reservation")
            .drain,
        state
            .final_bindings
            .as_ref()
            .expect("combined final bindings"),
    )?;
    let reservation_id = next_id(&NEXT_COMBINED_RESERVATION_ID, "D55 combined reservation")?;
    state.combined_reservation_id = Some(reservation_id);
    Ok(CombinedD55TerminalReservation {
        reservation_id,
        cutoff_id,
        terminal: state
            .terminal
            .clone()
            .expect("combined terminal reservation"),
        terminal_released_hooks: state
            .terminal_released_hooks
            .expect("combined terminal release receipts"),
        width: state.width.clone().expect("combined width reservation"),
        current_loss: state
            .current_loss
            .clone()
            .expect("combined loss reservation"),
        panel: state.panel.clone().expect("combined panel reservation"),
    })
}

pub fn commit_combined_terminal(reservation_id: u64) -> Result<(), String> {
    let mut state = COORDINATOR
        .lock()
        .map_err(|_| "D55 combined coordinator mutex poisoned".to_string())?;
    if state.combined_reservation_id != Some(reservation_id) {
        return Err("D55 combined terminal reservation id mismatch".into());
    }
    let terminal = state
        .terminal
        .as_ref()
        .expect("combined terminal reservation");
    let width = state.width.as_ref().expect("combined width reservation");
    let current_loss = state
        .current_loss
        .as_ref()
        .expect("combined loss reservation");
    let panel = state.panel.as_ref().expect("combined panel reservation");
    validate_width_committed_counts(
        &width.drain,
        state
            .final_bindings
            .as_ref()
            .expect("combined final bindings"),
    )?;
    crate::a729_hook::preflight_combined_commit(terminal.reservation_id)?;
    crate::x2_carriers::preflight_combined_terminal_commit(
        width.reservation_id,
        current_loss.reservation_id,
        panel.reservation_id,
    )?;

    // Every fallible check precedes this point. Combined ownership blocks all
    crate::a729_hook::commit_combined_preflighted(terminal.reservation_id);
    crate::x2_carriers::commit_combined_terminal_preflighted(
        width.reservation_id,
        current_loss.reservation_id,
        panel.reservation_id,
    );
    *state = CoordinatorState::new();
    CAPTURE_ADMISSION_CLOSED.store(false, Ordering::Release);
    COMBINED_CUSTODY_ACTIVE.store(false, Ordering::Release);
    Ok(())
}

pub fn rollback_combined_terminal(
    reservation_id: u64,
) -> Result<CombinedTerminalRetryToken, String> {
    let mut state = COORDINATOR
        .lock()
        .map_err(|_| "D55 combined coordinator mutex poisoned".to_string())?;
    if state.combined_reservation_id != Some(reservation_id) {
        return Err("D55 combined terminal reservation id mismatch".into());
    }
    let terminal = state
        .terminal
        .as_ref()
        .expect("combined terminal reservation");
    let width = state.width.as_ref().expect("combined width reservation");
    let current_loss = state
        .current_loss
        .as_ref()
        .expect("combined loss reservation");
    let panel = state.panel.as_ref().expect("combined panel reservation");
    crate::a729_hook::preflight_combined_rollback(terminal.reservation_id)?;
    crate::x2_carriers::preflight_combined_terminal_rollback(
        width.reservation_id,
        current_loss.reservation_id,
        panel.reservation_id,
    )?;
    crate::a729_hook::rollback_combined_preflighted(terminal.reservation_id);
    crate::x2_carriers::rollback_combined_terminal_preflighted(
        width.reservation_id,
        current_loss.reservation_id,
        panel.reservation_id,
    );
    let token = CombinedTerminalRetryToken {
        cutoff_id: state.cutoff_id.expect("combined cutoff id"),
    };
    state.clear_lane_reservations();
    Ok(token)
}

pub fn reserve_combined_terminal_retry(
    token: CombinedTerminalRetryToken,
) -> Result<CombinedD55TerminalReservation, String> {
    let bindings = {
        let state = COORDINATOR
            .lock()
            .map_err(|_| "D55 combined coordinator mutex poisoned".to_string())?;
        if state.cutoff_id != Some(token.cutoff_id)
            || state.combined_reservation_id.is_some()
            || !COMBINED_CUSTODY_ACTIVE.load(Ordering::Acquire)
            || !CAPTURE_ADMISSION_CLOSED.load(Ordering::Acquire)
        {
            return Err("D55 combined terminal retry token is stale".into());
        }
        state
            .final_bindings
            .clone()
            .ok_or_else(|| "D55 combined retry lost final bindings".to_string())?
    };
    seal_combined_terminal_inner(token.cutoff_id, bindings)
}
