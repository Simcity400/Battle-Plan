use dashboard_base::{Source, products::LoadContext, ranking::joined_spans, span::HourSpan};
use dashboard_query::{
    Query, identity::{Country, Holder, TAG_UNLINKED},
    series::{MissingSpan, ranked::{GroupMembership, GroupSelection}},
};
use dashboard_world::products::group_members::{UNKNOWN, dated};
use std::{collections::HashMap, io, sync::{Arc, atomic::AtomicBool}};

#[derive(Default)]
pub(super) struct Roster {
    slots: HashMap<u32, Vec<HourSpan>>,
    unknown: Vec<MissingSpan>,
    edges: Vec<i32>,
    pub(super) partial_from: Option<i32>,
}

impl Roster {
    pub(super) fn load(source: &Source, query: &Query<'_>, load: &LoadContext, group: &GroupSelection, cancelled: &AtomicBool) -> io::Result<Arc<Self>> {
        let cache = source.state::<HashMap<(u64, String), Arc<Self>>>();
        let key = (query.timeline.id, serde_json::to_string(group)?);
        if let Some(held) = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
            return Ok(Arc::clone(held));
        }
        let roster = Arc::new(Self::build(source, query, load, group, cancelled)?);
        Ok(Arc::clone(cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).entry(key).or_insert(roster)))
    }

    fn build(source: &Source, query: &Query<'_>, load: &LoadContext, group: &GroupSelection, cancelled: &AtomicBool) -> io::Result<Self> {
        let (first, last) = (query.timeline.first, query.timeline.last);
        let mut roster = Self::default();
        let selection = match dated(source, query, load, None, Some(group), cancelled) {
            Ok(Some(selection)) => selection,
            Err(ref error) if let Some(reason) = dashboard_core::refusal(error) => {
                roster.unknown.push(MissingSpan { first_hour: first, last_hour: last, reason: reason.to_owned() });
                return Ok(roster);
            }
            Err(error) => return Err(error),
            Ok(None) => return Err(dashboard_core::invalid("A group selection needs dated membership")),
        };
        let mut gaps = selection.unknown;
        for gap in selection.unobserved {
            let reason = query.holders.unknown_reason(gap.first_hour).map_or_else(|| UNKNOWN.to_owned(), |why| format!("Who holds the chosen countries is not known: {why}"));
            gaps.push(MissingSpan { first_hour: gap.first_hour, last_hour: gap.last_hour, reason });
        }
        for span in selection.membership {
            match span.country {
                Country::Slot(slot) => roster.slots.entry(slot).or_default().push(HourSpan { first_hour: span.first_hour, last_hour: span.last_hour }),
                Country::Tag(tag) => {
                    for held in query.holders.within(&tag, span.first_hour, span.last_hour) {
                        match held.holder {
                            Holder::Found { slot, .. } => roster.slots.entry(slot).or_default().push(HourSpan { first_hour: held.first, last_hour: held.last }),
                            Holder::Unresolved => gaps.push(MissingSpan { first_hour: held.first, last_hour: held.last, reason: format!("Who holds {tag} is not known: {TAG_UNLINKED}") }),
                        }
                    }
                }
            }
        }
        for spans in roster.slots.values_mut() {
            *spans = joined_spans(std::mem::take(spans), first, last);
            for span in spans {
                roster.edges.extend([span.first_hour, span.last_hour.saturating_add(1)]);
            }
        }
        gaps.sort_by_key(|gap| gap.first_hour);
        for mut gap in gaps {
            if group.membership == GroupMembership::Ever { gap.last_hour = last; }
            if let Some(previous) = roster.unknown.last_mut() {
                if previous.reason == gap.reason && gap.first_hour <= previous.last_hour.saturating_add(1) {
                    previous.last_hour = previous.last_hour.max(gap.last_hour);
                    continue;
                }
                if gap.last_hour <= previous.last_hour { continue; }
                gap.first_hour = gap.first_hour.max(previous.last_hour.saturating_add(1));
            }
            roster.unknown.push(gap);
        }
        roster.edges.extend(roster.unknown.iter().map(|gap| gap.first_hour));
        roster.edges.sort_unstable();
        roster.edges.dedup();
        roster.partial_from = super::battles_partial_from(query, first, last)?;
        Ok(roster)
    }

    pub(super) fn includes(&self, slot: u32, hour: i32) -> bool {
        self.slots.get(&slot).is_some_and(|spans| spans.get(spans.partition_point(|span| span.last_hour < hour)).is_some_and(|span| span.first_hour <= hour))
    }

    fn gap(&self, hour: i32) -> Option<&MissingSpan> {
        self.unknown.get(self.unknown.partition_point(|gap| gap.last_hour < hour)).filter(|gap| gap.first_hour <= hour)
    }

    pub(super) fn missing(&self, hour: i32) -> Option<&str> {
        self.gap(hour).map(|gap| gap.reason.as_str())
    }

    pub(super) fn through(&self, hour: i32) -> i32 {
        if let Some(gap) = self.gap(hour) { return gap.last_hour; }
        self.edges.get(self.edges.partition_point(|&edge| edge <= hour)).map_or(i32::MAX, |edge| edge - 1)
    }
}
