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

            // units — D2
            0x142760FA8 /* CDeleteTemplateAndItsUnits */ => self.delete_template(item, scopes),

            // empty — D2
            0x1427561C8 /* CSetCountryFlagEffect */ | 0x142756378 /* CClearCountryFlagEffect */ | 0x142737A40 /* CClearStateFlagEffect */ => {
                empty::describe()
            }

            _ => Err(format!(
                "{}@{:X} description is not ported (CEffect::BuildDescription 140532780)", class.name, class.vtable
            )),
        }
    }
}
