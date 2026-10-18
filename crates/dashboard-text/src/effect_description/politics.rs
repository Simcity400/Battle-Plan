//! Country/political own descriptions. Operands retain the effect_keys readers.

use super::*;
use replay_data::rows::tables;

mod content;
mod rules;
pub(super) use content::PoliticsContent;

#[cfg(test)]
mod tests;

pub(super) const NULL_IDEOLOGY: &str = Evaluator::NULL_IDEOLOGY;

impl Renderer<'_, '_, '_> {
    // cached named group, numeric database
    // ID, then decoded country's ruling group. IDs/tokens are
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
        if names.counts.is_ideology(operand)? {
            // Cached named pointers open no variable session or strict map
            // world; cached lookup returns immediately.
            let world = match self.context.expansion.and_then(|scopes| scopes.world) {
                Some(world) => world,
                None => self.context.text.trigger_world()?,
            };
            return Ok(world.named_ideology(operand)?.1.to_vec());
        }
        let (_, token) = self
            .evaluator()?
            .ideology_specifier(Some(operand), scopes, |token| {
                names.counts.is_ideology(token)
            })?;
        Ok(token.to_vec())
    }

    // noun suffix, IDEOLOGY and VALUE, explicit LF.

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
        noun.extend_from_slice(b"_noun"); // native suffix.
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

    // Keep long_name and name in separate fields.
    // Expand the selected name before NAME substitution;
    // ideology name remains plain until the outer expansion.

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

    // choose the authored base's definite key,
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
            .ok_or("country naming operands are absent")?
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

    // gate on client debug byte.
    // Viewer policy: use debug off, like its selection/pause decisions.
    pub(super) fn politics_shine(&self) -> Result<String, String> {
        Ok(String::new())
    }

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

    // ordinary name, no scope expansion.
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

    // Substitute the name without scope expansion.
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

    // a positive carried state selects
    // the country operand; otherwise keep scope.country and select a state.
    // parses selectors first, then scoped variable; the core target
    // helpers share country/state evaluation. No explicit LF.
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
                    .ok_or("contested-owner caller scope is absent")?
                    .this
            }
            None => match scopes.this {
                Scope::Country(country) | Scope::CountryLeader { country, .. } => {
                    EventObject::Country(country)
                }
                Scope::State(state) => EventObject::State(state),
                _ => return Err("contested-owner scope lacks native country/state fields".into()),
            },
        };
        let carried_state = match current {
            EventObject::State(state) | EventObject::CountryState { state, .. } => state,
            EventObject::Country(_) | EventObject::CountryUnitLeader { .. } => 0,
            _ => return Err("contested-owner scope lacks native country/state fields".into()),
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
        // set country, then state in a default scope;
        // have no arguments.
        self.localised(
            key,
            &[],
            self.fresh_country_state_scope(country, state)?,
            true,
        )
    }

    // gates on the operand but describes the CURRENT country.

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

    // Copy the raw operand into the wrapper; no LF.
    pub(super) fn politics_oob(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
    ) -> Result<String, String> {
        let name = item.value.c_str().map_err(|why| why.to_string())?;
        self.localised(key, &[("NAME", Argument::Text(name))], scopes, true)
    }

    // tree string, localise/expand before TREE_NAME substitution.

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

    // name is execution-only; zero selects REMOVE, negate wraps.

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

    // Sound names are RAW strings.

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

    // bool read, not a scoped country operand.

    pub(super) fn politics_major(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let enabled = item
            .value
            .scalar()
            .and_then(installed_content_core::parser::read_bool)
            .unwrap_or(false);
        let key = if enabled {
            b"EFFECT_SET_MAJOR_YES".as_slice()
        } else {
            b"EFFECT_SET_MAJOR_NO".as_slice()
        };
        let name = self.country_name(self.current_country(scopes)?, false)?;
        self.line(key, &[("COUNTRY", Argument::Text(&name))], scopes, true)
    }

    // Localise and expand the name, then substitute NAME into the wrapper.
    pub(super) fn politics_mio_name(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let name = self.localised(item.value.scalar().unwrap_or_default(), &[], scopes, true)?;
        self.line(
            b"EFFECT_MIO_SET_NAME_KEY",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    // feature 0x19 ( bit 5), then agency == 0.
    // retain the recorded table cadence.
    pub(super) fn politics_create_agency(&self, scopes: Scopes) -> Result<String, String> {
        let frame = self.context.text.frame();
        let [features] = frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|why| why.to_string())?
        else {
            return Err("SimulationFeatures lacks the agency feature mask".into());
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
            .ok_or("IntelligenceAgencies lacks agency identity")?;
        if row.created != 0 {
            return Ok(String::new());
        }
        self.line(b"EFFECT_CREATE_INTELLIGENCE_AGENCY", &[], scopes, true)
    }

    // Block reader consumes target, not captured_by.
    // No scope expansion and no LF; captive counts do not enter the description.
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

    // reader appends focus tokens and stores signed32 cost.
    // Names alone expand; header/list localisations do not.
    pub(super) fn politics_focus_cost(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let cost = field(fields, "cost")
            .and_then(|item| item.value.scalar())
            .and_then(|value| installed_content_core::crt::scan_i32(value).stored())
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
            // REDUCE_FOCUS_DAYS_EFFECT and its negative variant select the header.
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
            // Header and item keys are localised separately.
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
