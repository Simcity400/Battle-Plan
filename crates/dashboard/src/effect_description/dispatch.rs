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

    pub(super) fn leaf(&mut self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let class = self
            .effect_class(item)
            .ok_or_else(|| format!("unknown effect key {} (ReadEffect 1405345D0)", key(item)))?;
        match class.vtable {
            // scopes — I
            0x14299F6F8 /* CContextEffect */ => self
                .context_header(item, scopes)
                .map(|selected| selected.map(|(_, header)| header).unwrap_or_default()),
            0x1427576E8 /* CEveryStateEffect */ => lists::own(), // Own 140176EE0, build 140312880.

            // variables — I
            0x14275F568 /* CClearVariableEffect */ => variables::clear_variable(),
            0x14275A188 /* CSaveEventTargetAsEffect */ => self.save_event_target(item, scopes),

            // lists — I
            0x1427BE590 /* CIfEffect */ | 0x14299FCB8 /* CHiddenEffect */ | 0x14299FD88 /* CEffectTooltipEffect */ => lists::own(),

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
            // politics — F
            0x1429A04C8 /* CLogScriptEffect */ => Ok(String::new()), // 140176EE0
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
            0x14273C320 /* CModifyTechnologySharingBonusEffect */ => Err("runtime technology-sharing template identity (+0x48) is unrecorded (140321840 / 1413BCF50)".into()), // 140321840
            0x14275B4B0 /* CPlaySongEffect */ => self.politics_sound(item, scopes, false), // 140385DA0
            0x1427976C0 /* CAddHistoryEntryEffect */ => Err("CEventScope UNIT_REFERENCE (+0x70/+0x74) is not represented by the event graph (140529260); Divisions records the handles".into()), // 14045D2E0
            0x142761218 /* CRecallVolunteersFromEffect */ => self.politics_country(item, scopes, b"EFFECT_RECALL_VOLUNTEERS_FROM", true), // 140386960
            0x142759440 /* CHoldElectionEffect */ => self.politics_country(item, scopes, b"POLITICS_HOLD_ELECTION", false), // 140382C20
            0x1427526E0 /* CSetPoliticalPowerEffect */ => self.numeric_value(item, scopes, b"POLITICS_SET_POLITICAL_POWER"), // 1403927B0
            0x142731308 /* NIndustrialOrganisation::CSetOrganisationNameKeyEffect */ => self.politics_mio_name(item, scopes), // 1402FE660
            0x142764F78 /* CBreakEmbargoEffect */ => self.politics_country(item, scopes, b"EFFECT_BREAK_EMBARGO", false), // 140379FD0
            0x14275E5B0 /* CAddDaysRemoveDecisionEffect */ => self.decision_remove_days(item, scopes), // 140373020
            0x1427313F8 /* NIndustrialOrganisation::CSetOrganisationIconEffect */ => self.line(b"EFFECT_MIO_SET_ICON", &[], scopes, true), // 1402FE4E0
            0x1427AF710 /* CStateAndCountryEffect<> */ => Err("fresh country/state localisation scope API unavailable (1404A5070 / 1404A5160)".into()), // 1404A4F10
            0x1427BB648 /* NDoctrines::CUnlockCombatTacticEffect */ => self.politics_unlock_tactic(item, scopes), // 14051E260
            0x14272BB90 /* CReleaseFromCaptivityEffect */ => self.localised(b"EFFECT_RELEASE_FROM_CAPTIVITY", &[], scopes, false), // 1402F8610
            0x14272BA58 /* CReleaseCapturedGeneralsEffect */ => self.politics_release_generals(item, scopes), // 1402F8420
            0x14272B820 /* CCaptureGeneralEffect */ => self.character_capture_by(item, scopes), // 1402F8130
            0x14275B090 /* CRemoveRelationRuleOverrideEffect */ => self.politics_relation_rule(item, scopes, true, None), // 14038AFB0
            0x142761120 /* CRecallAttacheEffect */ => self.politics_country(item, scopes, b"EFFECT_RECALL_ATTACHE", true), // 140386680
            0x1427AF7E0 /* CStateAndCountryEffect<> */ => Err("fresh country/state localisation scope API unavailable (1404A5070 / 1404A5160)".into()), // 1404A4FC0
            0x14275BB58 /* CSetOobEffect */ => self.politics_oob(item, scopes, b"EFFECT_SET_OOB"), // 140391580
            0x14275ADB0 /* CSetPartyRuleEffect */ => self.politics_rules(item, scopes, true), // 140392590

            // technology — F
            0x1427BB520 /* NDoctrines::CAddMasteryEffect */ => self.mastery(item, scopes),
            0x14273BC98 /* CAddDoctrineCostReduction */ => self.doctrine_reduction(item, scopes),

            // events — F
            0x14299FF28 /* CCustomEffectTooltipEffect */ => self.custom_tooltip(item, scopes),
            0x142756450 /* CCountryEventEffect */ => self.country_event(item, scopes),
            0x14275E4C0 /* CRemoveTargetedDecisionEffect */ => self.removed_decision(item, scopes),

            // diplomacy — D2
            0x142758F98 /* CDeclareWarEffect */ => self.war_or_annex(item, scopes, false),
            0x142756B70 /* CAnnexCountryEffect */ => self.war_or_annex(item, scopes, true),
            0x1427B2A08 /* CAddToFactionEffect */ => self.faction(item, scopes),

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

            // empty — D2
            0x14275C610 /* CAddAIStrategyEffect */ => empty::describe(), // 140176EE0
            0x142752B58 /* CAddDivisionTemplateEffect */ => empty::describe(), // 140176EE0
            0x1427293C8 /* CClearCharacterFlagEffect */ => empty::describe(), // 140176EE0
            0x14275A328 /* CClearGlobalEventTargetEffect */ => empty::describe(), // 140176EE0
            0x1427560F0 /* CClearGlobalFlagEffect */ => empty::describe(), // 140176EE0
            0x14275ACE0 /* CClearRuleEffect */ => empty::describe(), // 140176EE0
            0x1427631D0 /* CCreateDynamicCountry */ => self.empty_child_container(item), // 1402E9B40
            0x142764020 /* CCreateEntityEffect */ => empty::describe(), // 1402E9B40
            0x142764840 /* CDamageUnitsEffect */ => empty::describe(), // 1402E9B40
            0x14275CC50 /* CDeleteUnitEffect */ => empty::describe(), // 1402E9B40
            0x142764108 /* CDestroyEntityEffect */ => empty::describe(), // 1402E9B40
            0x14275D008 /* CDropCosmeticTagEffect */ => empty::describe(), // 1402E9B40
            0x142763D28 /* CExecuteOperationCoordinatedStrike */ => empty::describe(), // 140176EE0
            0x142739B68 /* CForceDisableResistanceEffect */ => self.empty_child_container(item), // 1402E9B40
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

            _ => Err(format!(
                "{}@{:X} description is not ported (CEffect::BuildDescription 140532780)", class.name, class.vtable
            )),
        }
    }
}
