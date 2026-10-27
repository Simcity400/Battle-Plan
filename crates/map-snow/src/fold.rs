use crate::{Refresh, SnowCells, SnowMud, Summary, invalid, winter::Factors};
use replay_data::rows::{Recorded, TableId, WeatherProvinceRow, tables};
use replay_frame::frame::{DecodedFrame, TableView, ViewIdentity};
use std::{collections::BTreeSet, io, sync::Arc, time::Instant};

const PAGE_CELLS: usize = map_display_data::PAGE_ITEMS / 4;
const PAGE_WORDS: usize = PAGE_CELLS.div_ceil(u64::BITS as usize);

struct DirtyCells {
    pages: std::collections::BTreeMap<usize, [u64; PAGE_WORDS]>,
    credit: map_display_data::credit::Credit,
    budget: map_display_data::credit::MemoryBudget,
}
impl DirtyCells {
    fn new(budget: &map_display_data::credit::MemoryBudget) -> io::Result<Self> {
        Ok(Self {
            pages: Default::default(),
            credit: budget.admit_item(0)?,
            budget: budget.clone(),
        })
    }
    fn insert(&mut self, cell: usize) -> io::Result<()> {
        let page = cell / PAGE_CELLS;
        if !self.pages.contains_key(&page) {
            // A whole B-tree node per page also covers its root and internal links.
            let node_bytes = 12 * (size_of::<usize>() + size_of::<[u64; PAGE_WORDS]>()) + 128;
            let bytes = self
                .credit
                .bytes()
                .checked_add(node_bytes)
                .ok_or_else(|| invalid("snow dirty-page byte count overflow"))?;
            if bytes > self.budget.limit() {
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "snow dirty-page working set exceeds its display owner; common paging is required",
                ));
            }
            self.credit.resize(bytes).map_err(|error| {
                io::Error::new(error.kind(), format!("snow dirty-page admission: {error}"))
            })?;
            self.pages.insert(page, [0; PAGE_WORDS]);
        }
        let within = cell % PAGE_CELLS;
        self.pages.get_mut(&page).unwrap()[within / u64::BITS as usize] |=
            1 << (within % u64::BITS as usize);
        Ok(())
    }
    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.pages.iter().flat_map(|(&page, words)| {
            words.iter().enumerate().flat_map(move |(word, &bits)| {
                let mut bits = bits;
                std::iter::from_fn(move || {
                    if bits == 0 {
                        return None;
                    }
                    let bit = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    Some(page * PAGE_CELLS + word * u64::BITS as usize + bit)
                })
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Province {
    snow: i64,
    display: [u8; 2],
}
impl Province {
    fn read(row: &WeatherProvinceRow) -> Self {
        Self {
            snow: row.snow,
            display: [row.display_snow, row.display_mud],
        }
    }
    fn counts(self) -> [usize; 3] {
        [
            usize::from(self.snow > 0),
            usize::from(self.display[0] != 0),
            usize::from(self.display[1] != 0),
        ]
    }
}

pub struct Fold {
    source: Option<ViewIdentity>,
    value: Option<Arc<SnowMud>>,
    provinces: map_display_data::Paged<Option<Province>>,
    present: map_display_data::copy_index::CopyIndex<u32, ()>,
    volumes:
        Option<map_display_data::retained::Retained<std::collections::BTreeMap<u32, Vec<usize>>>>,
}
impl Default for Fold {
    fn default() -> Self {
        Self {
            source: None,
            value: None,
            provinces: Default::default(),
            present: Default::default(),
            volumes: None,
        }
    }
}
impl Fold {
    pub fn project(
        &mut self,
        cells: &SnowCells,
        frame: &DecodedFrame,
        winter: &impl Factors,
    ) -> io::Result<Arc<SnowMud>> {
        let started = Instant::now();
        let view = &frame.tables[TableId::WeatherProvinces.index()];
        if self.source.as_ref().is_some_and(|source| source.is(view)) {
            return Ok(Arc::clone(self.value.as_ref().expect("installed snow")));
        }
        let (rows, complete) = match &**view {
            TableView::Complete(rows) => (
                rows.rows::<tables::WeatherProvinces>().ok_or_else(|| {
                    invalid(format!(
                        "hour {} WeatherProvinces contains another table's rows",
                        frame.hour
                    ))
                })?,
                true,
            ),
            TableView::Truncated { rows, .. } => (
                rows.rows::<tables::WeatherProvinces>().ok_or_else(|| {
                    invalid(format!(
                        "hour {} WeatherProvinces contains another table's rows",
                        frame.hour
                    ))
                })?,
                false,
            ),
            TableView::Unavailable { .. } | TableView::NotRecorded { .. } => (&[][..], false),
        };
        let fields = crate::display_fields();
        let changed = replay_frame::frame::HourInput::new(frame).changes().since(
            TableId::WeatherProvinces,
            self.source.as_ref(),
            fields,
        );
        if matches!(changed, replay_frame::frame::TableChanges::Unchanged) {
            self.source = Some(view.identity());
            return Ok(Arc::clone(self.value.as_ref().expect("installed snow")));
        }
        let changes = match changed {
            replay_frame::frame::TableChanges::Rows(facts) => Some(facts),
            _ => None,
        };
        let invalid_row = if let Some(changes) = changes {
            changes
                .changed()
                .iter()
                .find(|change| change.key > u128::from(cells.largest))
                .and_then(|change| rows.iter().find(|row| u128::from(row.id) == change.key))
        } else {
            rows.iter().find(|row| row.id > cells.largest)
        };
        if let Some(row) = invalid_row {
            return Err(invalid(format!(
                "Snow/mud presentation: 1 refused weather province {} beyond the map's largest province {}",
                row.id, cells.largest
            )));
        }
        let owner = map_display_data::credit::display_products();
        if self.value.is_none() {
            let value = Arc::new(crate::build(cells, frame, winter)?);
            let _scratch = owner.admit_item(
                cells
                    .provinces()
                    .checked_mul(size_of::<Option<Province>>())
                    .and_then(|n| n.checked_add(128))
                    .ok_or_else(|| invalid("snow province-state byte count overflow"))?,
            )?;
            let mut values = Vec::new();
            values
                .try_reserve_exact(cells.provinces())
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::OutOfMemory))?;
            values.resize(cells.provinces(), None);
            let mut present = map_display_data::copy_index::CopyIndex::default();
            for row in rows {
                values[row.id as usize] = Some(Province::read(row));
                present = present.inserted(row.id, ())?;
            }
            let provinces = map_display_data::Paged::try_from_vec_in(owner, values)?;
            let volumes = if let Some(volumes) = winter.snow_records() {
                type Index = std::collections::BTreeMap<u32, Vec<usize>>;
                let mut allocation = map_display_data::admission::Accounting::new(
                    owner,
                    map_display_data::retained::Retained::<Index>::overhead(),
                )?;
                let mut index = Index::new();
                for (volume, records) in volumes.iter().enumerate() {
                    for record in records {
                        allocation.push_index(&mut index, record.province, volume)?;
                    }
                }
                Some(allocation.finish(index))
            } else {
                None
            };
            self.provinces = provinces;
            self.present = present;
            self.volumes = volumes;
            self.source = Some(view.identity());
            self.value = Some(Arc::clone(&value));
            return Ok(value);
        }
        let previous = self.value.as_ref().unwrap();
        if changes.is_some_and(|changes| !changes.touches(fields)) {
            self.source = Some(view.identity());
            return Ok(Arc::clone(previous));
        }
        let [width, height] = cells.dimensions();
        let edge_cells = (width as usize)
            .checked_add(height as usize)
            .and_then(|count| count.checked_sub(1))
            .filter(|_| width > 0 && height > 0)
            .ok_or_else(|| invalid("Snow/mud presentation: 1 refused empty grid"))?;
        let display_mask = (0..TableId::WeatherProvinces.schema().len())
            .filter(|&field| TableId::WeatherProvinces.field_name(field) == Some("display"))
            .fold(0, |mask, field| mask | (1u64 << field));
        let edge_changed = changes.is_none_or(|changes| {
            changes.deleted().contains(&0)
                || changes.changed().iter().any(|change| {
                    change.key == 0 && (change.inserted || change.fields & display_mask != 0)
                })
        });
        let display = if edge_changed {
            rows.iter()
                .find(|row| row.id == 0)
                .and_then(|row| match &row.display {
                    Recorded::Value(Some(display)) => Some(display),
                    _ => None,
                })
        } else {
            None
        };
        let refresh = if !edge_changed {
            previous.refresh
        } else if let Some(display) = display {
            if [display.width, display.height] != [width, height]
                || display.edges.len() != edge_cells * 2
            {
                return Err(invalid(
                    "Snow/mud presentation: 1 refused retained boundary size mismatch",
                ));
            }
            Some(Refresh {
                interval: display.refresh_interval,
                identity: display.refresh_identity,
            })
        } else {
            None
        };
        let count = if let Some(changes) = changes {
            changes.changed().len().checked_add(changes.deleted().len())
        } else {
            rows.len().checked_add(self.present.len())
        }
        .ok_or_else(|| invalid("snow changed-row count overflow"))?;
        let row_bytes = 2
            * (size_of::<(u32, Option<Province>)>() + size_of::<(usize, Option<i64>)>())
            + if changes.is_none() {
                12 * (size_of::<u32>() + size_of::<Province>()) + 256
            } else {
                0
            };
        let _scratch = owner.admit_item(
            count
                .checked_mul(row_bytes)
                .and_then(|n| n.checked_add(256))
                .ok_or_else(|| invalid("snow changed-row byte count overflow"))?,
        )?;
        let mut updates = Vec::new();
        updates
            .try_reserve_exact(count)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::OutOfMemory))?;
        if let Some(changes) = changes {
            for change in changes.changed() {
                let key = change.key;
                if !change.inserted && change.fields & fields == 0 {
                    continue;
                }
                let id =
                    u32::try_from(key).map_err(|_| invalid("snow province key exceeds u32"))?;
                let row = rows
                    .binary_search_by_key(&id, |row| row.id)
                    .ok()
                    .map(|index| &rows[index])
                    .ok_or_else(|| invalid("snow delta key absent from its certified result"))?;
                updates.push((id, Some(Province::read(row))));
            }
            updates.extend(changes.deleted().iter().map(|&key| (key as u32, None)));
        } else {
            let mut seen = BTreeSet::new();
            let mut last = std::collections::BTreeMap::new();
            for row in rows {
                seen.insert(row.id);
                last.insert(row.id, Province::read(row));
            }
            updates.extend(last.into_iter().map(|(id, value)| (id, Some(value))));
            updates.extend(
                self.present
                    .iter()
                    .filter(|(id, _)| !seen.contains(id))
                    .map(|(&id, _)| (id, None)),
            );
        }
        let mut dirty = DirtyCells::new(map_display_data::credit::display_products())?;
        for &(id, value) in &updates {
            let old = self.provinces[id as usize];
            if old.map_or([0; 2], |p| p.display) != value.map_or([0; 2], |p| p.display) {
                for &cell in cells.province_cells.get(&id).into_iter().flatten() {
                    dirty.insert(cell)?;
                }
            }
        }
        let provinces = self
            .provinces
            .try_edited(updates.iter().map(|&(id, value)| (id as usize, value)))?;
        let mut present = self.present.clone();
        let mut raw_edits = Vec::new();
        raw_edits
            .try_reserve_exact(count)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::OutOfMemory))?;
        let mut counts = [
            previous.summary.with_snow,
            previous.summary.shown_snow,
            previous.summary.shown_mud,
        ];
        for (id, value) in updates {
            let old = self.provinces[id as usize];
            if old == value {
                continue;
            }
            if changes.is_some() {
                if let Some(old) = old {
                    for (sum, amount) in counts.iter_mut().zip(old.counts()) {
                        *sum -= amount;
                    }
                }
                if let Some(value) = value {
                    for (sum, amount) in counts.iter_mut().zip(value.counts()) {
                        *sum += amount;
                    }
                }
            }
            if old.map(|p| p.snow) != value.map(|p| p.snow) {
                raw_edits.push((id as usize, value.map(|p| p.snow)));
            }
            if value.is_some() {
                present = present.inserted(id, ())?;
            } else {
                present = present.removed(&id)?;
            }
        }
        if changes.is_none() {
            counts = rows.iter().fold([0; 3], |mut counts, row| {
                for (sum, amount) in counts.iter_mut().zip(Province::read(row).counts()) {
                    *sum += amount;
                }
                counts
            });
        }
        let edits = dirty
            .iter()
            .filter(|&cell| {
                let x = cell as u32 % width;
                let z = cell as u32 / width;
                x + 1 != width && z + 1 != height
            })
            .flat_map(|cell| {
                let area = crate::SNOW_MUD_BLOCK * crate::SNOW_MUD_BLOCK;
                let mut snow = u32::from(cells.perm[cell]) * 255;
                let mut mud = 0u32;
                for &(province, snow_pixels, pixels) in
                    &cells.entries[cells.offsets[cell]..cells.offsets[cell + 1]]
                {
                    let [s, m] = provinces[province as usize].map_or([0; 2], |p| p.display);
                    snow += u32::from(snow_pixels) * u32::from(s);
                    mud += u32::from(pixels) * u32::from(m);
                }
                let [s, m] = [(snow / area) as u8, (mud / area) as u8];
                [m, s, s, m]
                    .into_iter()
                    .enumerate()
                    .map(move |(channel, byte)| (cell * 4 + channel, byte))
            });
        let edges = edge_changed.then_some(()).into_iter().flat_map(|()| {
            (0..height - 1)
                .map(|z| z * width + width - 1)
                .chain((0..width).map(|x| (height - 1) * width + x))
                .enumerate()
                .flat_map(|(edge, cell)| {
                    let [s, m] =
                        display.map_or([0; 2], |d| [d.edges[edge], d.edges[edge_cells + edge]]);
                    [m, s, s, m]
                        .into_iter()
                        .enumerate()
                        .map(move |(channel, byte)| (cell as usize * 4 + channel, byte))
                })
        });
        let texels = previous.texels.try_edited(edits.chain(edges))?;
        drop(dirty);
        let raw_changed = !raw_edits.is_empty();
        let mut dirty_volumes = map_display_data::dirty::Dirty::<16>::new(owner)?;
        if let Some(index) = &self.volumes {
            for (province, _) in &raw_edits {
                for &volume in index.get(&(*province as u32)).into_iter().flatten() {
                    dirty_volumes.insert(volume)?;
                }
            }
        }
        let province_snow = previous.province_snow.try_edited(raw_edits)?;
        let post_factors =
            if !raw_changed {
                previous.post_factors.clone()
            } else if let (Some(_), Some(old)) = (&self.volumes, &previous.post_factors) {
                if dirty_volumes.iter().next().is_none() {
                    Some(old.clone())
                } else {
                    Some(old.try_edited(
                        dirty_volumes.iter().map(|index| {
                            Ok((index, winter.snow_factor_at(index, &province_snow)?))
                        }),
                    )?)
                }
            } else {
                Some(winter.snow_factors_paged(&province_snow)?)
            };
        let value = SnowMud {
            width,
            height,
            texels,
            province_snow,
            post_factors,
            refresh,
            summary: Summary {
                provinces: rows.len(),
                complete,
                with_snow: counts[0],
                shown_snow: counts[1],
                shown_mud: counts[2],
                missing_edges: if !edge_changed {
                    previous.summary.missing_edges
                } else if display.is_some() {
                    0
                } else {
                    edge_cells
                },
            },
            build: started.elapsed(),
        };
        let value = if **previous == value {
            Arc::clone(previous)
        } else {
            Arc::new(value)
        };
        self.provinces = provinces;
        self.present = present;
        self.source = Some(view.identity());
        self.value = Some(Arc::clone(&value));
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_pages_keep_exact_sparse_order_without_population_limits() {
        for count in [3usize, 1500, 5000] {
            let budget = map_display_data::credit::MemoryBudget::new(32_000_000);
            let mut actual = DirtyCells::new(&budget).unwrap();
            let mut expected = BTreeSet::new();
            for cell in (0..count).map(|key| key.wrapping_mul(104729)).chain([
                0,
                63,
                64,
                PAGE_CELLS - 1,
                PAGE_CELLS,
                usize::MAX,
            ]) {
                actual.insert(cell).unwrap();
                actual.insert(cell).unwrap();
                expected.insert(cell);
            }
            assert!(actual.iter().eq(expected.iter().copied()));
            assert!(budget.used() > 0);
            drop(actual);
            assert_eq!(budget.used(), 0);
        }
    }

    #[test]
    fn dirty_page_admission_refuses_before_mutation_and_can_retry() {
        let budget = map_display_data::credit::MemoryBudget::new(4096);
        let held = budget.admit(3000).unwrap();
        let mut dirty = DirtyCells::new(&budget).unwrap();
        let error = dirty.insert(7).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(error.to_string().contains("snow dirty-page admission"));
        assert!(dirty.iter().next().is_none());
        assert_eq!(budget.used(), 3000);
        drop(held);
        dirty.insert(7).unwrap();
        dirty.insert(PAGE_CELLS + 7).unwrap();
        let before = budget.used();
        let error = dirty.insert(2 * PAGE_CELLS + 7).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
        assert!(error.to_string().contains("common paging is required"));
        assert_eq!(budget.used(), before);
        assert!(dirty.iter().eq([7, PAGE_CELLS + 7]));
        drop(dirty);
        assert_eq!(budget.used(), 0);
    }
}
