
pub fn every() -> Option<u64> {
    static EVERY: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *EVERY.get_or_init(|| {
        let value = std::env::var_os("BATTLEPLAN_VIEWER_CHECK_ACTORS").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_ACTORS"))?;
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

pub fn due(calls: &mut u64) -> bool {
    let Some(every) = every() else {
        return false;
    };
    *calls += 1;
    (*calls - 1) % every == 0
}

pub fn differs(what: &str, element: &dyn std::fmt::Debug, difference: &dyn std::fmt::Debug) {
    eprintln!("CHECK {what} DIFFERS at {element:?}: {difference:?}");
    viewer_runtime::view_record::add("actors check", "differs", 1);
    viewer_runtime::view_record::add_named("actors check", &format!("differs: {what}"), 1);
}

pub fn checked(what: &'static str, elements: u64) {
    viewer_runtime::view_record::add("actors check", "checks", 1);
    viewer_runtime::view_record::add_named("actors check", &format!("checked: {what}"), elements);
}
