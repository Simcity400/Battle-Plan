//! The actors' judge: `BATTLE_PLAN_VIEWER_CHECK_ACTORS` makes a frame run the walk it would have run
//! before anything was made lazy, on a copy of the state, and compare the two element by element.
//!
//! The eager walk is kept only for the judge, so the two cannot drift: the frame's own path and
//! the judge call the same code with the same inputs, and the judge's copy is thrown away. A
//! difference names the element and the field, on standard error and in the frame's record
//! (`actors check`), so a gate run says both that the check fired and that nothing differed.
//!
//! The value is how many frames apart the check runs, so a long run can be checked at a bounded
//! rate; any non-numeric value checks every frame.

/// How many frames apart the judge runs, or `None` when it is off.
pub fn every() -> Option<u64> {
    static EVERY: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *EVERY.get_or_init(|| {
        let value = std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_ACTORS").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_ACTORS"))?;
        let value = value.to_str()?;
        if value.is_empty() || value == "0" {
            return None;
        }
        Some(
            value
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|every| *every > 0)
                .unwrap_or(1),
        )
    })
}

/// Whether this call of a walk is one the judge checks. Each walk counts its own calls.
pub fn due(calls: &mut u64) -> bool {
    let Some(every) = every() else {
        return false;
    };
    *calls += 1;
    (*calls - 1) % every == 0
}

/// What the judge found: `what` names the walk, `element` the slot, province or figure, and
/// `difference` the field and the two values.
pub fn differs(what: &str, element: &dyn std::fmt::Debug, difference: &dyn std::fmt::Debug) {
    eprintln!("CHECK {what} DIFFERS at {element:?}: {difference:?}");
    viewer_foundation::view_record::add("actors check", "differs", 1);
    viewer_foundation::view_record::add_named("actors check", &format!("differs: {what}"), 1);
}

/// That the judge ran this frame, so a gate run shows the check fired.
pub fn checked(what: &'static str, elements: u64) {
    viewer_foundation::view_record::add("actors check", "checks", 1);
    viewer_foundation::view_record::add_named("actors check", &format!("checked: {what}"), elements);
}
