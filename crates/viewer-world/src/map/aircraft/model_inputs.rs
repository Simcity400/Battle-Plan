//! Aircraft model results, hour-local or kept across builds, and the same signed primary-equipment
//! fold.
use crate::map::{
    equipment_graphics::{
        Decided, Limits,
        bridge::{CensusRead, Retained},
        context,
    },
    unit_models::UnitModelAssets,
};
use replay_data::rows::{WingEquipmentRow, tables};
use replay_frame::frame::{DecodedFrame, kept};
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

pub type Selected = Result<Option<String>, String>;

/// Each (variant, raw country)'s model chosen by the content's scripts, kept from one build to the
/// next while what choosing it read reads the same: the rows it read or noted (a change drops
/// exactly the models it reaches, [`kept::Kept::indexed`]) and its census entries; the pools its
/// limits decided and the countries' joined geography are read unnoted, so they are compared whole
/// and one that differs drops every model. A refusal is chosen again each build, as its words may
/// name the hour. It holds the models of the last build and the frame they were kept at.
#[derive(Default)]
pub struct KeptModels {
    held: Option<Held>,
}

struct Held {
    frame: Arc<DecodedFrame>,
    decided: Decided,
    geography: Result<context::Prepared, String>,
    /// The census at `frame`, once a model read it.
    census: Option<Result<Arc<Retained>, String>>,
    models: kept::Kept<(u64, u32), Option<String>, CensusRead>,
}

/// What one build's models did: the (variant, country)s whose model was kept from the build
/// before, and those chosen anew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelsMade {
    pub kept: usize,
    pub chosen: usize,
}

impl KeptModels {
    /// One build's models over `frame` under `limits`: `run` is given each (variant, raw country)'s
    /// model, kept or chosen by `assets` and kept for the next build. Gives back what `run` gave,
    /// every pool the build's limits decided (what a product made from the models read of the
    /// scripts beyond the rows) and what it kept and chose.
    pub fn session<R>(
        &mut self,
        assets: &UnitModelAssets,
        frame: &DecodedFrame,
        limits: Limits<'_>,
        run: impl FnOnce(&dyn Fn(u64, u32) -> Selected) -> R,
    ) -> (R, Decided, ModelsMade) {
        let now = Arc::new(frame.clone());
        let geography = assets.aircraft_geography(frame);
        let held = self.held.take().filter(|held| {
            held.geography == geography && limits.decide_the_same(&held.decided)
        });
        let (then, models, was) = match held {
            Some(held) => (held.frame, held.models, held.census),
            None => (Arc::clone(&now), kept::Kept::indexed(), None),
        };
        let census = OnceCell::new();
        let made = Cell::new(ModelsMade::default());
        let (out, models) = {
            let census_now = || census.get_or_init(|| assets.census(frame));
            let changed = |read: &CensusRead| match (&was, census_now()) {
                (Some(Ok(was)), Ok(is)) => !(Arc::ptr_eq(was, is) || was.same_read(is, read)),
                (Some(Err(was)), Err(is)) => was != is,
                _ => true,
            };
            let between = kept::Between::new(&then, &now);
            let models = RefCell::new(models.hour(&between, &changed));
            let refused = RefCell::new(BTreeMap::<(u64, u32), String>::new());
            let asked = RefCell::new(HashSet::<(u64, u32)>::new());
            let model = |variant: u64, country: u32| -> Selected {
                let key = (variant, country);
                let first = asked.borrow_mut().insert(key);
                let held = models.borrow_mut().get(&key);
                if let Some(held) = held {
                    if first {
                        made.set(ModelsMade { kept: made.get().kept + 1, ..made.get() });
                    }
                    return Ok((*held.value).clone());
                }
                if let Some(why) = refused.borrow().get(&key) {
                    return Err(why.clone());
                }
                let (chosen, reads) = kept::record::<CensusRead, _>(frame, || {
                    assets.aircraft_entity(frame, limits, variant, country)
                });
                made.set(ModelsMade { chosen: made.get().chosen + 1, ..made.get() });
                match chosen {
                    Ok(entity) => {
                        if reads.iter().any(|read| matches!(read, kept::Read::Derived(_))) {
                            census_now();
                        }
                        models.borrow_mut().put(key, entity.clone(), reads);
                        Ok(entity)
                    }
                    Err(why) => {
                        refused.borrow_mut().insert(key, why.clone());
                        Err(why)
                    }
                }
            };
            let out = run(&model);
            (out, models.into_inner().finish())
        };
        let decided = limits.decided();
        self.held = Some(Held {
            frame: now,
            decided: decided.clone(),
            geography,
            census: census.into_inner(),
            models,
        });
        (out, decided, made.get())
    }
}

