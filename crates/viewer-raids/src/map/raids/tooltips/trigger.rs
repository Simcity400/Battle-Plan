use super::super::rules::Lines;
use super::*;
use content_reader_core::parser::Item;
use dashboard_text::triggers::{
    Evaluator,
    description::{self, Host},
};

struct RaidHost<'c, 'a, 'w> {
    context: &'c Context<'a, 'w>,
}

impl<'a> Host<'a> for RaidHost<'_, 'a, '_> {
    fn country_name(&self, country: u32) -> Result<String> {
        self.context.countries.name(country)
    }

    fn country_tag(&self, country: u32) -> Result<String> {
        Ok(self
            .context
            .countries
            .operands(country)
            .ok_or("trigger country naming operands absent")?
            .as_ref()
            .map_err(Clone::clone)?
            .tag
            .clone())
    }

    fn state_name(&self, state: u32) -> Result<String> {
        self.context.state_name(state)
    }

    fn leader_name(&self, reference: u64) -> Result<String> {
        self.context.leader_name(reference)
    }

    fn scripted_text(&self) -> Result<&dashboard_text::scripted_text::ScriptedText<'a>> {
        self.context.scripted_text()
    }

    fn expand(
        &self,
        text: &str,
        scopes: Scopes,
        evaluator: &Evaluator<'_, 'a>,
    ) -> Result<String> {
        let scripted = self.context.scripted_text()?;
        let graph = event_scopes(scopes);
        scripted.expand_in_session(text, &graph, 0, evaluator)
    }

    fn project_name(&self, name: &str, country: u32) -> Result<String> {
        self.context.project_name(name, country)
    }
}

impl Context<'_, '_> {
    pub(super) fn trigger_text(&self, triggers: &[Item], scopes: Scopes, name: &str) -> Lines {
        let graph = std::rc::Rc::new(std::cell::RefCell::new(event_scopes(scopes)));
        let evaluator = Evaluator::new(self.world, None).with_event_scopes(graph);
        let start = match evaluator.event_scopes(0) {
            Ok(projected) => Scopes {
                bare: scopes.bare,
                ..projected
            },
            Err(_) => scopes,
        };
        let host = RaidHost { context: self };
        description::describe_lines(triggers, &evaluator, &host, self.loc, start, 0, 1)
            .into_iter()
            .map(|(key, text)| (format!("{name}: {key}"), text))
            .collect()
    }

    fn leader_name(&self, reference: u64) -> Result<String> {
        let leaders = self
            .world
            .frame()
            .complete::<tables::UnitLeaders>()
            .map_err(|e| e.to_string())?;
        let leader = leaders
            .iter()
            .find(|r| (u64::from(r.id) << 32 | u64::from(r.reference_type)) == reference)
            .ok_or_else(|| format!("unit leader {reference:#x} is absent from UnitLeaders"))?;
        if leader.inherited_text_a.is_empty() {
            return Err(format!("unit leader {reference:#x} has no recorded name"));
        }
        localise(
            self.loc,
            std::str::from_utf8(&leader.inherited_text_a).map_err(|e| e.to_string())?,
            &[],
        )
    }

    fn project_name(&self, name: &str, country: u32) -> Result<String> {
        let token = self
            .world
            .token_of(name.as_bytes())
            .map_err(|why| why.to_string())?;
        let rows = self
            .world
            .frame()
            .complete::<tables::Projects>()
            .map_err(|e| e.to_string())?;
        if token.is_none()
            || !rows.iter().any(|row| {
                Some(row.definition_id) == token && self.world.slot(row.owner) == Some(country)
            })
        {
            return Ok(String::new());
        }
        self.project_tooltip_name(name, country)
    }
}
