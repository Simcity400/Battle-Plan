//! CEffect::BuildDescription (140532780), shared by event and balance tooltips.
//!
//! Stable domain entry points (core changes go through lead I):
//! - `Renderer::{line, localised, expanded}` localise keys/arguments and expand in
//!   the caller's scope/session (1410CF560); `line` appends LF.
//! - `Renderer::{country_target, state_target, current_country, country_name, tag}`
//!   resolve scoped operands and names; `state_name` uses GetStateName (1409CC380).
//! - `Renderer::evaluator` is the shared trigger/variable/saved-target evaluator;
//!   `scoped_value` evaluates fixed5 operands (1405382B0).
//! - `Renderer::modifier_list`, `crate::modifier_text::value_text`, localisation
//!   `Argument`, and `installed_content::fixed5::{fixed5_to_string, fixed5_percent}` retain
//!   native modifier/number formatting (14054FE50, 142230F20).
//! - `Renderer::group`, `append_description`, `indent_nonempty_lines`,
//!   `trim_description`, and `Context::{with_variable_session,
//!   with_expansion_scopes, variable_session}` retain indentation/session rules.
//! - `Renderer::refuse(class, reason)` counts a named failure; the class-keyed
//!   dispatch is in `dispatch.rs`. Each lead edits only its own domain and rows.
use std::{
    cell::{OnceCell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    sync::Arc,
};

use crate::triggers::{NULL_STATE, Scope};
use crate::{
    scripted_text::{EventScopes, ExpansionScopes, ScriptedText},
    triggers::{Evaluator, Scopes, World},
};
use installed_content::{
    localisation::{Argument, Localisation},
    parser::Item,
};

mod characters {}
mod diplomacy;
mod dispatch;
mod empty;
mod events;
mod lists;
mod modifiers;
mod numeric;
mod politics {}
mod scopes;
mod states;
mod technology;
#[cfg(test)]
mod tests;
mod units;
mod variables;

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Description {
    pub text: String,
    pub refusals: BTreeMap<String, usize>,
}

/// Resolves an event id to its parsed event, or why it cannot be.
pub type EventLookup<'a> = dyn Fn(&[u8]) -> Result<Arc<installed_content::events::Event>, String> + 'a;

/// Content already cached by the caller. Referenced events remain lazy.
#[derive(Default, Clone, Copy)]
pub struct Content<'a> {
    pub doctrines: Option<&'a installed_content::doctrines::Database>,
    pub decisions: Option<&'a installed_content::decisions::Database>,
    pub ideas: Option<&'a installed_content::ideas::IdeaContent>,
    pub name_lists: Option<&'a installed_content::names::NameLists>,
    pub events: Option<&'a EventLookup<'a>>,
    pub key_names: Option<&'a installed_content::key_names::KeyNames>,
    pub gaps: Option<&'a BTreeMap<&'static str, String>>,
}

/// Per recorded content stack. Event bodies use the caller's lazy ID cache.
pub struct CachedContent {
    doctrines: Option<installed_content::doctrines::Database>,
    decisions: Option<installed_content::decisions::Database>,
    ideas: Option<installed_content::ideas::IdeaContent>,
    name_lists: Option<installed_content::names::NameLists>,
    key_names: Option<installed_content::key_names::KeyNames>,
    gaps: BTreeMap<&'static str, String>,
    pub notes: Vec<String>,
}

