//! Regional weather's keeper on a native recording's segment: at each hour the observer
//! observes, the temperature operands and the regions' scalars, read only where the hour
//! changed them. The observer's regions are always those of the last hour it observed, which
//! are the hour before's wherever a table kept its status, so changed regions apply to them.

use super::super::admission;
use super::{Observer, Regional, regional};
use dashboard_base::prepared::prepass::HeldBytes;
use dashboard_keyed::{Delta, Values, block::Operands, delta::Names, native, segment::Touched};
use replay_data::{
    format::FormatVersion,
    rows::{TableId, tables},
    temperature,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::btree_map::Entry, io};

type Scalars = [Option<i128>; 14];

/// The regions' scalars at an hour.
enum Regions {
    /// Every region's, by id; none where the table is not complete.
    Whole {
        known: bool,
        rows: Vec<(u32, Scalars)>,
    },
    /// The table is complete, as it was the hour before: the scalars of the regions the hour
    /// wrote, by id, None where it removed one.
    Changed(Vec<(u32, Option<Scalars>)>),
}

#[derive(Default)]
pub(super) struct Keeper {
    /// The temperature operands of the last hour kept, where its table was complete.
    temperature: Option<temperature::Input>,
}

struct Kept {
    branch: u64,
    hour: i32,
    temperature: Result<temperature::Input, String>,
    regions: Regions,
}

fn reserved<T>(count: usize, owner: &'static str) -> io::Result<Vec<T>> {
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|error| {
        admission::pressure(owner, io::Error::new(io::ErrorKind::OutOfMemory, error))
    })?;
    Ok(values)
}

impl native::Keeper<Observer> for Keeper {
    fn keep(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        touched: &Touched,
    ) -> io::Result<(Box<dyn Operands<Observer>>, usize)> {
        let mut bytes = size_of::<Kept>();
        let touch = touched.of(TableId::WeatherProvinces);
        let temperature = match frame.complete::<tables::WeatherProvinces>() {
            Ok(rows) => {
                let input = match &self.temperature {
                    Some(previous) if !touch.restated && touch.deleted.is_empty() => {
                        temperature::Input::capture_changed(
                            rows,
                            previous,
                            touch.written.iter().map(|&(key, _)| key),
                        )?
                    }
                    previous => temperature::Input::capture(rows, previous.as_ref())?,
                };
                let mut shared = [std::ptr::null::<()>(); 2];
                if let Some(previous) = &self.temperature {
                    let mut at = 0;
                    previous.allocations(|pointer, _| {
                        if let Some(slot) = shared.get_mut(at) {
                            *slot = pointer;
                        }
                        at += 1;
                    });
                }
                input.allocations(|pointer, size| {
                    if !shared.contains(&pointer) {
                        bytes += size;
                    }
                });
                self.temperature = Some(input.clone());
                Ok(input)
            }
            Err(error) => {
                self.temperature = None;
                Err(error.to_string())
            }
        };
        let touch = touched.of(TableId::WeatherRegions);
        let regions = match frame.complete::<tables::WeatherRegions>() {
            Ok(rows) if !touch.restated => {
                let mut changed = reserved(
                    admission::add(touch.written.len(), touch.deleted.len())?,
                    "changed regions",
                )?;
                for &(key, _) in &touch.written {
                    if let Ok(id) = u32::try_from(key) {
                        let row = rows.binary_search_by_key(&id, |row| row.id).ok();
                        changed.push((id, row.map(|at| regional(&rows[at]))));
                    }
                }
                for &key in &touch.deleted {
                    if let Ok(id) = u32::try_from(key) {
                        changed.push((id, None));
                    }
                }
                changed.sort_unstable_by_key(|&(id, _)| id);
                bytes += admission::bytes::<(u32, Option<Scalars>)>(changed.capacity())?;
                Regions::Changed(changed)
            }
            Ok(rows) => {
                let mut whole = reserved(rows.len(), "region scalars")?;
                whole.extend(rows.iter().map(|row| (row.id, regional(row))));
                bytes += admission::bytes::<(u32, Scalars)>(whole.capacity())?;
                Regions::Whole {
                    known: true,
                    rows: whole,
                }
            }
            Err(_) => Regions::Whole {
                known: false,
                rows: Vec::new(),
            },
        };
        Ok((
            Box::new(Kept {
                branch: frame.branch,
                hour: frame.hour,
                temperature,
                regions,
            }),
            bytes,
        ))
    }
}

fn unfollowed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "regional-weather: an hour's changed regions follow no complete regions",
    )
}

