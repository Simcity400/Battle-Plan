//! Each faction's goal book: its lists and labels in first-seen order, which its goal lanes
//! number; and the definitions a reply shows of them.
use super::*;
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(in crate::products::faction_state) struct GoalList {
    pub absent: bool,
    pub reason: Option<String>,
    /// Ordinal, label and native status of each runtime goal.
    pub goals: Vec<(u32, u32, i32)>,
    pub completed: Vec<u32>,
    pub cancelled: Vec<u32>,
}
impl GoalList {
    pub fn absent() -> Self {
        Self {
            absent: true,
            ..Default::default()
        }
    }
}
/// Values in first-seen order, found again by their hash.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub(in crate::products::faction_state) struct Interned<T> {
    values: Vec<T>,
    #[serde(skip)]
    index: HashMap<u64, Vec<u32>>,
    #[serde(skip)]
    indexed: usize,
}
fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
impl<T: Hash + Eq> Interned<T> {
    pub fn get(&self, index: u32) -> Option<&T> {
        self.values.get(index as usize)
    }
    /// Its index, and whether it is new.
    pub fn intern(&mut self, value: T) -> (u32, bool) {
        // A restored book holds no index: build it once.
        if self.indexed != self.values.len() {
            self.index.clear();
            for (at, held) in self.values.iter().enumerate() {
                self.index.entry(hash_of(held)).or_default().push(at as u32);
            }
            self.indexed = self.values.len();
        }
        let hash = hash_of(&value);
        if let Some(&at) = self.index.get(&hash).and_then(|candidates| {
            candidates
                .iter()
                .find(|at| self.values[**at as usize] == value)
        }) {
            return (at, false);
        }
        let at = self.values.len() as u32;
        self.values.push(value);
        self.index.entry(hash).or_default().push(at);
        self.indexed += 1;
        (at, true)
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub(in crate::products::faction_state) struct GoalBook {
    pub labels: Interned<GoalLabel>,
    pub lists: Interned<GoalList>,
}
/// The list a reply synthesizes for hours a complete roster lacks the faction.
pub(in crate::products::faction_state) const ABSENT: u32 = u32::MAX;
/// The JSON a reply grows by, refused once it would pass the transport budget.
#[derive(Default)]
pub(in crate::products::faction_state) struct Budget(usize);
impl Budget {
    pub fn add<T: Serialize>(&mut self, value: &T) -> io::Result<()> {
        self.0 += serde_json::to_vec(value)?.len() + 1;
        if self.0 > dashboard_base::products::MAX_PRODUCT_REPLY_BYTES {
            return Err(io::Error::other(format!(
                "This product reply exceeds the transport reply budget of {} bytes",
                dashboard_base::products::MAX_PRODUCT_REPLY_BYTES
            )));
        }
        Ok(())
    }
}
/// Ordered disjoint spans of `spans` outside every span of `held`.
pub(in crate::products::faction_state) fn outside(spans: &[HourSpan], held: &[HourSpan]) -> Vec<HourSpan> {
    let mut result = Vec::new();
    for span in spans {
        let mut start = i64::from(span.first_hour);
        for gap in held {
            if i64::from(gap.last_hour) < start || gap.first_hour > span.last_hour {
                continue;
            }
            if start < i64::from(gap.first_hour) {
                result.push(HourSpan {
                    first_hour: start as i32,
                    last_hour: gap.first_hour - 1,
                });
            }
            start = start.max(i64::from(gap.last_hour) + 1);
        }
        if start <= i64::from(span.last_hour) {
            result.push(HourSpan {
                first_hour: start as i32,
                last_hour: span.last_hour,
            });
        }
    }
    result
}
/// The reply's lists in shown order, each label they name drawn once as faction details draw it.
pub(in crate::products::faction_state) fn definitions(
    source: &Source,
    context: &LoadContext,
    book: &GoalBook,
    lists: Vec<(u32, u32)>,
    budget: &mut Budget,
    reply: &mut FactionGoalsReply,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let cat = match context
        .content
        .as_ref()
        .map(|options| catalogue::catalogue(source, options))
        .transpose()
    {
        Ok(cat) => cat,
        Err(reason) => {
            reply
                .warnings
                .push(format!("Faction goal content: {reason}"));
            None
        }
    };
    if let Some(cat) = cat.as_deref() {
        reply
            .warnings
            .extend(faction_details::goal_content_warnings(cat));
    }
    budget.add(&reply.warnings)?;
    let mut labels = BTreeMap::<u32, u32>::new();
    let absent = GoalList::absent();
    for (list, _) in lists {
        check_cancel(cancelled)?;
        let held = if list == ABSENT {
            &absent
        } else {
            book.lists
                .get(list)
                .ok_or_else(|| dashboard_base::invalid("Faction goal list is unavailable"))?
        };
        let mut label = |index: u32| -> io::Result<u32> {
            if let Some(&shown) = labels.get(&index) {
                return Ok(shown);
            }
            check_cancel(cancelled)?;
            let found = book
                .labels
                .get(index)
                .ok_or_else(|| dashboard_base::invalid("Faction goal label is unavailable"))?;
            let definition = faction_details::finish_goal(found, cat.as_deref());
            budget.add(&definition)?;
            let shown = reply.definitions.len() as u32;
            reply.definitions.push(definition);
            labels.insert(index, shown);
            Ok(shown)
        };
        let mut shown = FactionGoalList {
            absent: held.absent,
            reason: held.reason.clone(),
            goals: Vec::new(),
            completed: Vec::new(),
            cancelled: Vec::new(),
        };
        for &(ordinal, index, status) in &held.goals {
            shown.goals.push(FactionHeldGoal {
                ordinal,
                definition: label(index)?,
                status,
            });
        }
        for &index in &held.completed {
            shown.completed.push(label(index)?);
        }
        for &index in &held.cancelled {
            shown.cancelled.push(label(index)?);
        }
        budget.add(&shown)?;
        reply.lists.push(shown);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_restored_book_finds_its_values_again_in_first_seen_order() {
        let mut book = GoalBook::default();
        let label = |name: &str| GoalLabel {
            token: Some(name.into()),
            ..Default::default()
        };
        assert_eq!(book.labels.intern(label("a")), (0, true));
        assert_eq!(book.labels.intern(label("b")), (1, true));
        assert_eq!(book.labels.intern(label("a")), (0, false));
        assert_eq!(book.lists.intern(GoalList::absent()), (0, true));
        let mut restored: GoalBook =
            serde_json::from_value(serde_json::to_value(&book).unwrap()).unwrap();
        assert_eq!(restored.labels.intern(label("b")), (1, false), "a held value is no new one");
        assert_eq!(restored.labels.intern(label("c")), (2, true));
        assert_eq!(restored.lists.intern(GoalList::absent()), (0, false));
        assert_eq!(restored.labels.get(2), Some(&label("c")));
    }
    #[test]
    fn absence_is_only_the_complete_hours_outside_held_lists() {
        let span = |first_hour, last_hour| HourSpan {
            first_hour,
            last_hour,
        };
        assert_eq!(
            outside(
                &[span(100, 101), span(103, 110)],
                &[span(105, 106), span(108, 120)]
            ),
            [span(100, 101), span(103, 104), span(107, 107)]
        );
        assert!(outside(&[], &[span(1, 2)]).is_empty());
    }
}
