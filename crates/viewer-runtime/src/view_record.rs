use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

pub type Sections = Vec<(&'static str, Vec<(&'static str, u64)>)>;

static ON: AtomicBool = AtomicBool::new(false);
static RECORD: Mutex<Sections> = Mutex::new(Vec::new());

pub fn enable() {
    ON.store(true, Ordering::Release);
}

pub fn on() -> bool {
    ON.load(Ordering::Acquire)
}

fn with(section: &'static str, name: &'static str, apply: impl FnOnce(&mut u64)) {
    let mut record = RECORD.lock().unwrap_or_else(|error| error.into_inner());
    let at = match record.iter().position(|(given, _)| *given == section) {
        Some(at) => at,
        None => {
            record.push((section, Vec::new()));
            record.len() - 1
        }
    };
    let counts = &mut record[at].1;
    match counts.iter_mut().find(|(given, _)| *given == name) {
        Some((_, value)) => apply(value),
        None => {
            let mut value = 0;
            apply(&mut value);
            counts.push((name, value));
        }
    }
}

pub fn add(section: &'static str, name: &'static str, n: u64) {
    if on() {
        with(section, name, |value| *value += n);
    }
}

pub fn add_named(section: &'static str, name: &str, n: u64) {
    if !on() {
        return;
    }
    static NAMES: Mutex<std::collections::BTreeSet<&'static str>> = Mutex::new(std::collections::BTreeSet::new());
    let name = {
        let mut names = NAMES.lock().unwrap_or_else(|error| error.into_inner());
        match names.get(name) {
            Some(kept) => *kept,
            None => {
                let kept: &'static str = Box::leak(name.to_owned().into_boxed_str());
                names.insert(kept);
                kept
            }
        }
    };
    with(section, name, |value| *value += n);
}

pub fn set(section: &'static str, name: &'static str, n: u64) {
    if on() {
        with(section, name, |value| *value = n);
    }
}

pub fn take() -> Sections {
    std::mem::take(&mut *RECORD.lock().unwrap_or_else(|error| error.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_sum_levels_stand_and_a_take_starts_the_frame_again() {
        enable();
        take();
        add("unit models", "lit_draws", 3);
        add("unit models", "lit_draws", 4);
        set("unit models", "lit_figures", 9);
        set("unit models", "lit_figures", 5);
        add("map buildings", "lit_draws", 1);
        add_named("unit models", &format!("refused: {}", "a reason"), 2);
        add_named("unit models", "refused: a reason", 1);
        let frame = take();
        let section = |name: &str| frame.iter().find(|(given, _)| *given == name).map(|(_, counts)| counts.clone());
        assert_eq!(section("unit models"), Some(vec![("lit_draws", 7), ("lit_figures", 5), ("refused: a reason", 3)]));
        assert_eq!(section("map buildings"), Some(vec![("lit_draws", 1)]));
        assert!(take().is_empty());
    }
}
