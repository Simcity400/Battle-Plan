
use super::*;
use crate::scripted_text::{EventObject, EventScope, Link, random};
use replay_data::rows::CapturedOperativeReferenceRow;

const CAPTURED_OWNER: u64 = mask::<CapturedOperativeReferenceRow>(&["owner"]);
const OPERATIVE_CHARACTER: u64 = mask::<UnitLeaderRow>(&["character_id"]);

#[cfg(test)]
mod tests;

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    fn char_action_operands<'a>(
        &self,
        item: &'a Item,
        scopes: Scopes,
        country_key: &str,
    ) -> (Option<&'a [u8]>, Option<&'a [u8]>) {
        let scalar = item.value.scalar();
        let character_scope = matches!(
            scopes.this,
            Scope::Character(_) | Scope::Leader(_) | Scope::CountryLeader { .. }
        );
        (
            field(children(item), "operative")
                .and_then(|item| item.value.scalar())
                .or(scalar.filter(|_| !character_scope)),
            field(children(item), country_key)
                .and_then(|item| item.value.scalar())
                .or(scalar.filter(|_| character_scope)),
        )
    }

    fn char_action_operative(
        &self,
        operand: Option<&[u8]>,
        scopes: Scopes,
    ) -> Result<Option<UnitLeader<'f>>, String> {
        let selected = self.effect_character_target(operand, scopes)?;
        let leader = match selected {
            Some(character) => Tables::read_in(self.tables())
                .unit_leader(character.id)
                .map_err(|why| why.to_string())?,
            None => None,
        };
        let leader = match leader {
            Some(leader) => Some(leader),
            None => self.effect_unit_leader(scopes)?,
        };
        Ok(leader.filter(|leader| leader.category() == 3))
    }

    fn char_action_country(
        &self,
        operand: Option<&[u8]>,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
        if let Some(country) = operand
            .map(|value| self.evaluator()?.scoped_country(value, scopes))
            .transpose()
            .map_err(|why| why.to_string())?
            .flatten()
            .filter(|country| *country as i32 > 0)
        {
            return Ok(Some(country));
        }
        Ok(match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. }
                if country as i32 > 0 =>
            {
                Some(country)
            }
            _ => None,
        })
    }

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
            country = content_reader_localisation::localisation::crt_case_text(&country, true, true);
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

    pub(in crate::effect_description) fn char_free_random_operative(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let captured_by = field(children(item), "captured_by").and_then(|item| item.value.scalar());
        let Some(capturer) = captured_by
            .map(|value| self.evaluator()?.scoped_country(value, scopes))
            .transpose()
            .map_err(|why| why.to_string())?
            .flatten()
            .filter(|country| *country as i32 > 0)
        else {
            return Ok(String::new());
        };
        let owner = self.current_country(scopes)?;
        let world = self.context.world()?;
        let mut found = false;
        let captured = self.complete::<tables::CapturedOperativeReferences>(CAPTURED_OWNER)?;
        for row in captured.keyed(|key| world.slot((key >> 32) as u32) == Some(capturer)) {
            let row = row.map_err(built)?;
            if let Some(raw) = row.owner {
                let country = if raw == 0 {
                    0
                } else {
                    world
                        .slot(raw)
                        .ok_or("country links lacks captured-operative owner")?
                };
                if country == owner {
                    found = true;
                    break;
                }
            }
        }
        if !found {
            return Ok(String::new());
        }
        let name = self.country_name(capturer, false)?;
        let name = format!("\u{11}Y{name}\u{11}!");
        self.char_native_text(
            if field(children(item), "all")
                .and_then(|item| item.value.scalar())
                .and_then(content_reader_parser::parser::read_bool)
                .unwrap_or(false)
            {
                b"EFFECT_FREE_RANDOM_OPERATIVE_ALL"
            } else {
                b"EFFECT_FREE_RANDOM_OPERATIVE"
            },
            &[("COUNTRY", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    fn char_operative_days(&self, item: &Item, scopes: Scopes) -> Result<i32, String> {
        let operand = item.value.scalar().unwrap_or_default();
        match content_reader_parser::crt::scan_i32(operand).stored() {
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
        let codename = self.effect_operative_codename(&operative)?;
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

    pub(in crate::effect_description) fn char_harm_operative(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.char_operative_codename_days(item, scopes, b"EFFECT_HARM_OPERATIVE", true)
    }

    fn char_event_tooltip(&self, item: &Item, scopes: Scopes) -> Result<Option<String>, String> {
        let Some(text) = field(children(item), "tooltip")
            .and_then(|item| item.value.read_string())
            .filter(|text| !content_reader_parser::parser::c_string(text).is_empty())
        else {
            return Ok(None);
        };
        let text = std::str::from_utf8(content_reader_parser::parser::c_string(text))
            .map_err(|_| "event tooltip is not UTF-8")?;
        let mut text = self.expanded(text, scopes)?;
        text.push('\n');
        Ok(Some(text))
    }

    fn char_event(&self, item: &Item) -> Result<Arc<content_reader_events::events::Event>, String> {
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
        Ok((
            random::scope_seed(seed),
            random::scope_counter(seed),
            parent.random_seed_refusal,
        ))
    }

    fn char_event_title(
        &self,
        event: &content_reader_events::events::Event,
        at: usize,
    ) -> Result<String, String> {
        let evaluator = self.evaluator()?;
        let selected = evaluator.event_scopes(at).map_err(|why| why.to_string())?;
        self.event_title(
            event.deterministic_title_key(|triggers| {
                evaluator.evaluate(triggers, selected).decided()
            })?,
            selected,
        )
    }

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
        let character = match &leader {
            UnitLeader::Direct(row) => row.character_id,
            UnitLeader::Referenced(row) => keyed(
                &self.character_leaders(OPERATIVE_CHARACTER)?,
                u128::from(row.reference),
            )?
            .map(|leader| leader.character_id)
                .ok_or_else(|| {
                    format!(
                        "referenced unit leader {} character is unrecorded",
                        row.reference
                    )
                })?,
        };
        let owner = if leader.owner() as i32 > 0 {
            self.context
                .world()?
                .slot(leader.owner())
                .ok_or("country links lacks event caster owner")?
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
            let operand = if key == "set_root" {
                field(children(item), "set_from").and_then(|item| item.value.scalar())
            } else {
                override_item.value.scalar()
            };
            let country = operand
                .map(|value| self.evaluator()?.scoped_country(value, scopes))
                .transpose()
                .map_err(|why| why.to_string())?
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
