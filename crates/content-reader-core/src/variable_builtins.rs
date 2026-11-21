
use engine_executable::executable_tables::{RuntimeTable, view};

pub static GLOBAL: RuntimeTable<&'static [&'static str]> =
    RuntimeTable::new(|| view(|tables| &tables.variables, |table| &table.global[..]));

pub static COUNTRY: RuntimeTable<&'static [&'static str]> =
    RuntimeTable::new(|| view(|tables| &tables.variables, |table| &table.country[..]));

pub static STATE: RuntimeTable<&'static [&'static str]> =
    RuntimeTable::new(|| view(|tables| &tables.variables, |table| &table.state[..]));

pub static CHARACTER: RuntimeTable<&'static [&'static str]> =
    RuntimeTable::new(|| view(|tables| &tables.variables, |table| &table.unit_leader[..]));

pub static ORGANISATION: RuntimeTable<&'static [&'static str]> = RuntimeTable::new(|| {
    view(
        |tables| &tables.variables,
        |table| &table.industrial_organisation[..],
    )
});

pub static PROJECT: RuntimeTable<&'static [&'static str]> =
    RuntimeTable::new(|| view(|tables| &tables.variables, |table| &table.project[..]));

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
        for table in [
            &GLOBAL[..],
            &COUNTRY[..],
            &STATE[..],
            &CHARACTER[..],
            &ORGANISATION[..],
            &PROJECT[..],
        ] {
            assert!(
                table
                    .windows(2)
                    .all(|pair| pair[0].as_bytes() < pair[1].as_bytes())
            );
        }
    }
}
