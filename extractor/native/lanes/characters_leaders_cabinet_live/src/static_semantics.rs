use crate::extractor::{advisor_bucket, CabinetBucket};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticCountryLeaderRole<'a> {
    pub character_token: &'a str,
    pub ideology: &'a str,
    pub traits: Vec<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticAdvisorRole<'a> {
    pub character_token: &'a str,
    pub idea_token: &'a str,
    pub slot: Option<&'a str>,
    pub ledger: Option<&'a str>,
    pub traits: Vec<&'a str>,
    pub has_allowed_script: bool,
    pub has_available_script: bool,
    pub has_visible_script: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvisorActiveStatusSupport {
    AppointedFromActiveIdeaIntersection,
    RejectedStaticDefinitionOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountryLeaderIdeologyScope {
    StaticSubIdeology,
    RuntimePartyGroup,
}

pub fn country_leader_ideology_scope(
    _role: &StaticCountryLeaderRole<'_>,
) -> CountryLeaderIdeologyScope {
    CountryLeaderIdeologyScope::StaticSubIdeology
}

pub fn active_ruler_party_scope() -> CountryLeaderIdeologyScope {
    CountryLeaderIdeologyScope::RuntimePartyGroup
}

pub fn static_advisor_bucket(role: &StaticAdvisorRole<'_>) -> CabinetBucket {
    advisor_bucket(role.slot, role.ledger, role.idea_token)
}

pub fn advisor_active_status_support(role: &StaticAdvisorRole<'_>) -> AdvisorActiveStatusSupport {
    if role.has_allowed_script || role.has_available_script || role.has_visible_script {
        AdvisorActiveStatusSupport::RejectedStaticDefinitionOnly
    } else {
        AdvisorActiveStatusSupport::RejectedStaticDefinitionOnly
    }
}

pub fn appointed_status_support() -> AdvisorActiveStatusSupport {
    AdvisorActiveStatusSupport::AppointedFromActiveIdeaIntersection
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn country_leader_static_role_carries_subideology_and_traits() {
        let role = StaticCountryLeaderRole {
            character_token: "ENG_winston_churchill",
            ideology: "conservatism",
            traits: vec!["british_bulldog"],
        };

        assert_eq!(
            country_leader_ideology_scope(&role),
            CountryLeaderIdeologyScope::StaticSubIdeology
        );
        assert_eq!(
            active_ruler_party_scope(),
            CountryLeaderIdeologyScope::RuntimePartyGroup
        );
        assert_eq!(role.ideology, "conservatism");
        assert_eq!(role.traits, vec!["british_bulldog"]);
    }

    #[test]
    fn advisor_static_role_carries_bucket_and_traits() {
        let role = StaticAdvisorRole {
            character_token: "ENG_leslie_hore_belisha",
            idea_token: "ENG_leslie_hore_belisha",
            slot: Some("political_advisor"),
            ledger: None,
            traits: vec!["compassionate_gentleman"],
            has_allowed_script: true,
            has_available_script: false,
            has_visible_script: false,
        };

        assert_eq!(
            static_advisor_bucket(&role),
            CabinetBucket::PoliticalAdvisor
        );
        assert_eq!(role.traits, vec!["compassionate_gentleman"]);
    }

    #[test]
    fn static_availability_scripts_are_not_live_active_status() {
        let role = StaticAdvisorRole {
            character_token: "ENG_philip_kerr",
            idea_token: "ENG_philip_kerr",
            slot: Some("political_advisor"),
            ledger: None,
            traits: vec![],
            has_allowed_script: true,
            has_available_script: true,
            has_visible_script: false,
        };

        assert_eq!(
            advisor_active_status_support(&role),
            AdvisorActiveStatusSupport::RejectedStaticDefinitionOnly
        );
        assert_eq!(
            appointed_status_support(),
            AdvisorActiveStatusSupport::AppointedFromActiveIdeaIntersection
        );
    }
}