impl CachedContent {
    pub fn read(layers: &installed_content::ResolvedLayers) -> std::io::Result<Self> {
        fn keep<T>(
            read: std::io::Result<T>,
            key: &'static str,
            gaps: &mut BTreeMap<&'static str, String>,
        ) -> Option<T> {
            read.map_err(|why| {
                gaps.insert(key, why.to_string());
            })
            .ok()
        }
        let mut gaps = BTreeMap::new();
        let doctrines = keep(installed_content::doctrines::read(layers), "doctrines", &mut gaps);
        let decisions = keep(installed_content::decisions::read(layers), "decisions", &mut gaps);
        let read_ideas = || {
            let officers = installed_content::defines::read(layers)?
                .strings("NCharacter", "OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU")?;
            installed_content::ideas::read(layers, &officers)
        };
        let ideas = keep(read_ideas(), "ideas", &mut gaps);
        let mut notes = doctrines
            .as_ref()
            .map(|db| db.notes.clone())
            .unwrap_or_default();
        notes.extend(
            decisions
                .as_ref()
                .map(|db| db.notes.clone())
                .unwrap_or_default(),
        );
        let ideas = ideas.map(|(ideas, _, warnings)| {
            notes.extend(warnings.into_iter().map(|(_, why)| why));
            ideas
        });
        let name_lists = keep(installed_content::names::read(layers), "name lists", &mut gaps);
        let key_names = keep(
            installed_content::key_names::KeyNames::read(layers),
            "effect key names",
            &mut gaps,
        );
        Ok(Self {
            doctrines,
            decisions,
            ideas,
            name_lists,
            key_names,
            gaps,
            notes,
        })
    }

    pub fn view<'a>(&'a self, events: Option<&'a EventLookup<'a>>) -> Content<'a> {
        Content {
            doctrines: self.doctrines.as_ref(),
            decisions: self.decisions.as_ref(),
            ideas: self.ideas.as_ref(),
            name_lists: self.name_lists.as_ref(),
            key_names: self.key_names.as_ref(),
            gaps: Some(&self.gaps),
            events,
        }
    }
}

