//! Built-in game variables compute their values on demand before stored keys
//! are considered. Names retain exact byte identity, without operand suffixes.
//! Purchase contracts and factions have no built-in variable table.

use engine_executable::executable_tables::RuntimeTable;

/// Built-ins of the global variable store.
pub static GLOBAL: RuntimeTable<&'static [&'static str]> =
    RuntimeTable::new(|| &engine_executable::executable_tables::get().variables.global);

/// Built-ins of a country's variable store.
pub static COUNTRY: RuntimeTable<&'static [&'static str]> = RuntimeTable::new(|| {
    &engine_executable::executable_tables::get()
        .variables
        .country
});

/// Built-ins of a state's variable store.
pub static STATE: RuntimeTable<&'static [&'static str]> =
    RuntimeTable::new(|| &engine_executable::executable_tables::get().variables.state);

/// Built-ins of a character's variable store, documented as `unit_leader`.
pub static CHARACTER: RuntimeTable<&'static [&'static str]> = RuntimeTable::new(|| {
    &engine_executable::executable_tables::get()
        .variables
        .unit_leader
});

/// Whether `name` is a built-in of `table` (sorted by bytes).
pub fn contains(table: &[&str], name: &[u8]) -> bool {
    table
        .binary_search_by(|entry| entry.as_bytes().cmp(name))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_preserves_case_and_trailing_underscore() {
        let table = ["alpha", "beta_"];
        assert!(contains(&table, b"alpha"));
        assert!(contains(&table, b"beta_"));
        assert!(!contains(&table, b"Alpha"));
        assert!(!contains(&table, b"beta"));
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_tables_are_sorted_and_unique() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        for table in [&GLOBAL[..], &COUNTRY[..], &STATE[..], &CHARACTER[..]] {
            assert!(
                table
                    .windows(2)
                    .all(|pair| pair[0].as_bytes() < pair[1].as_bytes())
            );
        }
    }
}
