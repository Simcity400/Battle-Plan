
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

}