impl crate::scripted_text::ScriptedContent {
    pub fn effects(&self) -> Result<&CachedContent, String> {
        self.effects
            .get_or_init(|| {
                CachedContent::read(&self.layers.layers)
                    .map_err(|why| format!("effect description content: {why}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// One hour's immutable inputs; the mutable event graph belongs to its caller.
pub struct Context<'c, 'f> {
    text: &'c ScriptedText<'f>,
    localisation: &'c Localisation,
    session: RefCell<Option<crate::triggers::VariableSession>>,
    nested_session: bool,
    expansion: Option<&'c ExpansionScopes>,
    perspective: u32,
    source: &'c str,
    content: Content<'c>,
}

impl<'c, 'f> Context<'c, 'f> {
    pub fn new(
        text: &'c ScriptedText<'f>,
        localisation: &'c Localisation,
        perspective: u32,
        source: &'c str,
        content: Content<'c>,
    ) -> Self {
        Self {
            text,
            localisation,
            session: RefCell::new(None),
            nested_session: false,
            expansion: None,
            perspective,
            source,
            content,
        }
    }

    fn world(&self) -> Result<&'c World<'f>, String> {
        self.text.world()
    }

    pub(crate) fn with_variable_session(
        mut self,
        session: &crate::triggers::VariableSession,
    ) -> Self {
        self.session.replace(Some(session.clone()));
        self.nested_session = true;
        self
    }

    pub(crate) fn with_expansion_scopes(mut self, scopes: &'c ExpansionScopes) -> Self {
        self.expansion = Some(scopes);
        self
    }

    pub(crate) fn variable_session(&self) -> Option<crate::triggers::VariableSession> {
        self.session.borrow().clone()
    }
}

/// Each call is its own native variable/description session. Cleanup follows FROM;
/// targets on a distinct ROOT endpoint may survive into the next call (140529520).
pub fn describe(
    items: &[Item],
    context: &Context<'_, '_>,
    scopes: &mut EventScopes,
) -> Description {
    if !context.nested_session {
        context.session.replace(None);
    }
    let graph = Rc::new(RefCell::new(scopes.clone()));
    let mut out = Renderer {
        context,
        graph,
        evaluator: OnceCell::new(),
        out: Description::default(),
    };
    let projected = crate::triggers::event_scopes::project(&out.graph.borrow(), 0);
    match projected {
        Ok(scopes) => {
            out.out.text = out.group(items, scopes, 0);
            if let Some(Ok(evaluator)) = out.evaluator.get() {
                let result = evaluator.end_description(scopes);
                context.session.replace(Some(evaluator.variable_session()));
                if let Err(why) = result {
                    out.refuse("description cleanup", why);
                }
            } else if !context.nested_session {
                let result = crate::triggers::event_scopes::cleanup_description(
                    &mut out.graph.borrow_mut(),
                    0,
                );
                if let Err(why) = result {
                    out.refuse("description cleanup", why);
                }
            }
        }
        Err(why) => out.refuse("event scope", why),
    }
    *scopes = out.graph.borrow().clone();
    out.out
}

struct Renderer<'r, 'c, 'f> {
    context: &'r Context<'c, 'f>,
    graph: Rc<RefCell<EventScopes>>,
    evaluator: OnceCell<Result<Evaluator<'c, 'f>, String>>,
    out: Description,
}

/// 1424B7400 trims exactly these four ASCII bytes.
pub fn trim_description(text: &str) -> &str {
    text.trim_matches(['\n', '\t', '\r', ' '])
}

/// 1424B7850 prefixes one space per level, leaving empty lines untouched.
pub(super) fn indent_nonempty_lines(text: &str, indent: usize) -> String {
    let prefix = " ".repeat(indent);
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        if line != "\n" {
            out.push_str(&prefix);
        }
        out.push_str(line);
    }
    out
}

pub(super) fn append_description(text: &mut String, child: &str) {
    if child.len() as i32 <= 1 {
        return;
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(child);
}

fn field<'a>(items: &'a [Item], name: &str) -> Option<&'a Item> {
    items.iter().rev().find(|item| item.keyword_is(name))
}

fn children(item: &Item) -> &[Item] {
    item.value.items().unwrap_or_default()
}
fn key(item: &Item) -> String {
    String::from_utf8_lossy(item.key.as_deref().unwrap_or_default()).into_owned()
}

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    pub(super) fn tag(&self, country: u32) -> Result<&str, String> {
        self.context
            .text
            .recorded()
            .tag(country)
            .ok_or_else(|| format!("Countries lacks tag for country {country}"))
    }

    pub(super) fn country_name(&self, country: u32, definite: bool) -> Result<String, String> {
        self.context.text.country_name(country, definite)
    }

    pub(super) fn current_country(&self, scopes: Scopes) -> Result<u32, String> {
        match scopes.this {
            Scope::Country(country) => Ok(country),
            _ => Err("current scope has no country (1405336C0)".into()),
        }
    }

    pub(super) fn country_target(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
        let operand = item
            .value
            .scalar()
            .or_else(|| field(children(item), "target").and_then(|field| field.value.scalar()));
        match operand {
            Some(value) => self.evaluator()?.scoped_country(value, scopes),
            None => Ok(match scopes.this {
                Scope::Country(country) => Some(country),
                _ => None,
            }),
        }
    }

    pub(super) fn state_target(&self, item: &Item, scopes: Scopes) -> Result<u32, String> {
        let state = match item.value.scalar() {
            Some(value) => self.evaluator()?.scoped_state_id(value, scopes)? as u32,
            None => 0,
        };
        if state == 0 {
            Ok(match scopes.this {
                Scope::State(state) => state,
                _ => NULL_STATE,
            })
        } else {
            Ok(state)
        }
    }

    pub(super) fn line(
        &self,
        key: &[u8],
        args: &[(&str, Argument<'_>)],
        scopes: Scopes,
        expand: bool,
    ) -> Result<String, String> {
        let mut text = self.localised(key, args, scopes, expand)?;
        text.push('\n');
        Ok(text)
    }

    pub(super) fn state_name(&self, state: u32) -> Result<String, String> {
        self.context.text.state_name(state) // GetStateName@1409CC380.
    }

    pub(super) fn scoped_value(&self, operand: &[u8], scopes: Scopes) -> Result<i64, String> {
        self.evaluator()?.scoped_value(operand, scopes) // CScopedVariable::Evaluate@1405382B0.
    }

    pub(super) fn evaluator(&self) -> Result<&Evaluator<'c, 'f>, String> {
        self.evaluator
            .get_or_init(|| {
                let mut evaluator = Evaluator::new(self.context.world()?, None)
                    .with_event_scopes(Rc::clone(&self.graph));
                if let Some(session) = self.context.session.borrow().as_ref() {
                    evaluator = evaluator.with_variable_session(session);
                }
                evaluator.begin_description();
                Ok(evaluator)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    pub(super) fn refuse(&mut self, key: &str, why: String) {
        *self
            .out
            .refusals
            .entry(format!("{} effect {key}: {why}", self.context.source))
            .or_default() += 1;
    }

    pub(super) fn expanded(&self, text: &str, scopes: Scopes) -> Result<String, String> {
        let session = self
            .evaluator
            .get()
            .and_then(|result| result.as_ref().ok())
            .map(Evaluator::variable_session)
            .or_else(|| self.context.variable_session());
        let (result, session) = self.context.text.expand_in_description(
            text,
            &self.graph.borrow(),
            scopes.cursor.unwrap_or(0),
            session,
            self.context.expansion,
        );
        if let Some(session) = &session
            && let Some(Ok(evaluator)) = self.evaluator.get()
        {
            evaluator.restore_variable_session(session);
        }
        self.context.session.replace(session);
        result
    }

    fn content_gap(&self, name: &'static str) -> String {
        self.context
            .content
            .gaps
            .and_then(|gaps| gaps.get(name))
            .map(|why| format!("{name} content unavailable: {why}"))
            .unwrap_or_else(|| format!("{name} content unavailable"))
    }

    pub(super) fn localised(
        &self,
        key: &[u8],
        args: &[(&str, Argument<'_>)],
        scopes: Scopes,
        expand: bool,
    ) -> Result<String, String> {
        let text = self
            .context
            .localisation
            .localise_with(key, args)
            .map_err(|why| why.to_string())?;
        if expand {
            self.expanded(&text, scopes)
        } else {
            Ok(text)
        }
    }

    fn own(&mut self, item: &Item, scopes: Scopes) -> String {
        match self.leaf(item, scopes) {
            Ok(text) => text,
            Err(why) => {
                let class = self
                    .effect_class(item)
                    .map(|class| class.name)
                    .unwrap_or("unknown key");
                self.refuse(class, format!("key {}: {why}", key(item)));
                String::new()
            }
        }
    }

    fn builder(&mut self, item: &Item, scopes: Scopes, indent: usize) -> String {
        match self.effect_class(item).map(|class| class.vtable) {
            Some(0x1427576E8) /* CEveryStateEffect */ => return self.every_state(item, scopes, indent),
            Some(0x14299F6F8) /* CContextEffect */ => return self.context_effect(item, scopes, indent),
            Some(0x14299FD88) /* CEffectTooltipEffect */ => return self.tooltip_group(item, scopes, indent),
            Some(0x14299FCB8) /* CHiddenEffect */ => return self.hidden_group(item, scopes, indent),
            _ => {}
        }
        let own = self.own(item, scopes);
        if own.len() as i32 <= 1 {
            return String::new();
        }
        // vf14 and generic own-line prefix, 140532780. Custom event desc differs.
        if self.indents(item) {
            format!("{}{own}", "   ".repeat(indent))
        } else {
            own
        }
    }

    fn indents(&self, item: &Item) -> bool {
        // Context vf14 is needed here for its parent's first-child header clear.
        // The builder returns context descriptions before its own-line prefix.
        // CIfEffect vf14 is ReturnTrue@1401802A0 (1427BE590 + 0x70).
        !(self.is_context(item)
            || item.keyword_is("country_event")
                && field(children(item), "desc")
                    .and_then(|item| item.value.read_string())
                    .is_some_and(|desc| !desc.is_empty()))
    }

    pub(super) fn group(&mut self, items: &[Item], scopes: Scopes, indent: usize) -> String {
        let mut text = String::new();
        for (item, branches) in self.attached(items, true) {
            let line = if item.keyword_is("if") {
                self.if_chain(item, &branches, scopes, indent, false)
                    .unwrap_or_default()
            } else {
                self.builder(item, scopes, indent)
            };
            append_description(&mut text, &line);
        }
        text
    }
}
