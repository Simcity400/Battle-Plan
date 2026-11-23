use std::sync::OnceLock;

use content_reader_script::{scope::ScopeKind, trigger_keys::Reader};
use engine_executable::executable_tables::{self, Tables};

pub fn enter() {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        let country = ScopeKind::Country.bit();
        content_reader_script::test_game::executable()
            .trigger("add_to_temp_variable", 0, Reader::Body, 0)
            .trigger("any_owned_state", country, Reader::Body, 0)
            .trigger("check_variable", 0, Reader::Body, 0)
            .trigger("compare", 0, Reader::Assign, 0)
            .trigger("divide_temp_variable", 0, Reader::Body, 0)
            .trigger("has_stability", country, Reader::Comparison, 0)
            .trigger("has_war_support", country, Reader::Comparison, 0)
            .trigger("is_in_tech_sharing_group", country, Reader::Assign, 0)
            .trigger("modulo_temp_variable", 0, Reader::Body, 0)
            .trigger("set_temp_variable", 0, Reader::Body, 0)
            .trigger("state", ScopeKind::State.bit(), Reader::Assign, 0)
            .trigger("tag", country, Reader::Assign, 0)
            .trigger("value", 0, Reader::Assign, 0)
            .trigger("var", 0, Reader::Assign, 0)
            .build()
    });
    executable_tables::enter(Some(*tables));
}
