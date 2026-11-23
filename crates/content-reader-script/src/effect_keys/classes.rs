use engine_executable::executable_tables::{RuntimeTable, view};

pub(super) static CLASS_IDENTITIES: RuntimeTable<&'static [(u32, &'static str, u64)]> =
    RuntimeTable::new(|| {
        view(
            |tables| &tables.registries,
            |table| &table.effect_identities[..],
        )
    });
