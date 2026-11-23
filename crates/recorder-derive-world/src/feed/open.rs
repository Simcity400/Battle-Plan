
use super::invalid;
use recorder_derive_framework::framework::{Rows, Value};
use replay_columnar::store::{TableView, Value as Held};
use replay_data::{
    event_feed_rows::{EventFeedEvent, EventFeedSource as Source},
    rows::TableId,
};
use std::{collections::HashMap, io};

type Fact = (u32, Box<[u8]>, Option<u32>, Option<i32>);

#[derive(Default)]
pub(super) struct Links {
    pub(super) generation: Option<u64>,
    pub(super) read: Option<i32>,
    pub(super) covered: bool,
}

fn event_key(kind: u32, id: u32, instance: u32, receiver: u32) -> u128 {
    u128::from(kind) << 96 | u128::from(id) << 64 | u128::from(instance) << 32 | u128::from(receiver)
}

fn number(view: &TableView<'_>, column: usize, slot: u32) -> io::Result<u64> {
    match view.value(column, slot) {
        Held::Number(bits) => Ok(bits),
        _ => Err(invalid("An open card's row does not hold every cell")),
    }
}

fn optional(view: &TableView<'_>, column: usize, slot: u32) -> io::Result<Option<u64>> {
    match view.value(column, slot) {
        Held::Number(bits) => Ok(Some(bits)),
        Held::Missing => Ok(None),
        _ => Err(invalid("An open card's row does not hold every cell")),
    }
}

#[derive(Default)]
pub(super) struct Open {
    events: HashMap<u128, Option<(u64, u64, i32)>>,
    decisions: HashMap<u128, Option<(Fact, i32)>>,
    index: Option<HashMap<Fact, (u128, i32)>>,
}

impl Open {
    fn event(&self, out: &Rows<'_>, key: u128) -> io::Result<Option<(u64, u64, i32)>> {
        if let Some(open) = self.events.get(&key) {
            return Ok(*open);
        }
        let view = out.table(TableId::FeedOpenEvents);
        let Some(slot) = view.slot(key) else { return Ok(None) };
        Ok(Some((number(&view, 0, slot)?, number(&view, 1, slot)?, number(&view, 2, slot)? as i32)))
    }

    fn index(&mut self, out: &Rows<'_>) -> io::Result<&mut HashMap<Fact, (u128, i32)>> {
        if self.index.is_none() {
            let view = out.table(TableId::FeedOpenDecisions);
            let mut index = HashMap::new();
            for slot in view.rows() {
                let key = view.key(slot).ok_or_else(|| invalid("An open decision's slot holds no row"))?;
                let token = view.text(1, slot).and_then(|id| view.string(id)).ok_or_else(|| invalid("An open decision's token is not held"))?;
                let fact = (number(&view, 0, slot)? as u32, token.into(), optional(&view, 2, slot)?.map(|target| target as u32), optional(&view, 3, slot)?.map(|numeric| numeric as i64 as i32));
                index.insert(fact, (key, number(&view, 4, slot)? as i32));
            }
            self.index = Some(index);
        }
        Ok(self.index.as_mut().expect("the index was just read"))
    }

    fn clear_events(&mut self, out: &Rows<'_>) {
        let view = out.table(TableId::FeedOpenEvents);
        for key in view.rows().filter_map(|slot| view.key(slot)) {
            self.events.insert(key, None);
        }
        self.events.values_mut().for_each(|open| *open = None);
    }

    fn clear_decisions(&mut self, out: &Rows<'_>) -> io::Result<()> {
        let open = std::mem::take(self.index(out)?);
        for (_, (key, _)) in open {
            self.decisions.insert(key, None);
        }
        Ok(())
    }

    fn open_decision(&mut self, out: &Rows<'_>, key: u128, fact: Fact, read: i32) -> io::Result<()> {
        if let Some((replaced, _)) = self.index(out)?.insert(fact.clone(), (key, read)) {
            self.decisions.insert(replaced, None);
        }
        self.decisions.insert(key, Some((fact, read)));
        Ok(())
    }

