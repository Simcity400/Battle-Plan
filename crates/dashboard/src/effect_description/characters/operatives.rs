//! Operative actions and unit-leader event title scopes.

use super::*;
use crate::scripted_text::{EventObject, EventScope, Link, random};

#[cfg(test)]
mod tests;

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    // 1403AA100: a scalar in a character scope is the country operand;
    // in other scopes it is the operative selector at +0x130.
    fn char_action_operands<'a>(
        &self,
        item: &'a Item,
        scopes: Scopes,
        country_key: &str,
    ) -> (Option<&'a [u8]>, Option<&'a [u8]>) {
        let scalar = item.value.scalar();
        let character_scope = matches!(scopes.this, Scope::Character(_) | Scope::Leader(_));
        (
            field(children(item), "operative")
                .and_then(|item| item.value.scalar())
                .or(scalar.filter(|_| !character_scope)),
            field(children(item), country_key)
                .and_then(|item| item.value.scalar())
                .or(scalar.filter(|_| character_scope)),
        )
    }

    // 140536B10 -> 140F8A840, then current scope's 1405294F0 fallback.
    fn char_action_operative(
        &self,
        operand: Option<&[u8]>,
        scopes: Scopes,
    ) -> Result<Option<UnitLeader<'f>>, String> {
        let selected = self.effect_character_target(operand, scopes)?;
        let leader = match selected {
            Some(character) => Tables::read(self.context.text.frame()).unit_leader(character.id)?,
            None => None,
        };
        let leader = match leader {
            Some(leader) => Some(leader),
            None => self.effect_unit_leader(scopes)?,
        };
        Ok(leader.filter(|leader| leader.category() == 3))
    }

    // 140536ED0, then scope+8 and ResolveCountryScope 1405336C0. The core
    // has already applied the effect's country selector to this scope.
    fn char_action_country(
        &self,
        operand: Option<&[u8]>,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
        if let Some(country) = operand
            .map(|value| self.evaluator()?.scoped_country(value, scopes))
            .transpose()?
            .flatten()
            .filter(|country| *country as i32 > 0)
        {
            return Ok(Some(country));
        }
        Ok(match scopes.this {
            Scope::Country(country) if country as i32 > 0 => Some(country),
            _ => None,
        })
    }

    // Own 14037AFB0; EFFECT_CAPTURE_OPERATIVE@142762440,
    // CAPTURED_BY@142762430, OPERATIVE@142762420. No appended LF.
    pub(in crate::effect_description) fn char_capture_operative(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.char_operative_action(
            item,
            scopes,
            "captured_by",
            b"EFFECT_CAPTURE_OPERATIVE",
            "CAPTURED_BY",
        )
    }

    // Own 1403834B0; EFFECT_KILL_OPERATIVE@142762568; 140327550 expands.
    pub(in crate::effect_description) fn char_kill_operative(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.char_operative_action(
            item,
            scopes,
            "killed_by",
            b"EFFECT_KILL_OPERATIVE",
            "KILLED_BY",
        )
    }

    // Own 140398B70; EFFECT_TURN_OPERATIVE@1427626E8; 140327550 expands.
    pub(in crate::effect_description) fn char_turn_operative(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.char_operative_action(
            item,
            scopes,
            "turned_by",
            b"EFFECT_TURN_OPERATIVE",
            "TURNED_BY",
        )
    }

    fn char_operative_action(
        &self,
        item: &Item,
        scopes: Scopes,
        country_key: &str,
        loc: &[u8],
        argument: &str,
    ) -> Result<String, String> {
        let (operative, country) = self.char_action_operands(item, scopes, country_key);
        let Some(operative) = self.char_action_operative(operative, scopes)? else {
            return Ok(String::new());
        };
        let Some(country) = self.char_action_country(country, scopes)? else {
            return Ok(String::new());
        };
        let name = self.effect_resolved_leader_name(operative)?;
        let mut country = self.country_name(country, true)?;
        if country_key == "captured_by" {
            // CountryTag_GetCapitalizedCountryName 140BA59B0 uses +0x50,
            // the definite name, then the shared native CRT case operation.
            country = installed_content::localisation::crt_case_text(&country, true, true);
        }
        self.char_native_text(
            loc,
            &[
                (argument, Argument::Text(&country)),
                ("OPERATIVE", Argument::Text(&name)),
            ],
            scopes,
            true,
        )
    }

    // Own 140380F70 queries 140FC7860's exact captured-reference roster.
    // OperativeMemberships has four other rosters; +0xFB0 is not membership.
    pub(in crate::effect_description) fn char_free_random_operative(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let captured_by = field(children(item), "captured_by").and_then(|item| item.value.scalar());
        let Some(_capturer) = captured_by
            .map(|value| self.evaluator()?.scoped_country(value, scopes))
            .transpose()?
            .flatten()
            .filter(|country| *country as i32 > 0)
        else {
            return Ok(String::new());
        };
        Err("captured operative references (CIntelligenceAgency+0x108/+0x114, 140FC7860) are unrecorded".into())
    }

    // CIntEffect reader 14137DD70 and evaluator 141384870 narrow to i32.
    fn char_operative_days(&self, item: &Item, scopes: Scopes) -> Result<i32, String> {
        let operand = item.value.scalar().unwrap_or_default();
        match installed_content::crt::scan_i32(operand).stored() {
            Some(days) => Ok(days),
            None => Ok((self.scoped_value(operand, scopes)? / 100_000) as i32),
        }
    }

    fn char_operative_codename_days(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
        newline: bool,
    ) -> Result<String, String> {
        let operative = self.effect_unit_leader(scopes)?;
        let days = self.char_operative_days(item, scopes)?;
        if days <= 0 {
            return Ok(String::new());
        }
        let Some(operative) = operative.filter(|leader| leader.category() == 3) else {
            return Ok(String::new());
        };
        let codename = self.effect_operative_codename(operative)?;
        let name = self.effect_resolved_leader_name(operative)?;
        let mut text = self.char_native_text(
            key,
            &[
                ("NAME", Argument::Text(&name)),
                ("CODENAME", Argument::Text(&codename)),
                ("DAYS", Argument::Int32(days)),
            ],
            scopes,
            true,
        )?;
        if newline {
            text.push('\n');
        }
        Ok(text)
    }

    // Own 140380AB0: EFFECT_FORCE_OPERATIVE_INTO_HIDING@142762C28,
    // NAME@142727E7C, CODENAME@142728860, DAYS@14275179C; no LF.
    pub(in crate::effect_description) fn char_hide_operative(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.char_operative_codename_days(
            item,
            scopes,
            b"EFFECT_FORCE_OPERATIVE_INTO_HIDING",
            false,
        )
    }

    // Own 140382790: EFFECT_HARM_OPERATIVE@142762AB8; same args, then LF.
    pub(in crate::effect_description) fn char_harm_operative(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.char_operative_codename_days(item, scopes, b"EFFECT_HARM_OPERATIVE", true)
    }

    // 14052B630 -> ProcessString 1412B9CA0 expands the raw authored bytes.
    fn char_event_tooltip(&self, item: &Item, scopes: Scopes) -> Result<Option<String>, String> {
        let Some(text) = field(children(item), "tooltip")
            .and_then(|item| item.value.read_string())
            .filter(|text| !installed_content::parser::c_string(text).is_empty())
        else {
            return Ok(None);
        };
        let text = std::str::from_utf8(installed_content::parser::c_string(text))
            .map_err(|_| "event tooltip is not UTF-8 (14052B630)")?;
        let mut text = self.expanded(text, scopes)?;
        text.push('\n');
        Ok(Some(text))
    }

    fn char_event(&self, item: &Item) -> Result<Arc<installed_content::events::Event>, String> {
        let id = item
            .value
            .scalar()
            .or_else(|| field(children(item), "id").and_then(|item| item.value.scalar()))
            .unwrap_or_default();
        let event = (self
            .context
            .content
            .events
            .ok_or("referenced event lookup unavailable")?)(id)?;
        if let Some(why) = &event.refused {
            return Err(why.clone());
        }
        Ok(event)
    }

    fn char_event_draw(&self, cursor: usize) -> Result<(u32, u32, Option<&'static str>), String> {
        let mut graph = self.graph.borrow_mut();
        let parent = graph
            .scopes
            .get_mut(cursor)
            .ok_or("event caller scope is not given")?;
        let seed = random::scoped_draw(parent.random_seed, parent.random_counter) as u32;
        parent.random_seed = parent.random_seed.wrapping_add(1);
        // 1402EE390/1402F1360 derive title lanes; preserve an unavailable input
        // until a title selection actually reads them (140A9CD70).
        Ok((
            random::scope_seed(seed),
            random::scope_counter(seed),
            parent.random_seed_refusal,
        ))
    }

    fn char_event_title(
        &self,
        event: &installed_content::events::Event,
        at: usize,
    ) -> Result<String, String> {
        let evaluator = self.evaluator()?;
        let selected = evaluator.event_scopes(at)?;
        match event
            .deterministic_title_key(|triggers| evaluator.evaluate(triggers, selected).decided())?
        {
            Some(key) => self.localised(key, &[], selected, true),
            None => Ok(String::new()),
        }
    }

    // Own 1402EE390: fresh default empty scope, seeded from caller +0x0C/+0x10;
    // FROM is caller ROOT. EVENT@142729920, wrapper@142729928, no expansion/LF.
    pub(in crate::effect_description) fn char_unit_leader_event(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        if let Some(text) = self.char_event_tooltip(item, scopes)? {
            return Ok(text);
        }
        let cursor = scopes
            .cursor
            .ok_or("unit_leader_event needs caller scope graph")?;
        let (seed, counter, refusal) = self.char_event_draw(cursor)?;
        let at = {
            let mut graph = self.graph.borrow_mut();
            let mut scope = EventScope::of(EventObject::Country(0));
            scope.random_seed = seed;
            scope.random_counter = counter;
            scope.random_seed_refusal = refusal;
            scope.from = match graph.scopes[cursor].root {
                Link::Itself => Link::Scope(cursor),
                link => link,
            };
            let at = graph.scopes.len();
            graph.scopes.push(scope);
            at
        };
        let event = self.char_event(item)?;
        let title = self.char_event_title(&event, at)?;
        self.char_native_text(
            b"EFFECT_COUNTRY_EVENT",
            &[("EVENT", Argument::Text(&title))],
            scopes,
            false,
        )
    }

    // Inherited own 141383540; title scopes 1402F1360 / 14137E4A0.
    // This wrapper's argument is EFFECT, unlike CUnitLeaderEventEffect's EVENT.
    pub(in crate::effect_description) fn char_operative_leader_event(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let event = self.char_event(item)?;
        if let Some(text) = self.char_event_tooltip(item, scopes)? {
            return Ok(text);
        }
        let at = self.char_operative_event_scope(item, scopes)?;
        let title = self.char_event_title(&event, at)?;
        self.char_native_text(
            b"EFFECT_COUNTRY_EVENT",
            &[("EFFECT", Argument::Text(&title))],
            scopes,
            false,
        )
    }

    fn char_operative_event_scope(&self, item: &Item, scopes: Scopes) -> Result<usize, String> {
        let at = {
            let mut graph = self.graph.borrow_mut();
            let at = graph.scopes.len();
            graph.scopes.push(EventScope::of(EventObject::Country(0)));
            at
        };
        let Some(leader) = self.effect_unit_leader(scopes)? else {
            return Ok(at);
        };
        let cursor = scopes
            .cursor
            .ok_or("operative_leader_event needs caller scope graph")?;
        let (seed, counter, refusal) = self.char_event_draw(cursor)?;
        // SetUnitLeader 14052EFD0 copies +0xF48, including a referenced role's
        // own character. That field is not present in CharacterOperatives.
        let character = match leader {
            UnitLeader::Direct(row) => row.character_id,
            UnitLeader::Referenced(row) => self
                .character_table::<tables::UnitLeaders>()?
                .iter()
                .find(|leader| {
                    (u64::from(leader.id) << 32) | u64::from(leader.reference_type) == row.reference
                })
                .map(|leader| leader.character_id)
                .ok_or_else(|| {
                    format!(
                        "referenced unit leader {} character (+0xF48) is unrecorded",
                        row.reference
                    )
                })?,
        };
        let owner = if leader.owner() as i32 > 0 {
            self.context
                .world()?
                .slot(leader.owner())
                .ok_or("CountryLinks lacks event caster owner")?
        } else {
            0
        };
        {
            let mut graph = self.graph.borrow_mut();
            let from = graph.scopes.len();
            graph
                .scopes
                .push(EventScope::of(EventObject::Country(owner)));
            graph.scopes[at].this = EventObject::Character(character);
            graph.scopes[at].random_seed = seed;
            graph.scopes[at].random_counter = counter;
            graph.scopes[at].random_seed_refusal = refusal;
            graph.scopes[at].from = Link::Scope(from);
        }
        for key in ["set_from", "set_from_from", "set_root"] {
            let Some(override_item) = field(children(item), key) else {
                continue;
            };
            // Native 14137E6AB evaluates +0x258 (set_from) for set_root,
            // despite its reader storing set_root's operand at +0x3F8.
            let operand = if key == "set_root" {
                field(children(item), "set_from").and_then(|item| item.value.scalar())
            } else {
                override_item.value.scalar()
            };
            let country = operand
                .map(|value| self.evaluator()?.scoped_country(value, scopes))
                .transpose()?
                .flatten()
                .unwrap_or(0);
            let mut graph = self.graph.borrow_mut();
            let new = graph.scopes.len();
            graph
                .scopes
                .push(EventScope::of(EventObject::Country(country)));
            match key {
                "set_from" => graph.scopes[at].from = Link::Scope(new),
                "set_from_from" => {
                    let from = match graph.scopes[at].from {
                        Link::Scope(from) => from,
                        _ => {
                            let from = graph.scopes.len();
                            graph.scopes.push(EventScope::of(EventObject::Country(0)));
                            graph.scopes[at].from = Link::Scope(from);
                            from
                        }
                    };
                    graph.scopes[from].from = Link::Scope(new);
                }
                _ => graph.scopes[at].root = Link::Scope(new),
            }
        }
        Ok(at)
    }
}
