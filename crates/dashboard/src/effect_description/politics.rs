//! Country/political own descriptions. Operands retain the effect_keys readers.

use super::*;
use replay::rows::tables;

mod content;
mod rules;
pub(super) use content::PoliticsContent;

#[cfg(test)]
mod tests;

pub(super) const NULL_IDEOLOGY: &str =
    "ideology database null-object identity (+0x78, 1402EE8E0) is unrecorded";

impl Renderer<'_, '_, '_> {
    // ResolveIdeologySpecifier 1402EE8E0: cached named group, numeric database
    // ID, then decoded country's ruling group (14138B640). IDs/tokens are
    // recorded native identities; KeyNames verifies the content-stack group.
    pub(super) fn politics_ideology(
        &self,
        operand: Option<&[u8]>,
        scopes: Scopes,
    ) -> Result<Vec<u8>, String> {
        let operand = operand.ok_or(NULL_IDEOLOGY)?;
        let names = self
            .context
            .content
            .key_names
            .ok_or_else(|| self.content_gap("effect key names"))?;
        let frame = self.context.text.frame();
        let parties = frame
            .complete::<tables::PoliticalParties>()
            .map_err(|why| why.to_string())?;
        if names.counts.is_ideology(operand)? {
            return parties
                .iter()
                .find(|row| row.token.eq_ignore_ascii_case(operand))
                .map(|row| row.token.clone())
                .ok_or_else(|| {
                    format!(
                        "ideology {} has no recorded native ID/token",
                        String::from_utf8_lossy(operand)
                    )
                });
        }
        let value = self.scoped_value(operand, scopes)?;
        let id = (value / 100_000) as u32;
        if let Some(party) = parties.iter().find(|row| row.id == id) {
            if !names.counts.is_ideology(&party.token)? {
                return Err(format!(
                    "ideology {} is absent from recorded content",
                    String::from_utf8_lossy(&party.token)
                ));
            }
            return Ok(party.token.clone());
        }
        // Decode the evaluated constant through the shared evaluator; the
        // authored operand is evaluated only once, retaining its session.
        let constant = installed_content::fixed5::fixed5_to_string(value, 5);
        let Some(country) = self
            .evaluator()?
            .scoped_country(constant.as_bytes(), scopes)?
        else {
            return Err(NULL_IDEOLOGY.into());
        };
        let politics = frame
            .complete::<tables::Politics>()
            .map_err(|why| why.to_string())?;
        let party = politics
            .iter()
            .find(|row| self.context.text.recorded().slot(row.country) == Some(country))
            .and_then(|row| row.ruling_party)
            .ok_or("Politics lacks ruling ideology (14138B640)")?;
        let token = parties
            .iter()
            .find(|row| {
                row.id == party && self.context.text.recorded().slot(row.country) == Some(country)
            })
            .ok_or("PoliticalParties lacks ruling ideology (14138B640)")?
            .token
            .clone();
        if !names.counts.is_ideology(&token)? {
            return Err(format!(
                "ideology {} is absent from recorded content",
                String::from_utf8_lossy(&token)
            ));
        }
        Ok(token)
    }

    // 140376270; noun suffix 14117C8D0, IDEOLOGY and VALUE, explicit LF.
    // POLITICS_IDEOLOGY_POPULARITY_CHANGE@142759618.
    pub(super) fn politics_popularity(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let ideology = self.politics_ideology(
            field(children(item), "ideology").and_then(|item| item.value.scalar()),
            scopes,
        )?;
        let mut noun = ideology;
        noun.extend_from_slice(b"_noun"); // 14117C8D0 native suffix.
        let name = self.localised(&noun, &[], scopes, false)?;
        let value = field(children(item), "popularity")
            .map(|item| self.scoped_value(item.value.scalar().unwrap_or_default(), scopes))
            .transpose()?
            .unwrap_or(0);
        self.line(
            b"POLITICS_IDEOLOGY_POPULARITY_CHANGE",
            &[
                ("IDEOLOGY", Argument::Text(&name)),
                ("VALUE", Argument::Fixed5(value)),
            ],
            scopes,
            true,
        )
    }

    // 140392160: long_name (+0x88), written by 1403B5280 token 0x34EB;
    // name (0x1B) writes +0x68. Expand +0x88 before NAME substitution;
    // ideology +0x18 name 140616BC0 remains plain until the outer expansion.
    // POLITICS_PARTY_NAME_CHANGE@142750EF8.
    pub(super) fn politics_party_name(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let ideology = self.politics_ideology(
            field(children(item), "ideology").and_then(|item| item.value.scalar()),
            scopes,
        )?;
        let ideology = self.localised(&ideology, &[], scopes, false)?;
        let name = self.localised(
            field(children(item), "long_name")
                .and_then(|item| item.value.scalar())
                .unwrap_or_default(),
            &[],
            scopes,
            true,
        )?;
        self.line(
            b"POLITICS_PARTY_NAME_CHANGE",
            &[
                ("IDEOLOGY", Argument::Text(&ideology)),
                ("NAME", Argument::Text(&name)),
            ],
            scopes,
            true,
        )
    }

    // 1403900D0 -> 1406E0440: choose the authored base's definite key,
    // expand it in the caller, then substitute current definite COUNTRY.
    pub(super) fn politics_cosmetic(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let base = item.value.c_str().map_err(|why| why.to_string())?;
        let Some(country) = (match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => Some(country),
            _ => None,
        }) else {
            return Ok(String::new());
        };
        if base.is_empty() {
            return Ok(String::new());
        }
        let operands = self
            .context
            .text
            .country_names()
            .operands(country)
            .ok_or("country naming operands are absent (1403900D0)")?
            .as_ref()
            .map_err(Clone::clone)?;
        let current = self.country_name(country, true)?;
        let candidate = match operands.candidate(base, "_DEF", self.context.localisation) {
            Some(key) => self.localised(key.as_bytes(), &[], scopes, true)?,
            None => self.expanded(
                &self.localised(current.as_bytes(), &[], scopes, false)?,
                scopes,
            )?,
        };
        self.line(
            b"EFFECT_SET_COSMETIC_TAG",
            &[
                ("COUNTRY", Argument::Text(&current)),
                ("COSMETIC_NAME", Argument::Text(&candidate)),
            ],
            scopes,
            true,
        )
    }

    // 14036FE00 / 14037E730 gate on client debug byte 143316899.
    // Viewer policy: use debug off, like its selection/pause decisions.
    pub(super) fn politics_shine(&self) -> Result<String, String> {
        Ok(String::new())
    }

    // 14036E4C0, 14037FD80, 140379FD0, 140382C20, 140386680, 140386960.
    pub(super) fn politics_country(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
        definite: bool,
    ) -> Result<String, String> {
        let Some(country) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let name = self.country_name(country, definite)?;
        self.line(key, &[("COUNTRY", Argument::Text(&name))], scopes, true)
    }

    // 140313AB0 / 140316D10: ordinary name, no ProcessString; COUNTRY@142728968.
    pub(super) fn politics_core(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
    ) -> Result<String, String> {
        let name = self
            .country_target(item, scopes)?
            .map(|country| self.country_name(country, false))
            .transpose()?
            .unwrap_or_default();
        self.line(key, &[("COUNTRY", Argument::Text(&name))], scopes, false)
    }

    // 140313620 / 140316A20: TAG@142738440 then COUNTRY@142728968, no expansion.
    pub(super) fn politics_claim(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
    ) -> Result<String, String> {
        let country = self.country_target(item, scopes)?;
        let name = country
            .map(|country| self.country_name(country, false))
            .transpose()?
            .unwrap_or_default();
        let tag = country
            .map(|country| self.tag(country))
            .transpose()?
            .unwrap_or_default();
        self.line(
            key,
            &[
                ("TAG", Argument::Text(tag)),
                ("COUNTRY", Argument::Text(&name)),
            ],
            scopes,
            false,
        )
    }

    // Own 1404A4F10/1404A4FC0: a positive carried state (+0xA8) selects
    // the country operand; otherwise keep scope.country (+8) and select a state.
    // 1404A4750 parses selectors first, then CScopedVariable; the core target
    // helpers share vf15/vf16 and EvaluateCountryTag/StateId. No explicit LF.
    pub(super) fn politics_contested_owner(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
    ) -> Result<String, String> {
        let current = match scopes.cursor {
            Some(at) => {
                self.graph
                    .borrow()
                    .scopes
                    .get(at)
                    .ok_or("contested-owner caller scope is absent (1404A4F10/1404A4FC0)")?
                    .this
            }
            None => match scopes.this {
                Scope::Country(country) | Scope::CountryLeader { country, .. } => {
                    EventObject::Country(country)
                }
                Scope::State(state) => EventObject::State(state),
                _ => return Err(
                    "contested-owner scope lacks native country/state fields (1404A4F10/1404A4FC0)"
                        .into(),
                ),
            },
        };
        let carried_state =
            match current {
                EventObject::State(state) | EventObject::CountryState { state, .. } => state,
                EventObject::Country(_) | EventObject::CountryUnitLeader { .. } => 0,
                _ => return Err(
                    "contested-owner scope lacks native country/state fields (1404A4F10/1404A4FC0)"
                        .into(),
                ),
            };
        let (country, state) = if (carried_state as i32) > 0 {
            (
                self.country_target(item, scopes)?.unwrap_or(0),
                carried_state,
            )
        } else {
            let country = match current {
                EventObject::Country(country)
                | EventObject::CountryState { country, .. }
                | EventObject::CountryUnitLeader { country, .. } => country,
                _ => 0,
            };
            (country, self.state_target(item, scopes)?)
        };
        // 1404A5070/1404A5160 set country, then state in a default scope;
        // EFFECT_ADD_CONTESTED_OWNER@1427AF660 / REMOVE@1427AF6B8 have no arguments.
        self.localised(
            key,
            &[],
            self.fresh_country_state_scope(country, state)?,
            true,
        )
    }

    // 14037B2B0 gates on the operand but describes the CURRENT country.
    // CHANGE_TAG_EFFECT@142759720, COUNTRY@142728968.
    pub(super) fn politics_change_tag(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        if self.country_target(item, scopes)?.is_none() {
            return Ok(String::new());
        }
        let name = self.country_name(self.current_country(scopes)?, false)?;
        self.line(
            b"CHANGE_TAG_EFFECT",
            &[("COUNTRY", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    // 140384500 / 140391580 copy the raw operand into NAME@142727E7C; no LF.
    pub(super) fn politics_oob(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
    ) -> Result<String, String> {
        let name = item.value.c_str().map_err(|why| why.to_string())?;
        self.localised(key, &[("NAME", Argument::Text(name))], scopes, true)
    }

    // 1403909F0: tree string (+0x60), localise/expand before TREE_NAME substitution.
    // TREE_NAME@14274FEA8, EFFECT_LOAD_FOCUS_TREE@14274FEB8.
    pub(super) fn politics_focus_tree(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let tree = item
            .value
            .scalar()
            .or_else(|| field(children(item), "tree").and_then(|item| item.value.scalar()))
            .unwrap_or_default();
        let name = self.localised(tree, &[], scopes, true)?;
        self.line(
            b"EFFECT_LOAD_FOCUS_TREE",
            &[("TREE_NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    // 1403748C0: name is execution-only; zero selects REMOVE, negate wraps.
    // EFFECT_ADD_THREAT@14275AAF8 / EFFECT_REMOVE_THREAT@14275AB10, THREAT@14275AAEC.
    pub(super) fn politics_named_threat(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let value = match field(children(item), "threat") {
            Some(item) => self.scoped_value(item.value.scalar().unwrap_or_default(), scopes)?,
            None => 0,
        };
        let (key, value) = if value < 1 {
            (b"EFFECT_REMOVE_THREAT".as_slice(), value.wrapping_neg())
        } else {
            (b"EFFECT_ADD_THREAT".as_slice(), value)
        };
        self.line(key, &[("THREAT", Argument::Fixed5(value))], scopes, true)
    }

    // 140385DA0 -> 1402F8DA0; 14038D800 -> 1403276C0. Sound names are RAW strings.
    // SOUND_NAME@14275B388, EFFECT_SOUND_DESC@14275B398 / EFFECT_SCOPED_SOUND_DESC@14275B488.
    pub(super) fn politics_sound(
        &self,
        item: &Item,
        scopes: Scopes,
        scoped: bool,
    ) -> Result<String, String> {
        let sound = item.value.c_str().map_err(|why| why.to_string())?;
        if scoped {
            let name = self.country_name(self.current_country(scopes)?, false)?;
            self.localised(
                b"EFFECT_SCOPED_SOUND_DESC",
                &[
                    ("SOUND_NAME", Argument::Text(sound)),
                    ("COUNTRY", Argument::Text(&name)),
                ],
                scopes,
                true,
            )
        } else {
            self.localised(
                b"EFFECT_SOUND_DESC",
                &[("SOUND_NAME", Argument::Text(sound))],
                scopes,
                true,
            )
        }
    }

    // 140391AB0; bool read 1424B06E0, not a scoped country operand.
    // EFFECT_SET_MAJOR_YES@14275C228 / EFFECT_SET_MAJOR_NO@14275C240.
    pub(super) fn politics_major(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let enabled = item
            .value
            .scalar()
            .and_then(installed_content::parser::read_bool)
            .unwrap_or(false);
        let key = if enabled {
            b"EFFECT_SET_MAJOR_YES".as_slice()
        } else {
            b"EFFECT_SET_MAJOR_NO".as_slice()
        };
        let name = self.country_name(self.current_country(scopes)?, false)?;
        self.line(key, &[("COUNTRY", Argument::Text(&name))], scopes, true)
    }

    // 1402FE660: localise/expand +0x58, then NAME in EFFECT_MIO_SET_NAME_KEY@1427313D8.
    pub(super) fn politics_mio_name(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let name = self.localised(item.value.scalar().unwrap_or_default(), &[], scopes, true)?;
        self.line(
            b"EFFECT_MIO_SET_NAME_KEY",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    // 14037CBE0: feature 0x19 (1401AE640 bit 5), then agency+0xC0 == 0.
    // EFFECT_CREATE_INTELLIGENCE_AGENCY@142762F00; retain the recorded table cadence.
    pub(super) fn politics_create_agency(&self, scopes: Scopes) -> Result<String, String> {
        let frame = self.context.text.frame();
        let [features] = frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|why| why.to_string())?
        else {
            return Err(
                "SimulationFeatures lacks the agency feature mask (1401AE640 case 0x19)".into(),
            );
        };
        if features.dlc_mask & (1 << 5) == 0 {
            return Ok(String::new());
        }
        let country = self.current_country(scopes)?;
        let rows = frame
            .complete::<tables::IntelligenceAgencies>()
            .map_err(|why| why.to_string())?;
        let row = rows
            .iter()
            .find(|row| self.context.text.recorded().slot(row.country) == Some(country))
            .ok_or("IntelligenceAgencies lacks agency+0xC0 (14037CBE0)")?;
        if row.created != 0 {
            return Ok(String::new());
        }
        self.line(b"EFFECT_CREATE_INTELLIGENCE_AGENCY", &[], scopes, true)
    }

    // 1402F8420: COUNTRY_NAME_DEF@14272BA20, EFFECT_RELEASE_GENERALS@14272BB70.
    // Block reader 1402F8680 consumes target (+0x58), not captured_by.
    // No ProcessString and no LF; captive counts do not enter the description.
    pub(super) fn politics_release_generals(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let target = item
            .value
            .scalar()
            .or_else(|| field(children(item), "target").and_then(|item| item.value.scalar()));
        let Some(country) = target
            .map(|target| self.evaluator()?.scoped_country(target, scopes))
            .transpose()?
            .flatten()
        else {
            return Ok(String::new());
        };
        let name = self.country_name(country, true)?;
        self.localised(
            b"EFFECT_RELEASE_GENERALS",
            &[("COUNTRY_NAME_DEF", Argument::Text(&name))],
            scopes,
            false,
        )
    }

    // 140386F40; reader 1403B3E00 appends focus tokens and stores signed32 cost.
    // Names alone expand; header/list localisations do not. DAYS@14275179C.
    pub(super) fn politics_focus_cost(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let cost = field(fields, "cost")
            .and_then(|item| item.value.scalar())
            .and_then(|value| installed_content::crt::scan_i32(value).stored())
            .unwrap_or(0);
        let focuses = fields
            .iter()
            .filter(|item| item.keyword_is("focus"))
            .flat_map(|item| match item.value.items() {
                Some(items) => items
                    .iter()
                    .filter_map(|item| item.value.scalar())
                    .collect::<Vec<_>>(),
                None => item.value.scalar().into_iter().collect(),
            })
            .collect::<Vec<_>>();
        if focuses.is_empty() {
            return Ok(String::new());
        }
        let days = cost.wrapping_abs();
        let mut text = if focuses.len() == 1 {
            let name = self.localised(focuses[0], &[], scopes, true)?;
            // 140386F40 literals REDUCE_FOCUS_DAYS_EFFECT[_NEGATIVE] and FOCUS@14275A6E0.
            self.localised(
                if cost < 0 {
                    b"REDUCE_FOCUS_DAYS_EFFECT_NEGATIVE"
                } else {
                    b"REDUCE_FOCUS_DAYS_EFFECT"
                },
                &[
                    ("FOCUS", Argument::Text(&name)),
                    ("DAYS", Argument::Int32(days)),
                ],
                scopes,
                false,
            )?
        } else {
            // Header literals at 140386F40; item key REDUCE_FOCUS_DAYS_MULTIPLE_FOCUS@1427657F0.
            let mut text = self.localised(
                if cost < 0 {
                    b"REDUCE_FOCUS_DAYS_MULTIPLE_HEADER_NEGATIVE"
                } else {
                    b"REDUCE_FOCUS_DAYS_MULTIPLE_HEADER"
                },
                &[("DAYS", Argument::Int32(days))],
                scopes,
                false,
            )?;
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            for focus in focuses {
                let name = self.localised(focus, &[], scopes, true)?;
                text.push_str(&self.localised(
                    b"REDUCE_FOCUS_DAYS_MULTIPLE_FOCUS",
                    &[("FOCUS", Argument::Text(&name))],
                    scopes,
                    false,
                )?);
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
            }
            text
        };
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }
}
