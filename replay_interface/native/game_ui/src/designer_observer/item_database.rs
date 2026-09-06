use crate::ability_visibility::{
    ability_visibility, parse_allowed_body, AllowedBody, AllowedScope, AllowedWorldFacts, Operand,
    Verdict,
};
use crate::paradox_text::Value;
use replay_core::design_catalog::DesignU32Observation;
use std::cell::Cell;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemPoolLimit(AllowedBody);

impl ItemPoolLimit {
    pub fn parse(value: &Value) -> Result<Self, String> {
        parse_allowed_body(value).map(Self)
            .map_err(|reason| format!("XUI13 item pool limit: {reason}"))
    }

    pub fn matches(&self, runtime: &DesignU32Observation) -> Result<bool, String> {
        let facts = RuntimeFacts { runtime, unexpected: Cell::new(None) };
        let result = self.evaluate(&facts);
        if let Some(field) = facts.unexpected.get() {
            return Err(format!("XUI13 item pool default scope requested {field}"));
        }
        result
    }

    fn evaluate(&self, facts: &dyn AllowedWorldFacts) -> Result<bool, String> {
        // 1409f1540 constructs a fresh 140528800 scope immediately before the limit call.
        let scope = AllowedScope { country_tag: None, caster: None, formation_selected: false };
        match ability_visibility("", false, &self.0, &scope, facts) {
            Verdict::Visible => Ok(true),
            Verdict::Hidden => Ok(false),
            Verdict::Refused(reason) => Err(format!("XUI13 item pool limit: {reason}")),
        }
    }
}

struct RuntimeFacts<'a> {
    runtime: &'a DesignU32Observation,
    unexpected: Cell<Option<&'static str>>,
}

impl AllowedWorldFacts for RuntimeFacts<'_> {
    fn dlc_mask(&self) -> Operand<u32> {
        match self.runtime {
            DesignU32Observation::Measured { raw_u32 } => Operand::Recorded(*raw_u32),
            DesignU32Observation::Unavailable { .. } => Operand::Unrecorded("A27 runtime_dlc_mask"),
        }
    }

    fn completed_focus(&self, _: &str, _: &str) -> Operand<bool> {
        self.unexpected.set(Some("country completed focus"));
        Operand::Unrecorded("item pool scope has no country")
    }

    fn country_flag_value(&self, _: &str, _: &str) -> Operand<Option<i64>> {
        self.unexpected.set(Some("country flag"));
        Operand::Unrecorded("item pool scope has no country")
    }

    fn trait_unlocks_ability(&self, _: &str, _: &str) -> Option<bool> {
        self.unexpected.set(Some("trait ability"));
        None
    }

    fn ability_exists(&self, _: &str) -> bool {
        self.unexpected.set(Some("ability catalog"));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradox_text::parse_str;
    use replay_core::design_catalog::DesignFieldRefusal;

    fn limit(text: &str) -> ItemPoolLimit {
        let doc = parse_str(&format!("limit = {{ {text} }}"));
        ItemPoolLimit::parse(doc.root.first_value_ci("limit").unwrap()).unwrap()
    }

    struct NoWorldFacts;
    impl AllowedWorldFacts for NoWorldFacts {
        fn dlc_mask(&self) -> Operand<u32> { panic!("unreached runtime fact") }
        fn completed_focus(&self, _: &str, _: &str) -> Operand<bool> { panic!("creator country was installed") }
        fn country_flag_value(&self, _: &str, _: &str) -> Operand<Option<i64>> { panic!("creator country was installed") }
        fn trait_unlocks_ability(&self, _: &str, _: &str) -> Option<bool> { panic!("ability clause was enabled") }
        fn ability_exists(&self, _: &str) -> bool { panic!("ability clause was enabled") }
    }

    #[test]
    fn default_scope_never_reads_creator_or_ability_facts() {
        for text in ["has_country_flag = creator_flag", "has_completed_focus = creator_focus",
            "has_ability = force_attack", "OWNER = { has_country_flag = creator_flag }",
            "is_leading_army_group = no", "is_border_war = no"] {
            assert_eq!(limit(text).evaluate(&NoWorldFacts), Ok(false), "{text}");
        }
        assert_eq!(limit("").evaluate(&NoWorldFacts), Ok(true));
        assert_eq!(limit("NOT = { has_country_flag = creator_flag }").evaluate(&NoWorldFacts), Ok(true));
    }

    #[test]
    fn runtime_feature_mask_drives_dlc_trigger_without_content_name_substitution() {
        let query = limit(r#"has_dlc = "Man the Guns""#);
        assert_eq!(query.matches(&DesignU32Observation::Measured { raw_u32: 0x10 }), Ok(true));
        assert_eq!(query.matches(&DesignU32Observation::Measured { raw_u32: 0x80 }), Ok(false));
        let unknown = DesignU32Observation::Unavailable { reason: DesignFieldRefusal {
            kind: "unreadable".into(), field: "runtime_dlc_mask".into(),
        }};
        assert!(query.matches(&unknown).unwrap_err().contains("DLC"));
        assert_eq!(limit(r#"has_dlc = "man the guns""#).matches(&unknown), Ok(false));
        assert_eq!(limit("").matches(&unknown), Ok(true));
    }

    #[test]
    fn short_circuit_and_correlated_unknowns_reuse_existing_trigger_law() {
        assert_eq!(limit(r#"has_country_flag = no_scope has_dlc = "Man the Guns""#)
            .evaluate(&NoWorldFacts), Ok(false));
        let unknown = DesignU32Observation::Unavailable { reason: DesignFieldRefusal {
            kind: "unreadable".into(), field: "runtime_dlc_mask".into(),
        }};
        assert_eq!(limit(r#"OR = { has_dlc = "Man the Guns" NOT = { has_dlc = "Man the Guns" } }"#)
            .matches(&unknown), Ok(true));
        assert_eq!(limit(r#"has_dlc = "Man the Guns" NOT = { has_dlc = "Man the Guns" }"#)
            .matches(&unknown), Ok(false));
    }

    #[test]
    fn unsupported_authored_trigger_is_named_instead_of_admitted() {
        let doc = parse_str("limit = { unproven_icon_trigger = yes }");
        assert!(ItemPoolLimit::parse(doc.root.first_value_ci("limit").unwrap())
            .unwrap_err().contains("unproven_icon_trigger"));
    }
}
