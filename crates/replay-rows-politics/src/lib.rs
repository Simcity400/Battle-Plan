use engine_allocation as allocation;
use replay_rows::codec::{Decoder, invalid_static as invalid, put_varint};
use replay_rows::rows::*;
use replay_rows::typed_codec;
use replay_rows::{opaque_value, row_codec, value_struct};
use replay_rows_presentation::*;
use replay_rows_world::*;
use std::io;

mod base;
pub use base::*;
mod politics_research;
pub use politics_research::*;
mod politics_rows;
pub use politics_rows::*;

pub use schema::*;
mod schema;

#[doc(hidden)]
pub mod ops {
    use super::*;
    replay_rows::row_ops! {
        Focus => FocusRow,
        CompletedFocuses => CompletedFocusRow,
        ResearchTechnologies => ResearchTechnologyRow,
        ResearchSlots => ResearchSlotRow,
        TechnologyDefinitions => TechnologyDefinitionRow,
        Politics => PoliticsRow,
        PoliticalParties => PoliticalPartyRow,
        ActiveIdeas => ActiveIdeaRow,
        DecisionCounts => DecisionCountsRow,
        Decisions => DecisionRow,
        DynamicModifiers => DynamicModifierRow,
        TechSharingGroups => TechSharingGroupRow,
        TechSharingMembers => TechSharingMemberRow,
        PowerBalances => PowerBalanceRow,
        PowerBalanceMembers => PowerBalanceMemberRow,
        PowerBalanceRanges => PowerBalanceRangeRow,
        PowerBalanceOverrides => PowerBalanceOverrideRow,
        CountryExperience => CountryExperienceRow,
        OccupationSettings => OccupationSettingsRow,
        StateOccupationLaws => StateOccupationLawRow,
        StateGarrisons => StateGarrisonRow,
        StateAssignedManpower => StateAssignedManpowerRow,
        CountryOccupationReserveManpower => CountryOccupationReserveManpowerRow,
        StateResistance => StateResistanceRow,
        CountryCollaboration => CountryCollaborationRow,
        Capitulation => CapitulationRow,
        Subjects => SubjectRow,
        Autonomy => AutonomyRow,
        ScriptFlags => ScriptFlagRow,
        ScriptVariables => ScriptVariablesRow,
        PeaceConferences => PeaceConferenceRow,
        LexicalTokens => LexicalTokenRow,
        WorldTension => WorldTensionRow,
        ThreatSources => ThreatSourceRow,
        ThreatCountryAggregates => ThreatCountryAggregateRow,
        IncomingDiplomaticActions => IncomingDiplomaticActionRow,
        RelationOpinions => RelationOpinionRow,
        RelationTuples => RelationTupleRow,
        StateFlags => StateFlagRow,
        ResearchInputs => ResearchInputRow,
        ResearchAppliedBonuses => ResearchAppliedBonusRow,
        ResearchModifiers => ResearchModifierRow,
        RelationLinks => RelationLinkRow,
        SliceReads => SliceReadRow,
        ModifierDefinitions => ModifierDefinitionRow,
        CountryStabilityInputs => CountryStabilityInputRow,
        CountryStabilityLines => CountryStabilityLineRow,
        StateDynamicModifiers => StateDynamicModifierRow,
        CountryWars => CountryWarRow,
        BorderConflicts => BorderConflictRow,
        CountryCommandPower => CountryCommandPowerRow,
        StateCores => StateCoreRow,
        PartyLeaders => PartyLeaderRow,
        CountryCivilWars => CountryCivilWarRow,
        GlobalSavedEventTargets => GlobalSavedEventTargetRow,
        OwnedWargoals => OwnedWargoalRow,
        TimedIdeas => TimedIdeaRow,
        DecisionRandomItems => DecisionRandomItemRow,
        CountryExperienceGains => CountryExperienceGainRow,
    }
}
