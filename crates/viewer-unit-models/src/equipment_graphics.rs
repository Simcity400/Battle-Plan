use content_reader_equipment_graphics::equipment_graphics::*;
use std::cell::RefCell;
pub mod bridge;
pub mod legacy;

use content_reader_parser::parser::Item;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy)]
pub struct Limits<'a> {
    evaluate:
        &'a dyn Fn(&[Item], dashboard_text::triggers::Scopes) -> dashboard_text::triggers::Eval,
    values: &'a LimitValues,
}

#[derive(Default)]
pub struct LimitValues(RefCell<BTreeMap<usize, (Arc<[Item]>, dashboard_text::triggers::Eval)>>);

pub type Decided = BTreeMap<usize, (Arc<[Item]>, Result<bool, String>)>;

impl<'a> Limits<'a> {
    pub fn new(
        values: &'a LimitValues,
        evaluate: &'a dyn Fn(
            &[Item],
            dashboard_text::triggers::Scopes,
        ) -> dashboard_text::triggers::Eval,
    ) -> Self {
        Self { evaluate, values }
    }

    pub fn decided(&self) -> Decided {
        self.values
            .0
            .borrow()
            .iter()
            .map(|(&pool, (items, eval))| (pool, (Arc::clone(items), eval.clone().decided())))
            .collect()
    }

    pub fn decide_the_same(&self, decided: &Decided) -> bool {
        decided
            .iter()
            .all(|(&pool, (items, was))| self.holds(pool, items) == *was)
    }
}

impl PoolLimits for Limits<'_> {
    fn holds(&self, pool: usize, items: &[Item]) -> Result<bool, String> {
        self.values
            .0
            .borrow_mut()
            .entry(pool)
            .or_insert_with(|| {
                let eval = replay_frame::frame::view_reads::unrecorded(|| {
                    (self.evaluate)(items, dashboard_text::triggers::Scopes::empty())
                });
                (items.into(), eval)
            })
            .1
            .clone()
            .decided()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
pub mod tests;
