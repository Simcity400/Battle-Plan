use engine::executable_tables::RuntimeTable;

pub(super) static CLASS_IDENTITIES: RuntimeTable<&'static [(u32, &'static str, u64)]> =
    RuntimeTable::new(|| {
        &engine::executable_tables::get()
            .registries
            .effect_identities
    });
