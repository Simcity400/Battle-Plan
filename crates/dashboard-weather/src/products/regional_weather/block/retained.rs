use super::super::admission;
use super::{Observer, Regional, regional};
use dashboard_base::prepared::{
    facts::{Credit, MemoryBudget},
    prepass::HeldBytes,
};
use dashboard_keyed::{Delta, Values, block::Operands};
use replay_data::{
    rows::{TableId, WeatherRegionRow, tables},
    temperature,
};
use replay_frame::frame::{DecodedFrame, ViewIdentity};
use std::{collections::BTreeMap, io, sync::Arc};

#[derive(Default)]
pub(super) struct Keeper {
    temperature: Option<temperature::Input>,
    regional: Option<Arc<Regionals>>,
    identity: Option<ViewIdentity>,
}

struct Regionals {
    known: bool,
    rows: BTreeMap<u32, [Option<i128>; 14]>,
    _credit: Credit,
}

impl Regionals {
    fn allocation_bytes(count: usize) -> io::Result<usize> {
        let nodes = count
            .checked_mul(admission::tree_entry::<u32, [Option<i128>; 14]>()?)
            .ok_or_else(|| {
                admission::pressure("regional scalar size", io::ErrorKind::OutOfMemory.into())
            })?;
        admission::add(
            admission::add(2 * size_of::<usize>(), size_of::<Self>())?,
            nodes,
        )
    }

    fn project(
        rows: Option<&[WeatherRegionRow]>,
        previous: Option<&Arc<Self>>,
        budget: &MemoryBudget,
    ) -> io::Result<Arc<Self>> {
        let known = rows.is_some();
        let rows = rows.unwrap_or_default();
        if let Some(previous) = previous {
            if previous.known == known
                && previous.rows.len() == rows.len()
                && previous
                    .rows
                    .iter()
                    .zip(rows)
                    .all(|((&id, held), row)| id == row.id && *held == regional(row))
            {
                return Ok(Arc::clone(previous));
            }
        }
        let credit = admission::admit_in(
            budget,
            Self::allocation_bytes(rows.len())?,
            "shared regional scalars",
        )?;
        let mut projected = BTreeMap::new();
        for row in rows {
            projected.insert(row.id, regional(row));
        }
        Ok(Arc::new(Self {
            known,
            rows: projected,
            _credit: credit,
        }))
    }
}

struct Input {
    branch: u64,
    hour: i32,
    temperature: Result<temperature::Input, String>,
    regional: Arc<Regionals>,
}

fn regional_bytes(regional: &Arc<Regionals>, bytes: &mut HeldBytes) {
    if bytes.once(Arc::as_ptr(regional)) {
        bytes.add(2 * size_of::<usize>() + size_of::<Regionals>());
        bytes.tree::<u32, [Option<i128>; 14]>(regional.rows.len());
    }
}

impl dashboard_keyed::block::Keeper<Observer> for Keeper {
    fn keep(
        &mut self,
        _: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
    ) -> io::Result<Box<dyn Operands<Observer>>> {
        let temperature = match frame.complete::<tables::WeatherProvinces>() {
            Ok(rows) => {
                let input = temperature::Input::capture(rows, self.temperature.as_ref())?;
                self.temperature = Some(input.clone());
                Ok(input)
            }
            Err(error) => {
                self.temperature = None;
                Err(error.to_string())
            }
        };
        let identity = frame.tables[TableId::WeatherRegions.index()].identity();
        let regional = if self.identity.as_ref() == Some(&identity) {
            Arc::clone(self.regional.as_ref().unwrap())
        } else {
            Regionals::project(
                frame.complete::<tables::WeatherRegions>().ok(),
                self.regional.as_ref(),
                &MemoryBudget::preparation(),
            )?
        };
        self.identity = Some(identity);
        self.regional = Some(Arc::clone(&regional));
        Ok(Box::new(Input {
            branch: frame.branch,
            hour: frame.hour,
            temperature,
            regional,
        }))
    }

    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(size_of::<Self>());
        if let Some(temperature) = &self.temperature {
            temperature.allocations(|pointer, size| {
                bytes.shared(pointer, size);
            });
        }
        if let Some(regional) = &self.regional {
            regional_bytes(regional, bytes);
        }
    }
}

impl Operands<Observer> for Input {
    fn observe(&self, observer: &mut Observer, values: &mut Values) -> io::Result<bool> {
        let _ = observer.input_point((self.branch, self.hour));
        let temperature_known = observer.temperature.observe_input(
            self.branch,
            self.hour,
            self.temperature.as_ref().map_err(String::as_str),
            values,
        )?;
        observer.regions.clear();
        for (&id, &row) in &self.regional.rows {
            let region = Regional::new(id, row)?;
            for (name, value) in region.names.iter().zip(row) {
                if let Some(value) = value {
                    values.push(name.clone(), value);
                }
            }
            observer.regions.insert(id, region);
        }
        observer.regional_known = self.regional.known;
        Ok(self.regional.known || temperature_known)
    }