    pub(super) fn observe_read(&mut self, out: &Rows<'_>, links: &mut Links, read: i32, complete: bool) -> io::Result<()> {
        if !complete || links.read.and_then(|last| last.checked_add(1)) != Some(read) {
            self.clear_decisions(out)?;
        }
        links.read = Some(read);
        links.covered = complete;
        Ok(())
    }

    pub(super) fn link(&mut self, out: &Rows<'_>, links: &mut Links, event: &EventFeedEvent, token: Option<&[u8]>, read: i32) -> io::Result<Option<(u64, i32)>> {
        if links.generation != Some(event.generation) {
            if links.generation.is_some() {
                self.clear_events(out);
                self.clear_decisions(out)?;
            }
            links.generation = Some(event.generation);
        }
        let Some(script) = event.script else { return Ok(None) };
        let receiver = match event.source {
            Source::ScriptEvent | Source::NewsEvent => event.target.id,
            _ => event.actor.id,
        };
        let dispatch = script.native_event.zip(script.instance).map(|(native, instance)| event_key(native.kind, native.id, instance, receiver));
        let decision = token.filter(|token| !token.is_empty() && links.covered).map(|token| -> Fact { (event.actor.id, token.into(), script.country_target, script.numeric_target) });
        let own = u128::from(event.generation) << 64 | u128::from(event.sequence);
        Ok(match event.source {
            Source::ScriptEvent | Source::NewsEvent => {
                if let Some(key) = dispatch {
                    self.events.insert(key, Some((event.generation, event.sequence, read)));
                }
                None
            }
            Source::EventOption => {
                let Some(key) = dispatch else { return Ok(None) };
                let open = self.event(out, key)?;
                if open.is_some() && !script.option.is_some_and(|ordinal| ordinal < 0) {
                    self.events.insert(key, None);
                }
                open.map(|(_, sequence, opened)| (sequence, opened))
            }
            Source::DecisionSelected => {
                if let Some(fact) = decision {
                    self.open_decision(out, own, fact, read)?;
                }
                None
            }
            Source::DecisionActivated
            | Source::DecisionDurationChanged
            | Source::DecisionRemovalEffect
            | Source::DecisionCancellationEffect
            | Source::DecisionCompletionEffect
            | Source::DecisionTimeoutEffect
            | Source::DecisionCompleted
            | Source::DecisionCancelled
            | Source::DecisionTimedOut
            | Source::DecisionRemoved => {
                let Some(fact) = decision else { return Ok(None) };
                match self.index(out)?.get(&fact).copied() {
                    Some((key, opened)) => {
                        if matches!(event.source, Source::DecisionCompleted | Source::DecisionCancelled | Source::DecisionTimedOut | Source::DecisionRemoved) {
                            self.index(out)?.remove(&fact);
                            self.decisions.insert(key, None);
                        }
                        Some((key as u64, opened))
                    }
                    None => {
                        if event.source == Source::DecisionActivated {
                            self.open_decision(out, own, fact, read)?;
                        }
                        None
                    }
                }
            }
            _ => None,
        })
    }

    pub(super) fn flush(self, out: &mut Rows<'_>) {
        let mut events: Vec<_> = self.events.into_iter().collect();
        events.sort_unstable_by_key(|(key, _)| *key);
        for (key, open) in events {
            match open {
                Some((generation, sequence, read)) => out.set(TableId::FeedOpenEvents, key, vec![Value::U64(generation), Value::U64(sequence), Value::I32(read)]),
                None => out.delete(TableId::FeedOpenEvents, key),
            }
        }
        let mut decisions: Vec<_> = self.decisions.into_iter().collect();
        decisions.sort_unstable_by_key(|(key, _)| *key);
        for (key, open) in decisions {
            match open {
                Some(((country, token, target, numeric), read)) => {
                    let token = out.text(&token);
                    out.set(TableId::FeedOpenDecisions, key, vec![Value::U32(country), token, Value::OptionalU32(target), Value::OptionalI32(numeric), Value::I32(read)]);
                }
                None => out.delete(TableId::FeedOpenDecisions, key),
            }
        }
    }
}
