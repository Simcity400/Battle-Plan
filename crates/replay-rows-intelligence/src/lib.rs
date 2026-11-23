use engine_allocation as allocation;
use replay_rows::codec::{Decoder, invalid_static as invalid, put_varint};
use replay_rows::rows::*;
use replay_rows::typed_codec;
use replay_rows::value_struct;
use std::io;

mod characters_intelligence;
pub use characters_intelligence::*;

pub use schema::*;
mod schema;

#[doc(hidden)]
pub mod ops {
    use super::*;
    replay_rows::row_ops! {
        Characters => CharacterRow,
        CountryLeaderRoles => CountryLeaderRoleRow,
        AdvisorRoles => AdvisorRoleRow,
        UnitLeaders => UnitLeaderRow,
        UnitLeaderTraits => UnitLeaderTraitRow,
        UnitLeaderTraitExperience => UnitLeaderTraitExperienceRow,
        ActiveAbilities => ActiveAbilityRow,
        IntelligenceAgencies => IntelligenceAgencyRow,
        OperativeMemberships => OperativeMembershipRow,
        Cryptology => CryptologyRow,
        IntelNetworks => IntelNetworkRow,
        Operations => OperationRow,
        OperationAssignments => OperationAssignmentRow,
        OperationTokens => OperationTokensRow,
        UnitIntel => UnitIntelRow,
        Scientists => ScientistRow,
        ScientistSkills => ScientistSkillRow,
        CountryIntel => CountryIntelRow,
        HiredAdvisors => HiredAdvisorRow,
        CountryCharacters => CountryCharacterRow,
        AdvisorTokens => AdvisorTokenRow,
        FactionAdvisors => FactionAdvisorRow,
        CountryRetiredCharacters => CountryCharacterRow,
        CharacterRoleTraits => CharacterRoleTraitRow,
        CharacterOperatives => CharacterOperativeRow,
        AvailableAdvisors => AvailableAdvisorRow,
        CapturedOperativeReferences => CapturedOperativeReferenceRow,
        AgencyUpgradeLevels => AgencyUpgradeLevelRow,
        OperationCompletions => OperationCompletionRow,
        OperativeMissions => OperativeMissionRow,
        StrategicOperatives => StrategicOperativeRow,
        AgencyOperationBuffers => AgencyOperationBuffersRow,
        ScientistDaily => ScientistDailyRow,
        IntelNetworkSummaries => IntelNetworkSummaryRow,
        PropagandaEfforts => PropagandaEffortRow,
        IntelContributions => IntelContributionRow,
        IntelSources => IntelSourceRow,
        IntelNetworkAuxiliaries => IntelNetworkAuxiliaryRow,
    }
}