impl Operands<Observer> for Kept {
    fn observe(&self, observer: &mut Observer, values: &mut Values) -> io::Result<bool> {
        observer.previous = Some((self.branch, self.hour));
        let temperature_known = observer.temperature.observe_input(
            self.branch,
            self.hour,
            self.temperature.as_ref().map_err(String::as_str),
            values,
        )?;
        let known = match &self.regions {
            Regions::Whole { known, rows } => {
                observer.regions.clear();
                for &(id, scalars) in rows {
                    observer.regions.insert(id, Regional::new(id, scalars)?);
                }
                *known
            }
            Regions::Changed(_) if !observer.regional_known => return Err(unfollowed()),
            Regions::Changed(changed) => {
                for &(id, scalars) in changed {
                    match (observer.regions.entry(id), scalars) {
                        (Entry::Occupied(mut held), Some(scalars)) => held.get_mut().values = scalars,
                        (Entry::Occupied(held), None) => {
                            held.remove();
                        }
                        (Entry::Vacant(entry), Some(scalars)) => {
                            entry.insert(Regional::new(id, scalars)?);
                        }
                        (Entry::Vacant(_), None) => {}
                    }
                }
                true
            }
        };
        for region in observer.regions.values() {
            for (name, value) in region.names.iter().zip(region.values) {
                if let Some(value) = value {
                    values.push(name.clone(), value);
                }
            }
        }
        observer.regional_known = known;
        Ok(known || temperature_known)
    }

    fn observe_changes(
        &self,
        observer: &mut Observer,
        _: &Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<Option<bool>> {
        if observer.previous.is_none() {
            return Ok(None);
        }
        observer.previous = Some((self.branch, self.hour));
        let temperature_known = observer.temperature.observe_input_changes(
            self.branch,
            self.hour,
            self.temperature.as_ref().map_err(String::as_str),
            delta,
        )?;
        let known = match &self.regions {
            Regions::Whole { known, rows } => {
                let (mut ended, _credit) =
                    admission::vector(observer.regions.len(), "retired region keys")?;
                ended.extend(observer.regions.keys().copied().filter(|id| {
                    rows.binary_search_by_key(id, |&(held, _)| held).is_err()
                }));
                for id in ended {
                    observer.update_values(id, None, delta)?;
                }
                for &(id, scalars) in rows {
                    observer.update_values(id, Some(scalars), delta)?;
                }
                *known
            }
            Regions::Changed(_) if !observer.regional_known => return Err(unfollowed()),
            Regions::Changed(changed) => {
                for &(id, scalars) in changed {
                    observer.update_values(id, scalars, delta)?;
                }
                true
            }
        };
        observer.regional_known = known;
        Ok(Some(known || temperature_known))
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
        match &self.regions {
            Regions::Whole { rows, .. } => bytes.heap(rows),
            Regions::Changed(changed) => bytes.heap(changed),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::Observer;
    use crate::products::regional_weather::tests::{frame, row};
    use dashboard_keyed::native::testing::{assert_published_as_walked, write};
    use replay::columnar_frames::synthetic::Synthetic;
    use replay_data::rows::{Recorded, TableId, WeatherRegionRow, tables};
    use replay_frame::frame::DecodedFrame;

    fn weather(walk: &mut Synthetic, frame: &DecodedFrame) {
        write::<tables::WeatherRegions>(walk, frame);
        write::<tables::WeatherProvinces>(walk, frame);
    }

    fn rain(id: u32, light: u8, heavy: u8) -> WeatherRegionRow {
        WeatherRegionRow {
            light_rain: light,
            heavy_rain: heavy,
            ..row(id)
        }
    }

    /// From 100: a flag moving, a region appearing and ending, the regions unavailable for two
    /// hours, an empty read, a gap after 110, a modifier read and the map modifiers unknown.
    fn scenario() -> Vec<DecodedFrame> {
        let detected = WeatherRegionRow {
            air_detection: Recorded::Value(Some(Some(5))),
            ..row(71)
        };
        let unmapped = WeatherRegionRow {
            map_modifiers_known: Recorded::Value(false),
            ..row(72)
        };
        let mut frames = vec![
            frame(100, Some(vec![row(71), row(72)])),
            frame(101, Some(vec![row(71), row(72)])),
            frame(102, Some(vec![rain(71, 3, 0), row(72)])),
            frame(103, Some(vec![rain(71, 3, 0), row(72), row(73)])),
            frame(104, None),
            frame(105, None),
            frame(106, Some(vec![rain(71, 3, 0), row(73)])),
            frame(107, Some(vec![rain(71, 3, 2), row(73)])),
            frame(108, Some(vec![rain(71, 3, 2), row(73)])),
            frame(109, Some(vec![])),
            frame(110, Some(vec![row(72)])),
            frame(114, Some(vec![row(72)])),
            frame(115, Some(vec![detected.clone(), row(72)])),
            frame(116, Some(vec![detected, unmapped])),
            frame(117, Some(vec![row(71), row(72)])),
        ];
        for frame in &mut frames {
            frame.metrics.table_ns[TableId::WeatherRegions.index()] = 1;
        }
        frames
    }

    /// The native publication, its hours kept as their changed regions, holds the runs, known
    /// hours, reads and names the walk's scan holds, whatever segments start, wherever the
    /// publisher cuts and after a resume.
    #[test]
    fn a_native_publication_holds_what_the_walks_scan_holds() {
        let frames = scenario();
        for (starts, cuts, resume) in [
            (&[][..], &[117][..], None),
            // A resumed emit's next hour starts a segment, as the driver resumes.
            (&[3, 9, 10][..], &[103, 109, 117][..], Some(109)),
            (&[5, 6, 7, 13][..], &[100, 105, 110, 115, 117][..], Some(105)),
            (&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14][..], &[107, 117][..], Some(107)),
        ] {
            let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
            assert_published_as_walked::<Observer>(&frames, weather, starts, cuts, resume, &label);
        }
    }
}