    fn observe_changes(
        &self,
        observer: &mut Observer,
        _: &dashboard_keyed::delta::Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<Option<bool>> {
        if observer.previous.is_none() {
            return Ok(None);
        }
        let input = observer.input_point((self.branch, self.hour));
        let temperature_known = observer.temperature.observe_input_changes(
            self.branch,
            self.hour,
            self.temperature.as_ref().map_err(String::as_str),
            delta,
        )?;
        let keys = input
            .as_ref()
            .and_then(|input| input.regions.as_deref())
            .filter(|_| self.regional.known && observer.regional_known);
        match keys {
            Some(keys) => {
                for &key in keys {
                    if let Ok(id) = u32::try_from(key) {
                        observer.update_values(id, self.regional.rows.get(&id).copied(), delta)?;
                    }
                }
            }
            None => {
                let (mut ended, _credit) =
                    admission::vector(observer.regions.len(), "retired region keys")?;
                ended.extend(
                    observer
                        .regions
                        .keys()
                        .copied()
                        .filter(|id| !self.regional.rows.contains_key(id)),
                );
                for id in ended {
                    observer.update_values(id, None, delta)?;
                }
                for (&id, &row) in &self.regional.rows {
                    observer.update_values(id, Some(row), delta)?;
                }
            }
        }
        observer.regional_known = self.regional.known;
        Ok(Some(self.regional.known || temperature_known))
    }

    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(size_of::<Self>());
        match &self.temperature {
            Ok(temperature) => temperature.allocations(|pointer, size| {
                bytes.shared(pointer, size);
            }),
            Err(reason) => {
                bytes.heap(reason);
            }
        }
        regional_bytes(&self.regional, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::regional_weather::tests::{frame, row};
    use dashboard_keyed::{Observer as _, block::Keeper as _};
    use replay_data::{format::FormatVersion, rows::TableId};

    #[test]
    fn weather_projection_drops_source_views_and_preserves_scalar_order_and_unknowns() {
        let mut keeper = Keeper::default();
        let mut whole = Observer::default();
        let mut compact = Observer::default();
        for (hour, rows) in [
            (10, Some(vec![row(71), row(72)])),
            (11, None),
            (12, Some(vec![])),
        ] {
            let frame = frame(hour, rows);
            let identity = frame.tables[TableId::WeatherRegions.index()].identity();
            let input = keeper.keep(FormatVersion::CURRENT, &frame).unwrap();
            let mut expected = Values::default();
            let expected_known = whole
                .observe(FormatVersion::CURRENT, &frame, &mut expected)
                .unwrap();
            drop(frame);
            assert!(!identity.is_alive());
            let mut actual = Values::default();
            assert_eq!(
                input.observe(&mut compact, &mut actual).unwrap(),
                expected_known
            );
            assert!(expected.iter().eq(actual.iter()));
            assert_eq!(
                serde_json::to_value(&whole).unwrap(),
                serde_json::to_value(&compact).unwrap()
            );
        }
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use crate::products::regional_weather::tests::row;

    #[test]
    fn shared_scalars_admit_before_growth_and_equal_projection_needs_no_credit() {
        let rows = [row(71), row(72)];
        let denied = MemoryBudget::new(0);
        let error = Regionals::project(Some(&rows), None, &denied)
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(error.to_string().contains("shared regional scalars"));
        assert_eq!(denied.used(), 0);
        let expected = Regionals::allocation_bytes(rows.len()).unwrap();
        let budget = MemoryBudget::new(expected);
        let first = Regionals::project(Some(&rows), None, &budget).unwrap();
        assert_eq!(budget.used(), expected);
        let same = Regionals::project(Some(&rows), Some(&first), &denied).unwrap();
        assert!(Arc::ptr_eq(&first, &same));
        assert_eq!(budget.used(), expected);
        let mut changed = rows.clone();
        changed[0].light_rain += 1;
        assert_eq!(
            Regionals::project(Some(&changed), Some(&same), &denied)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(first.rows[&71], regional(&rows[0]));
        assert_eq!(budget.used(), expected);
        drop(first);
        assert_eq!(budget.used(), expected);
        drop(same);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn unknown_and_known_empty_scalars_keep_distinct_admitted_shared_owners() {
        let expected = Regionals::allocation_bytes(0).unwrap();
        let budget = MemoryBudget::new(expected);
        let unknown = Regionals::project(None, None, &budget).unwrap();
        let same = Regionals::project(None, Some(&unknown), &MemoryBudget::new(0)).unwrap();
        assert!(Arc::ptr_eq(&unknown, &same));
        assert!(!unknown.known);
        assert!(unknown.rows.is_empty());
        assert_eq!(
            Regionals::project(Some(&[]), Some(&unknown), &budget)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        drop(unknown);
        drop(same);
        assert_eq!(budget.used(), 0);
        let empty = Regionals::project(Some(&[]), None, &budget).unwrap();
        assert!(empty.known);
        assert!(empty.rows.is_empty());
        assert_eq!(budget.used(), expected);
        drop(empty);
        assert_eq!(budget.used(), 0);
    }
}