/// One build call's immutable model selection results. Raw country references
/// stay distinct, and failures and known absence are retained just like names.
/// Construct a fresh memo for each hour, content and authored limit context.
#[derive(Default)]
pub struct ModelMemo {
    selected: RefCell<BTreeMap<(u64, u32), Selected>>,
}

impl ModelMemo {
    pub fn resolve(
        &self,
        variant: u64,
        raw_country: u32,
        read: impl FnOnce() -> Selected,
    ) -> Selected {
        self.selected
            .borrow_mut()
            .entry((variant, raw_country))
            .or_insert_with(read)
            .clone()
    }
}

pub(super) struct Primary<'a> {
    frame: &'a DecodedFrame,
    // Keep acquisition lazy: hours with no emitted aircraft never demand this table.
    index: OnceCell<Result<BTreeMap<u64, u64>, String>>,
}

impl<'a> Primary<'a> {
    pub(super) fn new(frame: &'a DecodedFrame) -> Self {
        Self {
            frame,
            index: OnceCell::new(),
        }
    }

    pub(super) fn get(&self, wing: u64) -> Result<Option<u64>, String> {
        self.index
            .get_or_init(|| {
                self.frame
                    .complete::<tables::WingEquipment>()
                    .map(index)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map(|index| index.get(&wing).copied())
            .map_err(Clone::clone)
    }
}

fn index(rows: &[WingEquipmentRow]) -> BTreeMap<u64, u64> {
    let mut best = BTreeMap::<u64, &WingEquipmentRow>::new();
    for row in rows {
        let old = best.entry(row.wing).or_insert(row);
        if row.amount_raw > old.amount_raw
            || (row.amount_raw == old.amount_raw && row.ordinal < old.ordinal)
        {
            *old = row;
        }
    }
    best.into_iter()
        .map(|(wing, row)| (wing, row.variant))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_primary_keeps_signed_amount_native_ordinal_and_first_exact_tie() {
        let rows = [
            (1, 9, 2, 400),
            (1, 7, 0, 400),
            (1, 3, 1, 200),
            (1, 8, 0, 400),
            (2, 9, 2, i64::MIN),
            (2, 7, 0, -100_000),
            (2, 3, 1, -200_000),
            (3, 3, 1, 0),
            (3, 4, 0, 0),
        ]
        .map(|(wing, variant, ordinal, amount_raw)| WingEquipmentRow {
            wing,
            variant,
            ordinal,
            amount_raw,
        });
        assert_eq!(index(&rows), [(1, 7), (2, 7), (3, 4)].into());
    }

    #[test]
    fn model_memo_keeps_raw_country_identity_absence_and_failure() {
        let memo = ModelMemo::default();
        let mut calls = 0;
        for _ in 0..3 {
            assert_eq!(
                memo.resolve(1, 1, || {
                    calls += 1;
                    Ok(None)
                }),
                Ok(None)
            );
            assert_eq!(
                memo.resolve(1, 2, || {
                    calls += 1;
                    Ok(Some("other country".into()))
                }),
                Ok(Some("other country".into()))
            );
            assert_eq!(
                memo.resolve(2, 1, || {
                    calls += 1;
                    Err("variant unavailable".into())
                }),
                Err("variant unavailable".into())
            );
        }
        assert_eq!(calls, 3);
    }
}
