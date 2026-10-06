//! One class-to-domain dispatch. Owners add only rows in their domain groups.
use super::*;

#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    pub(super) fn effect_class(&self, item: &Item) -> Option<installed_content::effect_keys::NativeClass> {
        if let Some(class) = item
            .keyword()
            .and_then(|key| installed_content::effect_keys::native_class(&key.folded()))
        {
            return Some(class);
        }
        if self.is_context(item) {
            return Some(installed_content::effect_keys::NativeClass {
                name: "CContextEffect",
                vtable: 0x14299F6F8,
            }); // 14299F6F8, ReadEffect@1405345D0.
        }
        let name = installed_content::parser::c_string(item.key.as_deref().unwrap_or_default());
        self.context
            .content
            .key_names
            .filter(|names| names.scripted_effects.contains(&name.to_ascii_lowercase()))
            .map(|_| installed_content::effect_keys::NativeClass {
                name: "CScriptedEffect",
                vtable: 0x14299F7C8,
            }) // 14299F7C8, ReadEffect@1405345D0.
    }

    pub(super) fn leaf(&mut self, item: &Item, scopes: Scopes) -> Option<Result<String, String>> {
        let class = self.effect_class(item)?;
        Some(match class.vtable {
            // scopes — I
            0x14299F6F8 /* CContextEffect */ => self
                .context_header(item, scopes)
                .map(|selected| selected.map(|(_, header)| header).unwrap_or_default()),
            0x1427576E8 /* CEveryStateEffect */ => lists::own(), // Own 140176EE0, build 140312880.
            0x1427B6A68 /* CEveryCollectionElementEffect */ => lists::own(), // Build 1404C3640.

            0x142767790 /* CAlliedCountryEffectBase<> */ => lists::own(), // Build 14036AA80.
            0x1427678A0 /* CAlliedCountryEffectBase<> */ => lists::own(), // Build 1402F4BA0.
            0x14272B088 /* CCharacterListTooltipEffect */ => lists::own(), // Build 1402F53E0.
            0x1427971A8 /* CEveryArmyEffect */ => lists::own(), // Build 1402F4200.
            0x142753F50 /* CEveryArmyLeaderEffect */ => lists::own(), // Build 14036AA80.
            0x14272AE58 /* CEveryCharacterEffect */ => lists::own(), // Build 1402F4200.
            0x142738188 /* CEveryControlledStateEffect */ => lists::own(), // Build 140312880.
            0x142757E68 /* CEveryCoreStateEffect */ => lists::own(), // Build 140312880.
            0x142756730 /* CEveryCountryEffect */ => lists::own(), // Build 14036AA80.
            0x142758AE8 /* CEveryCountryWithOriginalTag */ => lists::own(), // Build 14036AA80.
            0x142758688 /* CEveryEnemyCountryEffect */ => lists::own(), // Build 14036AA80.
            0x1427B2F58 /* CEveryFactionMember */ => lists::own(), // Build 14036AA80.
            0x142753E68 /* CEveryNavyLeaderEffect */ => lists::own(), // Build 14036AA80.
            0x142758248 /* CEveryNeighborCountryEffect */ => lists::own(), // Build 14036AA80.
            0x142738278 /* CEveryNeighborStateEffect */ => lists::own(), // Build 140312880.
            0x1427588A8 /* CEveryOccupiedCountryEffect */ => lists::own(), // Build 14036AA80.
            0x142754128 /* CEveryOperativeLeaderEffect */ => lists::own(), // Build 14036AA80.
            0x142756A60 /* CEveryOtherCountryEffect */ => lists::own(), // Build 14036AA80.
            0x142757D78 /* CEveryOwnedStateEffect */ => lists::own(), // Build 140312880.
            0x142756840 /* CEveryPossibleCountryEffect */ => lists::own(), // Build 14036AA80.
            0x1427A9500 /* CEveryScientistEffect@?A0x9898cfbe@NProject */ => lists::own(), // Build 1402F4200.
            0x1427975D8 /* CEveryStateArmyEffect */ => lists::own(), // Build 1402F4200.
            0x142758578 /* CEverySubjectCountryEffect */ => lists::own(), // Build 14036AA80.
            0x142753BA0 /* CEveryUnitLeaderEffect */ => lists::own(), // Build 14036AA80.
            0x142754210 /* CGlobalEveryArmyLeaderEffect */ => lists::own(), // Build 14036AA80.
            0x14272B230 /* CPartyLeaderTarget */ => lists::own(), // Build 1402F4BA0.
            0x1427A96D0 /* CRandomActiveScientistEffect@?A0x9898cfbe@NProject */ => lists::own(), // Build 1402F4BA0.
            0x1427970B8 /* CRandomArmyEffect */ => lists::own(), // Build 1402F4BA0.
            0x142753C88 /* CRandomArmyLeaderEffect */ => lists::own(), // Build 1402F4BA0.
            0x14272AD68 /* CRandomCharacterEffect */ => lists::own(), // Build 1402F4BA0.
            0x142757AA8 /* CRandomControlledStateEffect */ => lists::own(), // Build 1402F4BA0.
            0x142758048 /* CRandomCoreStateEffect */ => lists::own(), // Build 1402F4BA0.
            0x142756620 /* CRandomCountryEffect */ => lists::own(), // Build 1402F4BA0.
            0x1427589B8 /* CRandomCountryWithOriginalTag */ => lists::own(), // Build 1402F4BA0.
            0x142758358 /* CRandomEnemyCountryEffect */ => lists::own(), // Build 1402F4BA0.
            0x142753D78 /* CRandomNavyLeaderEffect */ => lists::own(), // Build 1402F4BA0.
            0x142758138 /* CRandomNeighborCountryEffect */ => lists::own(), // Build 1402F4BA0.
            0x1427575F8 /* CRandomNeighborStateEffect */ => lists::own(), // Build 1402F4BA0.
            0x142758798 /* CRandomOccupiedCountryEffect */ => lists::own(), // Build 1402F4BA0.
            0x142754038 /* CRandomOperativeLeaderEffect */ => lists::own(), // Build 1402F4BA0.
            0x142756950 /* CRandomOtherCountryEffect */ => lists::own(), // Build 1402F4BA0.
            0x142757C88 /* CRandomOwnedControlledStateEffect */ => lists::own(), // Build 1402F4BA0.
            0x1427578C8 /* CRandomOwnedStateEffect */ => lists::own(), // Build 1402F4BA0.
            0x1427A9410 /* CRandomScientistEffect@?A0x9898cfbe@NProject */ => lists::own(), // Build 1402F4BA0.
            0x142760010 /* CRandomScopeInArray */ => lists::own(), // Build 14036B940.
            0x1427974E8 /* CRandomStateArmyEffect */ => lists::own(), // Build 1402F4BA0.
            0x142757418 /* CRandomStateEffect */ => lists::own(), // Build 1402F4BA0.
            0x142758468 /* CRandomSubjectCountryEffect */ => lists::own(), // Build 1402F4BA0.
            0x142753AB0 /* CRandomUnitLeaderEffect */ => lists::own(), // Build 1402F4BA0.
            0x142750FC8 /* CSetPoliticsEffect */ => self.set_politics(item, scopes), // Build 14036C090.
            0x14272F680 /* NIndustrialOrganisation::CEveryIndustrialOrgEffect */ => lists::own(), // Build 1402F4200.
            0x14272F590 /* NIndustrialOrganisation::CRandomIndustrialOrgEffect */ => lists::own(), // Build 1402F4BA0.

            // variables — I
            // Empty vf7 never executes persistent or temporary setters.
            0x14275F568 /* CClearVariableEffect */
            | 0x142767D30 /* CAddToArrayEffectImp<false> */
            | 0x142767E00 /* CAddToArrayEffectImp<true> */
            | 0x14275ECF0 /* CClampVariableEffect */
            | 0x142768070 /* CClearArrayEffectImp<false> */
            | 0x142768140 /* CClearArrayEffectImp<true> */
            | 0x1427683B0 /* CFindHighestLowestInArrayEffect<true> */
            | 0x142768480 /* CFindHighestLowestInArrayEffect<false> */
            | 0x14275F2F8 /* CRandomizeVariableEffect<true> */
            | 0x14275F3C8 /* CRandomizeVariableEffect<false> */
            | 0x142767ED0 /* CRemoveFromArrayEffectImp<false> */
            | 0x142767FA0 /* CRemoveFromArrayEffectImp<true> */
            | 0x142768210 /* CResizeArrayEffectImp<false> */
            | 0x1427682E0 /* CResizeArrayEffectImp<true> */
            | 0x14275F0A8 /* CRoundTempVariableEffect */
            | 0x14275EF90 /* CRoundVariableEffect */
            | 0x1427A21D8 /* CVariableEffectBuilder<GetSortedScoredCountries> */
            | 0x1427A2390 /* CVariableEffectBuilder<GetHighestScoredCountry> */
            | 0x1427A2530 /* CVariableEffectBuilder<Modulo> */
            | 0x1427A26D0 /* CVariableEffectBuilder<SetVariableToRandomRange> */ => variables::empty(), // 140176EE0/1402E9B40.
            0x14275F498 /* CSetVariableEffect */ => self.variable_effect(item, scopes, "set_variable", false),
            0x14275F638 /* CAddToVariableEffect */ => self.variable_effect(item, scopes, "add_to_variable", false),
            0x14275F708 /* CSubtractFromVariableEffect */ => self.variable_effect(item, scopes, "subtract_from_variable", false),
            0x14275F7D8 /* CMultiplyVariableEffect */ => self.variable_effect(item, scopes, "multiply_variable", false),
            0x14275F8A8 /* CDivideVariableEffect */ => self.variable_effect(item, scopes, "divide_variable", false), // 14039A7D0.
            0x14275F978 /* CSetTempVariableEffect */ => self.variable_effect(item, scopes, "set_temp_variable", true),
            0x14275FA48 /* CAddToTempVariableEffect */ => self.variable_effect(item, scopes, "add_to_temp_variable", true),
            0x14275FB18 /* CSubtractFromTempVariableEffect */ => self.variable_effect(item, scopes, "subtract_from_temp_variable", true),
            0x14275FBE8 /* CMultiplyTempVariableEffect */ => self.variable_effect(item, scopes, "multiply_temp_variable", true),
            0x14275FCB8 /* CDivideTempVariableEffect */ => self.variable_effect(item, scopes, "divide_temp_variable", true), // 140379020.
            0x14275F178 /* CClampTempVariableEffect */ => self.temp_variable_builder(item, scopes, "clamp_temp_variable"), // 14037B520.
            0x1427A2600 /* CVariableEffectBuilder<ModuloTemp> */ => self.temp_variable_builder(item, scopes, "modulo_temp_variable"), // 14047BF30.
            0x1427A27A0 /* CVariableEffectBuilder<SetTempVariableToRandomRange> */ => self.temp_variable_builder(item, scopes, "set_temp_variable_to_random"), // 14047BF80.
            0x1427A2940 /* CVariableEffectBuilder<GetSupplyVehiclesTemp> */ => self.temp_variable_builder(item, scopes, "get_supply_vehicles_temp"), // 14047BEE0.
            0x1427A2460 /* CVariableEffectBuilder<GetHighestScoredCountryTemp> */ => self.scored_countries(item, scopes, false), // 14047BE40.
            0x1427A22A8 /* CVariableEffectBuilder<GetSortedScoredCountriesTemp> */ => self.scored_countries(item, scopes, true), // 14047BE90.
            0x14275A188 /* CSaveEventTargetAsEffect */ => self.save_event_target(item, scopes),
            0x14275A258 /* CSaveGlobalEventTargetAsEffect */ => self.save_global_event_target(item, scopes), // 14038D6B0.

            // lists — I
            0x1427BE590 /* CIfEffect */ | 0x14299FCB8 /* CHiddenEffect */ | 0x14299FD88 /* CEffectTooltipEffect */ => lists::own(),

            0x14299F7C8 /* CScriptedEffect */ | 0x14275FF40 /* CForEachScopeEffect */ | 0x142761860 /* CMetaEffect */ | 0x1429A00F0 /* CRandomEffect */ => lists::own(),
            0x1429A0260 /* CRandomListEffect */ => self.random_list(item, scopes), // 141383AB0.
            0x14299FFF8 /* CWithTooltipOverrideEffect */ => self.override_own(item, scopes), // 141384190.

            // modifiers — I
            0x142751C20 /* CAddTimedIdeaEffect */ => self.timed_idea(item, scopes),

            // numeric — I
            0x142751FF8 /* CAddPoliticalPowerEffect */ => {
                self.numeric_value(item, scopes, b"POLITICS_ADD_POLITICAL_POWER")
            } // 140376050
            0x142759DB8 /* CAddArmyExperienceEffect */ => {
                self.numeric_value(item, scopes, b"EFFECT_ADD_ARMY_EXPERIENCE")
            } // 140370FF0
            0x142758BF8 /* CAddStabilityEffect */ => self.numeric_value(item, scopes, b"EFFECT_ADD_STABILITY"), // 140377DF0
            0x142758DC8 /* CAddWarSupportEffect */ => self.numeric_value(item, scopes, b"EFFECT_ADD_WAR_SUPPORT"), // 1403797B0

            // characters — F
            0x14275BDD0 /* CWhitePeaceEffect */ => self.char_peace(item, scopes, false), // 14039AA90
            0x14275BF80 /* CStartPeaceConferenceEffect */ => self.char_peace(item, scopes, true), // 140396550
            0x142727C08 /* CAddTraitEffect */ => self.char_trait(item, scopes, false),
            0x142727EA8 /* CRemoveTraitEffect */ => self.char_trait(item, scopes, true),
            0x1427507B0 /* CAddCountryLeaderTraitEffect */ => self.char_country_trait(item, scopes, false),
            0x142750A38 /* CRemoveCountryLeaderTraitEffect */ => self.char_country_trait(item, scopes, true),
            0x142728990 /* CAddUnitLeaderTraitEffect */ => self.char_unit_trait(item, scopes, false, false),
            0x142728D08 /* CRemoveUnitLeaderTraitEffect */ => self.char_unit_trait(item, scopes, true, false),
            0x1427518D0 /* CSwapRulerTraitsEffect */ => self.char_swap_traits(item, scopes, false),
            0x1427281F0 /* CSwapCountryLeaderTraitsEffect */ => self.char_swap_traits(item, scopes, true),
            0x142728A88 /* CAddTimedUnitLeaderTraitEffect */ => self.char_unit_trait(item, scopes, false, true),
            0x142727AB8 /* CAddRandomTraitEffect */ => self.char_random_trait(item, scopes),
            0x142727910 /* CAddMaxAssignableTrait */ => self.char_max_trait(item, scopes),
            0x142762870 /* CFreeRandomOperativeEfect */ => self.char_free_random_operative(item, scopes),
            0x142762290 /* CCaptureOperativeEffect */ => self.char_capture_operative(item, scopes),
            0x142762468 /* CKillOperativeEffect */ => self.char_kill_operative(item, scopes),
            0x142762588 /* CTurnOperativeLeaderEffect */ => self.char_turn_operative(item, scopes),
            0x142762B50 /* CForceOperativeLeaderIntoHiding */ => self.char_hide_operative(item, scopes),
            0x1427629E0 /* CHarmOperativeEffect */ => self.char_harm_operative(item, scopes),
            0x142729840 /* CUnitLeaderEventEffect */ => self.char_unit_leader_event(item, scopes),
            0x142729948 /* COperativeLeaderEventEffect */ => self.char_operative_leader_event(item, scopes),
            0x1427A9910 /* CAddScientistXpEffect@?A0x9898cfbe@NProject */ => self.char_add_scientist_xp(item, scopes),
            0x1427A97F8 /* CAddScientistLevelEffect@?A0x9898cfbe@NProject */ => self.char_add_scientist_level(item, scopes),
            0x1427A9B28 /* CInjureScientistEffect@?A0x9898cfbe@NProject */ => self.char_injure_scientist(item, scopes),
            0x1427555E0 /* CGenerateScientistRoleEffect */ => self.char_generate_scientist(item, scopes),
            0x1427A9A10 /* CAddScientistTraitEffect@?A0x9898cfbe@NProject */ => self.char_add_scientist_trait(item, scopes),
            0x142755478 /* CRemoveScientistRoleEffect */ => self.char_remove_scientist_role(item, scopes),
            0x1427552E0 /* CAddScientistRoleEffect */ => self.char_add_scientist_role(item, scopes),
            0x14274FED8 /* CLegacyCreateCountryLeaderEffect */ => self.char_create_country_leader(item, scopes),
            0x142754B88 /* CAddCountryLeaderRoleEffect */ => self.char_add_country_leader_role(item, scopes),
            0x142750168 /* CPromoteCharacterToCountryLeader */ => self.char_promote_character(item, scopes),
            0x142754A68 /* CRetireCharacterEffect */ => self.char_retire_character(item, scopes),
            0x1427502F8 /* CRetireCountryLeaderEffect */ => self.char_remove_party_leader(item, scopes, false, false),
            0x1427503F0 /* CKillCountryLeaderEffect */ => self.char_remove_party_leader(item, scopes, false, true),
            0x142754D08 /* CRemoveCountryLeaderRoleEffect */ => self.char_remove_country_leader_role(item, scopes),
            0x1427504E8 /* CRetireIdeologyLeaderEffect */ => self.char_remove_party_leader(item, scopes, true, false),
            0x1427505D8 /* CKillIdeologyLeaderEffect */ => self.char_remove_party_leader(item, scopes, true, true),
            0x142754FC0 /* CActivateAdvisorEffect */ => self.char_activate_advisor(item, scopes),
            0x1427550E8 /* CDeactivateAdvisorEffect */ => self.char_activate_advisor(item, scopes),
            0x142754E40 /* CAddAdvisorRoleEffect */ => self.char_add_advisor_role(item, scopes),
            0x1427551F0 /* CRemoveAdvisorRoleEffect */ => self.char_remove_advisor_role(item, scopes),
            0x142765418 /* CSetCanBeFiredToAdvisorRole */ => self.char_set_can_be_fired(item, scopes),
            0x142754868 /* CRecruitCharacterEffect */ => self.char_recruit_character(item, scopes),
            0x14275A090 /* CAddAceEffect */ => self.char_add_ace(item, scopes),
            0x1427B26E0 /* CSetFactionLeaderEffect */ => self.char_set_faction_leader(scopes),
            0x1427558F8 /* CAddCorpsRoleEffect */ => self.char_add_military_role(item, scopes, 1),
            0x142755A98 /* CAddFieldMarshallRoleEffect */ => self.char_add_military_role(item, scopes, 0),
            0x142755C38 /* CAddNavalRoleEffect */ => self.char_add_military_role(item, scopes, 2),
            0x1427545F8 /* CLegacyCreateCorpsCommanderEffect */ => self.char_create_legacy_military(item, scopes, 1),
            0x1427546C8 /* CLegacyCreateFieldMarshalEffect */ => self.char_create_legacy_military(item, scopes, 0),
            0x142754798 /* CLegacyCreateNavyLeaderEffect */ => self.char_create_legacy_military(item, scopes, 2),
            0x142755DF8 /* CCreateOperativeLeaderEffect */ => self.char_create_operative(item, scopes),
            0x142751440 /* CShowUnitLeadersTooltipEffect */ => self.char_show_unit_leaders(item, scopes),
            0x142728DF8 /* CPromoteLeaderEffect */ => self.char_change_rank(scopes, true),
            0x142728EE8 /* CDemoteLeaderEffect */ => self.char_change_rank(scopes, false),
            0x142728638 /* CRemoveUnitLeaderEffect */ => self.char_remove_unit_leader(item, scopes),
            0x142755D08 /* CRemoveMilitaryRoleEffect */ => self.char_remove_military_role(item, scopes),
            0x142797B10 /* CAddDivisionalCommanderXpEffect */ => self.char_divisional_xp(),
            0x1427978B0 /* CAddUnitMedalEffect */ => self.char_unit_medal(),
            0x1427979F0 /* CAddRandomValidUnitLeaderTraitEffect */ => self.char_random_trait_from_unit(item, scopes),
            0x1427314E8 /* NIndustrialOrganisation::CTraitTooltipEffect */ => self.char_mio_trait(),
            0x14272FBF0 /* NIndustrialOrganisation::CUnlockTraitEffect */ => self.char_mio_trait(),
            // politics — F
            0x1429A04C8 /* CLogScriptEffect */ => empty::describe(), // 140176EE0
            0x142759550 /* CAddPopularityEffect */ => self.politics_popularity(item, scopes), // 140376270
            0x142738560 /* CAddCoreOfEffect */ => self.politics_core(item, scopes, b"EFFECT_ADD_CORE_OF"), // 140313AB0
            0x14275CF10 /* CSetCosmeticTagEffect */ => self.politics_cosmetic(item, scopes), // 1403900D0
            0x14275C438 /* CSetPartyNameEffect */ => self.politics_party_name(item, scopes), // 140392160
            0x142764C70 /* CSetNationalityEffect */ => self.character_set_nationality(item, scopes), // 140391DB0
            0x14275A600 /* CCompleteNationalFocusEffect */ => self.politics_focus(item, scopes, 0), // 14037B7B0
            0x142738368 /* CAddClaimByEffect */ => self.politics_claim(item, scopes, b"EFFECT_ADD_CLAIM_BY"), // 140313620
            0x14275B978 /* CLoadOOBEffect */ => self.politics_oob(item, scopes, b"EFFECT_LOAD_OOB"), // 140384500
            0x142750DA8 /* CSetPopularitiesEffect */ => self.politics_popularities(item, scopes), // 1403929D0
            0x14274FD40 /* CSetCountryNationalFocusTreeEffect */ => self.politics_focus_tree(item, scopes), // 1403909F0
            0x14275AC00 /* CSetRuleEffect */ => self.politics_rules(item, scopes, false), // 140393260
            0x14275A4D0 /* CUnlockNationalFocusEffect */ => self.politics_focus(item, scopes, 1), // 14039A170
            0x14273C110 /* CAddToTechnologySharingGroupEffect */ => self.politics_sharing(item, scopes, false), // 1403214E0
            0x1427679B0 /* CReleaseNationEffect<> */ => self.politics_country(item, scopes, b"EFFECT_RELEASE_NATION", false), // 14036E4C0
            0x142728898 /* CRetireEffect */ => self.character_retire(scopes), // 1402EDA60
            0x142738658 /* CRemoveCoreOfEffect */ => self.politics_core(item, scopes, b"EFFECT_REMOVE_CORE_OF"), // 140316D10
            0x14275AB30 /* CAddNamedThreatEffect */ => self.politics_named_threat(item, scopes), // 1403748C0
            0x142759648 /* CChangeTagEffect */ => self.politics_change_tag(item, scopes), // 14037B2B0
            0x14275A718 /* CUncompleteNationalFocusEffect */ => self.politics_focus(item, scopes, 2), // 140398CF0
            0x142762F30 /* CUpgradeIntelligenceAgencyEffect */ => self.politics_upgrade_agency(item, scopes), // 14039A4E0
            0x142738468 /* CRemoveClaimByEffect */ => self.politics_claim(item, scopes, b"EFFECT_REMOVE_CLAIM_BY"), // 140316A20
            0x142761DC0 /* CAddOperationTokenEffect */ => self.politics_operation_token(item, scopes, false), // 140375370
            0x142762DF8 /* CCreateIntelligenceAgencyEffect */ => self.politics_create_agency(scopes), // 14037CBE0
            0x142765680 /* CReduceFocusCost */ => self.politics_focus_cost(item, scopes), // 140386F40
            0x142764E80 /* CEmbargoEffect */ => self.politics_country(item, scopes, b"EFFECT_SEND_EMBARGO", false), // 14037FD80
            0x142762C80 /* CAddNationalityToOperativeEffect */ => self.character_add_nationality(item, scopes), // 140374C60
            0x14275B3B8 /* CScopedSoundEffect */ => self.politics_sound(item, scopes, true), // 14038D800
            0x14273C000 /* CRemoveFromTechnologySharingGroupEffect */ => self.politics_sharing(item, scopes, true), // 140321A90
            0x14275B5A0 /* CScopedPlaySongEffect */ => self.politics_sound(item, scopes, true), // 14038D800
            0x142763AE8 /* CActivateShineOnFocus */ => self.politics_shine(), // 14036FE00; viewer debug off
            0x142763C38 /* CDeactivateShineOnFocus */ => self.politics_shine(), // 14037E730; viewer debug off
            0x142767A90 /* CReleaseNationEffect<> */ => self.politics_country(item, scopes, b"EFFECT_RELEASE_NATION", false), // 14036E4C0
            0x14275B2B8 /* CSoundEffect */ => self.politics_sound(item, scopes, false), // 140385DA0
            0x142761F98 /* CRemoveOperationTokenEffect */ => self.politics_operation_token(item, scopes, true), // 140389F00
            0x1427AEFE8 /* CAddRaidHistoryEntryEffect@?A0x8a0aef5d@NRaids */ => Err("CEventScope RAID_INSTANCE_REFERENCE (+0x88/+0x8C) is not represented by the event graph (140529460); Raids records the handles".into()), // 1404A2FB0
            0x14275AF28 /* CAddRelationRuleOverrideEffect */ => self.politics_relation_rule(item, scopes, false, None), // 1403769B0
            0x14275C158 /* CSetMajorEffect */ => self.politics_major(item, scopes), // 140391AB0
            0x14273C320 /* CModifyTechnologySharingBonusEffect */ => self.politics_sharing_bonus(item, scopes), // 140321840
            0x14275B4B0 /* CPlaySongEffect */ => self.politics_sound(item, scopes, false), // 140385DA0
            0x1427976C0 /* CAddHistoryEntryEffect */ => Err("CEventScope UNIT_REFERENCE (+0x70/+0x74) is not represented by the event graph (140529260); Divisions records the handles".into()), // 14045D2E0
            0x142761218 /* CRecallVolunteersFromEffect */ => self.politics_country(item, scopes, b"EFFECT_RECALL_VOLUNTEERS_FROM", true), // 140386960
            0x142759440 /* CHoldElectionEffect */ => self.politics_country(item, scopes, b"POLITICS_HOLD_ELECTION", false), // 140382C20
            0x1427526E0 /* CSetPoliticalPowerEffect */ => self.numeric_value(item, scopes, b"POLITICS_SET_POLITICAL_POWER"), // 1403927B0
            0x142731308 /* NIndustrialOrganisation::CSetOrganisationNameKeyEffect */ => self.politics_mio_name(item, scopes), // 1402FE660
            0x142764F78 /* CBreakEmbargoEffect */ => self.politics_country(item, scopes, b"EFFECT_BREAK_EMBARGO", false), // 140379FD0
            0x14275E5B0 /* CAddDaysRemoveDecisionEffect */ => self.decision_remove_days(item, scopes), // 140373020
            0x1427313F8 /* NIndustrialOrganisation::CSetOrganisationIconEffect */ => self.line(b"EFFECT_MIO_SET_ICON", &[], scopes, true), // 1402FE4E0
            0x1427AF710 /* CStateAndCountryEffect<> */ => self.politics_contested_owner(item, scopes, b"EFFECT_ADD_CONTESTED_OWNER"), // 1404A4F10
            0x1427BB648 /* NDoctrines::CUnlockCombatTacticEffect */ => self.politics_unlock_tactic(item, scopes), // 14051E260
            0x14272BB90 /* CReleaseFromCaptivityEffect */ => self.localised(b"EFFECT_RELEASE_FROM_CAPTIVITY", &[], scopes, false), // 1402F8610
            0x14272BA58 /* CReleaseCapturedGeneralsEffect */ => self.politics_release_generals(item, scopes), // 1402F8420
            0x14272B820 /* CCaptureGeneralEffect */ => self.character_capture_by(item, scopes), // 1402F8130
            0x14275B090 /* CRemoveRelationRuleOverrideEffect */ => self.politics_relation_rule(item, scopes, true, None), // 14038AFB0
            0x142761120 /* CRecallAttacheEffect */ => self.politics_country(item, scopes, b"EFFECT_RECALL_ATTACHE", true), // 140386680
            0x1427AF7E0 /* CStateAndCountryEffect<> */ => self.politics_contested_owner(item, scopes, b"EFFECT_REMOVE_CONTESTED_OWNER"), // 1404A4FC0
            0x14275BB58 /* CSetOobEffect */ => self.politics_oob(item, scopes, b"EFFECT_SET_OOB"), // 140391580
            0x14275ADB0 /* CSetPartyRuleEffect */ => self.politics_rules(item, scopes, true), // 140392590

            // technology — F
            0x1427BB520 /* NDoctrines::CAddMasteryEffect */ => self.mastery(item, scopes),
            0x14273BAE0 /* CAddTechBonusEffect */ => self.technology_bonus(item, scopes),
            0x14273C210 /* CSetTechnologyEffect */ => self.set_technology(item, scopes),
            0x14273BDE0 /* CAddResearchSlotEffect */ => self.research_slots(item, scopes, false),
            0x1427A9D78 /* CAddBreakthroughProgress@?A0x9898cfbe@NProject */ => self.breakthrough(item, scopes, true),
            0x1427BB398 /* NDoctrines::CAddMasteryBonusEffect */ => self.mastery_bonus(item, scopes, false),
            0x1427A92F0 /* CAddProjectProgressRatioEffect@?A0x9898cfbe@NProject */ => self.project_progress(item, scopes),
            0x14273C420 /* CInheritTechnologyEffect */ => self.inherit_technology(item, scopes),
            0x142730260 /* NIndustrialOrganisation::CAddResearchBonusEffect */ => self.mio_research_bonus(item, scopes, false),
            0x1427A9068 /* CCompleteProjectEffect@?A0x9898cfbe@NProject */ => self.complete_project(item, scopes),
            0x14273B7A0 /* CStealRandomTechBonusEffect */ => self.steal_technology_bonus(item, scopes),
            0x1427BAEB8 /* NDoctrines::CSetSubDoctrineEffect */ => self.set_doctrine(item, scopes, true),
            0x1427BB1F8 /* NDoctrines::CAddDailyMasteryEffect */ => self.mastery_bonus(item, scopes, true),
            0x14273BF00 /* CSetResearchSlotsEffect */ => self.research_slots(item, scopes, true),
            0x1427A9C50 /* CAddBreakthroughPoints@?A0x9898cfbe@NProject */ => self.breakthrough(item, scopes, false),
            0x1427BAD00 /* NDoctrines::CSetGrandDoctrineEffect */ => self.set_doctrine(item, scopes, false),
            0x1427AEEF0 /* CRaidReduceProjectProgressRatioEffect@?A0x8a0aef5d@NRaids */ => self.raid_project_progress(item, scopes),
            0x1427B38B8 /* CAddFactionPowerProjectionEffect */ => self.faction_power_projection(item, scopes),
            0x142730350 /* NIndustrialOrganisation::CSetResearchBonusEffect */ => self.mio_research_bonus(item, scopes, true),
            0x1427B33A8 /* CSetFactionResearhUnlockedEffect */ => self.faction_research_unlocked(item, scopes),
            0x14273BC98 /* CAddDoctrineCostReduction */ => self.doctrine_reduction(item, scopes),

            // events — F
            0x14299FF28 /* CCustomEffectTooltipEffect */ => self.custom_tooltip(item, scopes),
            0x142756450 /* CCountryEventEffect */ => self.country_event(item, scopes),
            0x14275E4C0 /* CRemoveTargetedDecisionEffect */ => self.removed_decision(item, scopes),
            0x142756538 /* CNewsEventEffect */ => self.country_event(item, scopes),
            0x14275DAA0 /* CUnlockDecisionTooltipEffect */ => self.unlock_decision(item, scopes),
            0x14275E3B0 /* CActivateTargetedDecisionEffect */ => self.activate_targeted_decision(item, scopes),
            0x14275E0F0 /* CActivateMissionEffect */ => self.decision_effect(item, scopes),
            0x14275E6F0 /* CAddDaysMissionTimeoutEffect */ => self.mission_days(item, scopes),
            0x14275DDA0 /* CUnlockDecisionCategoryTooltipEffect */ => self.decision_category(item, scopes),
            0x142731C10 /* NIndustrialOrganisation::CShowIndustrialOrgTooltip */ => self.show_mio(item, scopes),
            0x14299FE58 /* CEffectShowDependentTooltip */ => self.event_option(item, scopes),
            0x14275E1C0 /* CRemoveMissionEffect */ => self.decision_effect(item, scopes),
            0x14275DEE8 /* CActivateDecisionEffect */ => self.decision_effect(item, scopes),
            0x14275DFD8 /* CRemoveDecisionEffect */ => self.decision_effect(item, scopes),
            0x1427315D0 /* NIndustrialOrganisation::CUnlockIndustrialOrgTooltipEffect */ => self.unlock_mio(item, scopes),
            0x142737B18 /* CStateEventEffect */ => self.state_event(item, scopes),
            0x14275DC78 /* CActivateMissionTooltipEffect */ => self.decision_effect(item, scopes),
            0x14275EA18 /* CRemoveDecisionOnCooldown */ => self.decision_cooldown(item, scopes),
            0x1427316B8 /* NIndustrialOrganisation::CUnlockPolicyTooltipEffect */ => self.unlock_mio_policy(item, scopes),

            // diplomacy — D2
            0x142758F98 /* CDeclareWarEffect */ => self.war_or_annex(item, scopes, false),
            0x142756B70 /* CAnnexCountryEffect */ => self.war_or_annex(item, scopes, true),
            0x1427B2A08 /* CAddToFactionEffect */ => self.faction(item, scopes),

            0x14275C810 /* CAddAutonomyRatio */ => self.diplomacy_autonomy_add(item, scopes, true), // 140371210
            0x14275C6E8 /* CAddAutonomyScoreEffect */ => self.diplomacy_autonomy_add(item, scopes, false), // 140371620
            0x142763E20 /* CAddCivilWarTarget */ => self.diplomacy_country(item, scopes, b"EFFECT_ADD_CIVIL_WAR_TARGET", false), // 1403719F0
            0x1427B35C0 /* CAddFactionGoalEffect */ => self.faction_goal_content(scopes), // 1404AA450
            0x1427B3BF8 /* CAddFactionGoalSlotEffect */ => self.faction_goal_slot(item, scopes), // 1404AA830
            0x1427B3B10 /* CAddFactionInfluenceRatioEffect */ => Err("native faction influence localisation needs a shared faction expansion environment (1404AAEC0/1404AB530 -> 1410CF4A0; SScriptEnvironment faction +0xC8), which Renderer does not expose; amount and faction membership are recorded, so no new recording field is needed".into()), // 1404AAEC0
            0x1427B39E0 /* CAddFactionInfluenceScoreEffect */ => Err("native faction influence localisation needs a shared faction expansion environment (1404AAEC0/1404AB530 -> 1410CF4A0; SScriptEnvironment faction +0xC8), which Renderer does not expose; amount and faction membership are recorded, so no new recording field is needed".into()), // 1404AB530
            0x1427B3798 /* CAddFactionInitiativeEffect */ => self.faction_initiative(item, scopes), // 1404ABBA0
            0x14275C508 /* CAddToWarEffect */ => self.add_to_war(item, scopes), // 140379070
            0x142761648 /* CBecomeExiledGovernmentEffect */ => self.diplomacy_become_exiled(item, scopes), // 140379CD0
            0x142760950 /* CCancelBorderWar */ => self.end_border_war(item, scopes, false), // 14037A900
            0x1427B23A8 /* CCreateFactionEffect */ => self.create_faction(item, scopes), // 1404AD3D0
            0x1427B2490 /* CCreateFactionFromTemplateEffect */ => self.create_faction_template(scopes), // 1404AD630
            0x142759CB8 /* CCreateWargoalEffect */ => Err("wargoal type content lookup (14037DF80/140A2C5D0) and current country's existing wargoal type/claimed-state set (1403A35A0 -> 140D28650, CWargoal+0x40/+0x48/+0x54) are unavailable; cannot determine the native empty branch".into()),
            0x14275A840 /* CDiplomaticRelationEffect */ => self.diplomatic_relation(item, scopes), // 14037F730
            0x1427B25E8 /* CDismantleFactionEffect */ => self.dismantle_faction(scopes), // 1404ADAD0
            0x142761740 /* CEndExileEffect */ => self.diplomacy_end_exile(item, scopes), // 14037FFF0
            0x142759238 /* CEndPuppetEffect */ => self.diplomacy_pair(item, scopes, b"EFFECT_END_PUPPET", "COUNTRY"), // 1403801E0
            0x1427607E8 /* CFinalizeBorderWar */ => self.end_border_war(item, scopes, true), // 1403804C0
            0x1427570E8 /* CGiveGuaranteeEffect */ => self.diplomacy_country(item, scopes, b"EFFECT_GIVE_GUARANTEE", false), // 1403813C0
            0x142757228 /* CGiveMilitaryAccessEffect */ => self.diplomacy_country(item, scopes, b"EFFECT_GIVE_MILITARY_ACCESS", false), // 140381630
            0x1427B2D30 /* CLeaveFactionEffect */ => self.leave_faction(scopes), // 1404ADE10
            0x142759830 /* CMakePuppetEffect */ => self.diplomacy_make_puppet(item, scopes), // 140384910
            0x142759998 /* CReleaseAutonomyEffect */ => self.diplomacy_autonomy_level(item, scopes, true), // 140387530
            0x142767B70 /* CReleasePuppetEffect<> */ => self.diplomacy_country(item, scopes, b"EFFECT_RELEASE_PUPPET", false), // 14036E730
            0x142767C50 /* CReleasePuppetEffect<> */ => self.diplomacy_country(item, scopes, b"EFFECT_RELEASE_PUPPET", false), // 14036E730
            0x142763F20 /* CRemoveCivilWarTarget */ => self.diplomacy_country(item, scopes, b"EFFECT_REMOVE_CIVIL_WAR_TARGET", false), // 140387E00
            0x142728100 /* CRemoveExileTagEffect */ => self.diplomacy_remove_exile_tag(scopes), // 1402EC6F0
            0x1427B36A8 /* CRemoveFactionGoalEffect */ => self.faction_goal_content(scopes), // 1404AE210
            0x1427B2C30 /* CRemoveFromFactionEffect */ => self.remove_from_faction(item, scopes), // 1404AE5F0
            0x1427590F8 /* CRemoveWargoalEffect */ => self.diplomacy_pair(item, scopes, b"EFFECT_REMOVE_WARGOAL", "COUNTRY"), // 14038C2C0
            0x142759AB8 /* CSetAutonomyEffect */ => self.diplomacy_autonomy_level(item, scopes, false), // 14038D870
            0x142760650 /* CSetBorderWarCombatData */ => self.update_border_war(item, scopes), // 14038DFC0
            0x142738868 /* CSetBorderWarEffect */ => self.set_border_war(item, scopes), // 140317820
            0x1427B32B8 /* CSetFactionManifestEffect */ => self.faction_goal_content(scopes), // 1404AEEE0
            0x1427B3068 /* CSetFactionMemberUpgradeMin */ => self.faction_upgrade_min(scopes), // 1404AF2C0
            0x1427B34D0 /* CSetFactionMilitaryUnlockedEffect */ => self.faction_military_access(item, scopes), // 1404AF650
            0x1427B28D0 /* CSetFactionNameEffect */ => self.set_faction_name(item, scopes), // 1404AF850
            0x1427B2E08 /* CSetFactionRuleEffect */ => self.faction_rule_content(scopes), // 1404AFEB0
            0x1427B27D8 /* CSetFactionSpyMasterEffect */ => self.set_faction_spymaster(scopes), // 1404B0390
            0x142758EB0 /* CSetWarSupportEffect */ => self.numeric_value(item, scopes, b"EFFECT_SET_WAR_SUPPORT"), // 140394630
            0x142756CA8 /* CStartCivilWarEffect */ => Err("native civil-war own text appends politics helper 1403A0010 using selected ideology's party name (14118F860); shared selected-party name accessor and helper are unavailable; no new recording input is required".into()), // 140395FE0
            0x142733A70 /* NInternationalMarket::CCreatePurchaseContractEffect */ => Err("native purchase-contract own text needs shared CountryTag_FormatMarkedName 140BA59E0 and seller-scoped equipment list description 140306B70/1413A8A20; these description accessors are unavailable".into()), // 1403061D0
            0x142733EC8 /* NInternationalMarket::CGiveMarketAccessEffect */ => self.diplomacy_country(item, scopes, b"EFFECT_MARKET_GIVE_MARKET_ACCESS", true), // 140306890

            // states — D2
            0x1427535F8 /* CAddStateClaimEffect */ => self.state_change(item, scopes, b"EFFECT_ADD_STATE_CLAIM"), // 140378010
            0x1427531F8 /* CTransferStateEffect */ => self.state_change(item, scopes, b"EFFECT_TRANSFER_STATE"), // 140398680
            0x1427602D0 /* CStartBorderWarEffect */ => self.start_border_war(item, scopes),

            0x142737C30 /* CAddBuildingConstructionEffect */ => self.building_construction(item, scopes), // 140313210
            0x1427393F8 /* CAddComplianceEffect */ => self.occupation_percent(item, scopes, b"EFFECT_ADD_COMPLIANCE", Some(true)), // 140313910
            0x142738750 /* CAddDemilitarizedZone */ => self.demilitarized_zone(item, scopes), // 140313D10
            0x142737730 /* CAddExtraSharedBuildingSlotsEffect */ => self.extra_shared_slots(item, scopes), // 140314030
            0x1427A75C0 /* CAddOffsiteBuildingEffect@?A0x9cb35315 */ => self.building_construction(item, scopes), // 14048EC50
            0x1427392F0 /* CAddResistanceEffect */ => self.occupation_percent(item, scopes, b"EFFECT_ADD_RESISTANCE", Some(false)), // 140314310
            0x142739C40 /* CAddResistanceTargetEffect */ => self.resistance_target(item, scopes), // 1403144B0
            0x14275B678 /* CAddResourceEffect */ => self.add_resource(item, scopes), // 140376E70
            0x1427537E0 /* CAddStateCoreEffect */ => self.state_change(item, scopes, b"EFFECT_ADD_STATE_CORE"), // 140378300
            0x142764680 /* CBuildRailwayEffect */ => self.build_railway(item, scopes), // 14037A240
            0x142739840 /* CCancelResistance */ => self.cancel_resistance(scopes), // 140315DE0
            0x14273A510 /* CConstructBuildingInRandomProvinceEffect */ => self.construct_random_province(item, scopes), // 140315E10
            0x14275CD80 /* CCreateRailwayGunEffect */ => self.create_railway_gun(item, scopes), // 14037CDA0
            0x142738F40 /* CDamageBuildingEffect */ => self.damage_building(item, scopes), // 1403160C0
            0x142739A10 /* CForceEnableResistanceEffect */ => self.force_enable_resistance(item, scopes), // 140316590
            0x142761B48 /* CGiveResourceRightsEffect */ => self.give_resource_rights(item, scopes), // 140381910
            0x14275C260 /* CGoToProvinceEffect */ => self.province_state_description(item, scopes, true), // 1403821C0
            0x14275C358 /* CGoToStateEffect */ => self.go_to_state(item, scopes), // 140382520
            0x14275E900 /* CModifyBuildingEffect */ => self.modify_building_resources(item, scopes), // 1403851D0
            0x142738058 /* CRemoveBuildingEffect */ => self.remove_building(item, scopes), // 140316760
            0x142761CC8 /* CRemoveResourceRights */ => self.remove_resource_rights(item, scopes), // 14038B470
            0x1427536E8 /* CRemoveStateClaimEffect */ => self.state_change(item, scopes, b"EFFECT_REMOVE_STATE_CLAIM"), // 14038B980
            0x1427538D0 /* CRemoveStateCoreEffect */ => self.state_change(item, scopes, b"EFFECT_REMOVE_STATE_CORE"), // 14038BC70
            0x14275D528 /* CRenameProvinceEffect */ => self.rename_province(item, scopes), // 14038C5B0
            0x14275D660 /* CResetProvinceEffect */ => self.reset_province_name(item, scopes), // 14038CAF0
            0x14273A128 /* CResetStateNameEffect */ => self.state_name_effect(item, scopes, true), // 1403188E0
            0x142737E00 /* CSetBuildingConstructionEffect */ => self.building_construction(item, scopes), // 140317CE0
            0x1427539C8 /* CSetCapitalEffect */ => self.set_capital(item, scopes), // 14038FB90
            0x1427395C8 /* CSetComplianceEffect */ => self.occupation_percent(item, scopes, b"EFFECT_SET_COMPLIANCE", None), // 140318020
            0x1427686F0 /* CSetOrAddVictoryPointsEffect<> */ => self.province_victory_points(item, scopes, false), // 14036E9A0
            0x1427687C0 /* CSetOrAddVictoryPointsEffect<> */ => self.province_victory_points(item, scopes, true), // 14036EBA0
            0x1427534F8 /* CSetProvinceControllerEffect */ => self.province_state_description(item, scopes, false), // 140392F90
            0x1427394E0 /* CSetResistanceEffect */ => self.occupation_percent(item, scopes, b"EFFECT_SET_RESISTANCE", None), // 140318350
            0x142739098 /* CSetStateCategoryEffect */ => self.set_state_category(item), // 1403184E0
            0x142753400 /* CSetStateControllerEffect */ => self.state_change(item, scopes, b"EFFECT_SET_STATE_CONTROLLER"), // 140393560
            0x14273A410 /* CSetStateControllerToEffect */ => self.state_country_to(item, scopes, b"EFFECT_SET_STATE_CONTROLLER_TO"), // 140318680
            0x14273A028 /* CSetStateNameEffect */ => self.state_name_effect(item, scopes, false), // 1403188E0
            0x142753310 /* CSetStateOwnerEffect */ => self.state_change(item, scopes, b"EFFECT_SET_STATE_OWNER"), // 140393850
            0x14273A310 /* CSetStateOwnerToEffect */ => self.state_country_to(item, scopes, b"EFFECT_SET_STATE_OWNER_TO"), // 140318BA0
            0x1427B93C8 /* CSetStrategicStateLocationBuildingLevel */ => self.strategic_state_location(item, scopes), // 14051B6F0
            0x1427396B0 /* CStartResistance */ => self.start_resistance(item, scopes), // 140318E00
            0x142729758 /* CSupplyUnits */ => self.supply_units(item, scopes), // 1402EDC70
            0x14273A1F8 /* CTransferStateToEffect */ => self.state_country_to(item, scopes, b"EFFECT_TRANSFER_STATE_TO"), // 1403192B0

            // units — D2
            0x142760FA8 /* CDeleteTemplateAndItsUnits */ => self.delete_template(item, scopes),
            0x142759FA0 /* CAddAirExperienceEffect */ => self.numeric_value(item, scopes, b"EFFECT_ADD_AIR_EXPERIENCE"), // 140370DD0
            0x1427AC2B8 /* CAddDesignTemplateBonusEffect */ => self.units_design_bonus(item, scopes), // 14049B380
            0x1427AC1C0 /* CAddEquipmentBonus */ => self.units_equipment_bonus(item, scopes), // 14049B640
            0x1427A76C8 /* CAddEquipmentProduction@?A0x9cb35315 */ => self.units_equipment_production(item, scopes), // 14048EA80
            0x1427ABA60 /* CAddEquipmentToStockpileEffect */ => self.units_stockpile(item, scopes), // 14049B6D0
            0x142734050 /* NInternationalMarket::CAddEquipmentSubsidyEffect */ => self.units_subsidy(item, scopes), // 140305540
            0x142759EB0 /* CAddNavyExperienceEffect */ => self.numeric_value(item, scopes, b"EFFECT_ADD_NAVY_EXPERIENCE"), // 140374F30
            0x1427655B0 /* CAddUnitBonusEffect */ => self.units_unit_bonus(item), // 1403794A0
            0x142752CF8 /* CAddUnitsToDivisionTemplateEffect */ => self.units_add_to_template(item, scopes), // 1403794F0
            0x1427977B8 /* CArmyChangeTemplateEffect */ => Err("CEventScope UNIT_REFERENCE (+0x70/+0x74) is not represented by the event graph (140529260); generic live-division naming operands are not recorded (140C72F00/1409B5640)".into()), // 14045DB70
            0x142760DA8 /* CClearTemplateDivisionCapEffect */ => self.units_template(item, scopes, units::TemplateDescription::ClearCap), // 14037B560
            0x1427ABD10 /* CCreateEquipmentVariantEffect */ => self.units_create_variant(item, scopes), // 14049BEA0
            0x142763928 /* CCreateShipEffect */ => self.units_create_ship(item, scopes), // 14037D200
            0x14275C8E0 /* CCreateUnitEffect */ => self.units_create(item, scopes), // 14037D490
            0x1427AEBA0 /* CDamageRaidUnitEffect@?A0x8a0aef5d@NRaids */ => self.units_raid_damage(item, scopes), // 1404A31F0
            0x142763088 /* CDeleteUnitsEffect */ => self.units_template(item, scopes, units::TemplateDescription::Delete), // 14037EEF0
            0x14275D3D8 /* CDestroyShipsEffect */ => self.units_destroy_ships(item, scopes), // 14037F150
            0x1427973F0 /* CDestroyUnitEffect */ => Err("CEventScope UNIT_REFERENCE (+0x70/+0x74) is not represented by the event graph (1405294C0); generic live-unit naming operands are not recorded (140C72F00/1409B5640)".into()), // 14045DF90
            0x142760B88 /* CLockAllTemplateEffect */ => self.units_lock_all(item, scopes), // 140384680
            0x142759330 /* CManpowerAddEffect */ => self.units_manpower(item, scopes), // 140384C00
            0x1427AED68 /* CRaidAddUnitExperienceEffect@?A0x8a0aef5d@NRaids */ => self.numeric_value(item, scopes, b"EFFECT_RAID_GIVE_UNIT_EXPERIENCE"), // 1404A3D00
            0x1427AB7B8 /* CSendEquipmentEffect */ => self.units_send_equipment(item, scopes), // 14049CAA0
            0x1427AB950 /* CSendEquipmentFractionEffect */ => self.units_equipment_fraction(item, scopes, true), // 14049CF20
            0x142760A40 /* CSetDivisionTemplateLockEffect */ => self.units_template(item, scopes, units::TemplateDescription::Lock), // 140390D10
            0x1427AB6C8 /* CSetEquipmentFraction */ => self.units_equipment_fraction(item, scopes, false), // 14049D1D0
            0x14275BC28 /* CSetNavalOobEffect */ => self.politics_oob(item, scopes, b"EFFECT_SET_OOB"), // 140391580
            0x142760CA8 /* CSetTemplateDivisionCapEffect */ => self.units_template(item, scopes, units::TemplateDescription::Cap), // 140393B40
            0x142760EA0 /* CSetTemplateDivisionForceRecruitingEffect */ => self.units_template(item, scopes, units::TemplateDescription::Recruit), // 140393DE0
            0x142797D00 /* CSetUnitOrganization */ => Err("CEventScope UNIT_REFERENCE (+0x70/+0x74) is not represented by the event graph (140529260); Divisions already records the handle".into()), // 14045E6D0
            0x14275D260 /* CTransferNavyEffect */ => self.units_transfer(item, scopes, true), // 1403983B0
            0x142756ED8 /* CTransferUnitsFractionEffect */ => self.units_transfer(item, scopes, false), // 140398970
            0x1427BB750 /* NDoctrines::CUnlockSubUnitEffect */ => self.units_unlock(item, scopes), // 14051E3C0

            // empty — D2
            0x14275C610 /* CAddAIStrategyEffect */ => empty::describe(), // 140176EE0
            0x142752B58 /* CAddDivisionTemplateEffect */ => empty::describe(), // 140176EE0
            0x1427293C8 /* CClearCharacterFlagEffect */ => empty::describe(), // 140176EE0
            0x14275A328 /* CClearGlobalEventTargetEffect */ => empty::describe(), // 140176EE0
            0x1427560F0 /* CClearGlobalFlagEffect */ => empty::describe(), // 140176EE0
            0x14275ACE0 /* CClearRuleEffect */ => empty::describe(), // 140176EE0
            0x1427631D0 /* CCreateDynamicCountry */ => empty::describe(), // Own 1402E9B40; inherited child walk 140532780.
            0x142764020 /* CCreateEntityEffect */ => empty::describe(), // 1402E9B40
            0x142764840 /* CDamageUnitsEffect */ => empty::describe(), // 1402E9B40
            0x14275CC50 /* CDeleteUnitEffect */ => empty::describe(), // 1402E9B40
            0x142764108 /* CDestroyEntityEffect */ => empty::describe(), // 1402E9B40
            0x14275D008 /* CDropCosmeticTagEffect */ => empty::describe(), // 1402E9B40
            0x142763D28 /* CExecuteOperationCoordinatedStrike */ => empty::describe(), // 140176EE0
            0x142739B68 /* CForceDisableResistanceEffect */ => empty::describe(), // Own 1402E9B40; inherited child walk 140532780.
            0x142761A70 /* CForceRecalcModifiersEffect */ => empty::describe(), // 1402E9B40
            0x142764910 /* CForceUpdateMapMode */ => empty::describe(), // 1402E9B40
            0x14275FE70 /* CForEachEffect */ => empty::describe(), // 1402E9B40; empty build 1402E9B40
            0x142760200 /* CForEffect */ => empty::describe(), // 1402E9B40; empty build 1402E9B40
            0x142764AF8 /* CGenerateCharacter */ => empty::describe(), // 1402E9B40
            0x1427695A0 /* CMarkFocusTreeLayoutDirtyEffect */ => empty::describe(), // 1402E9B40
            0x142769680 /* CMarkTechnologyTreeLayoutDirtyEffect */ => empty::describe(), // 1402E9B40
            0x1427292F0 /* CModifyCharacterFlagEffect */ => empty::describe(), // 140176EE0
            0x1427562A0 /* CModifyCountryFlagEffect */ => empty::describe(), // 140176EE0
            0x14299F8A0 /* CModifyGlobalFlagEffect */ => empty::describe(), // 140176EE0
            0x142737968 /* CModifyStateFlagEffect */ => empty::describe(), // 140176EE0
            0x14275EB20 /* CPrintVariablesEffect */ => empty::describe(), // 140176EE0
            0x142763408 /* CReserveDynamicCountryEffect */ => empty::describe(), // 1402E9B40
            0x142729218 /* CSetCharacterFlagEffect */ => empty::describe(), // 140176EE0
            0x1427506A8 /* CSetCountryLeaderIdeologyEffect */ => empty::describe(), // 140176EE0
            0x14272A158 /* CSetCountryLeaderNamePortraitOrDescription<> */ => empty::describe(), // 1402E9B40
            0x14272A228 /* CSetCountryLeaderNamePortraitOrDescription<> */ => empty::describe(), // 1402E9B40
            0x14272A2F8 /* CSetCountryLeaderNamePortraitOrDescription<> */ => empty::describe(), // 1402E9B40
            0x1427643A8 /* CSetEntityAnimationEffect */ => empty::describe(), // 1402E9B40
            0x1427641D8 /* CSetEntityPositionEffect */ => empty::describe(), // 1402E9B40
            0x142756018 /* CSetGlobalFlagEffect */ => empty::describe(), // 140176EE0
            0x142729ED0 /* CSetLeaderNamePortraitOrDescription<> */ => empty::describe(), // 1402E9B40
            0x142729FA8 /* CSetLeaderNamePortraitOrDescription<> */ => empty::describe(), // 1402E9B40
            0x14272A080 /* CSetLeaderNamePortraitOrDescription<> */ => empty::describe(), // 1402E9B40
            0x142764DB0 /* CSetNameEffect */ => empty::describe(), // 1402E9B40
            0x1427651E0 /* CSetOccupationLawEffect */ => empty::describe(), // 140176EE0
            0x142765348 /* CSetOccupationLawWhereAvailableEffect */ => empty::describe(), // 140176EE0
            0x142750CD8 /* CSetPoliticalPartyEffect */ => empty::describe(), // 1402E9B40; empty build 1402E9B40
            0x142729AA0 /* CSetPortraitEffect */ => empty::describe(), // 1402E9B40
            0x1427A3B60 /* CSetPowerBalanceGfx */ => empty::describe(), // 140176EE0
            0x1427A9EA0 /* CSetProjectFlagEffect@?A0x9898cfbe@NProject */ => empty::describe(), // 140176EE0
            0x142737868 /* CSetStateFlagEffect */ => empty::describe(), // 140176EE0
            0x142737598 /* CSetStateProvincesControllerEffect */ => empty::describe(), // 1402E9B40
            0x1427374C0 /* CTeleportArmiesEffect */ => empty::describe(), // 1402E9B40
            0x1427649E0 /* CTeleportRailwayGunsToDeployProvinceEffect */ => empty::describe(), // 1402E9B40
            0x14275D0E0 /* CTransferShip */ => empty::describe(), // 140176EE0
            0x142760108 /* CWhileEffect */ => empty::describe(), // 1402E9B40; empty build 1402E9B40
            0x1427253F8 /* NCareerProfile::CStepMissiolinis */ => empty::describe(), // 140176EE0
            0x14272FF18 /* NIndustrialOrganisation::CClearIndustrialOrgFlagEffect */ => empty::describe(), // 140176EE0
            0x14272FD68 /* NIndustrialOrganisation::CSetIndustrialOrgFlagEffect */ => empty::describe(), // 140176EE0
            0x1427561C8 /* CSetCountryFlagEffect */ => empty::describe(), // 140176EE0
            0x142756378 /* CClearCountryFlagEffect */ => empty::describe(), // 140176EE0
            0x142737A40 /* CClearStateFlagEffect */ => empty::describe(), // 140176EE0

            _ => return None,
        })
    }
}
