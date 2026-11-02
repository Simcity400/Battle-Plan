use super::{Content, Memo, Simulation, State};
use crate::rows::{Recorded, TemperaturePass, TemperatureRecord, WeatherProvinceRow};
use std::{borrow::Cow, io, sync::Arc};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Words {
    Anchored(Vec<State>),
    Hourly(Vec<State>),
    Refused(&'static str),
}

/// Temperature operands detached from weather rows; equal anchors share their words.
#[derive(Clone)]
pub struct Input {
    words: Arc<Words>,
    pass: Arc<Recorded<Option<TemperaturePass>>>,
}

impl Input {
    pub fn capture(rows: &[WeatherProvinceRow], previous: Option<&Self>) -> io::Result<Self> {
        let mut anchored = false;
        let mut hourly = false;
        let mut refused = None;
        let mut same = previous.is_some();
        for (slot, row) in rows.iter().enumerate() {
            if row.id as usize != slot {
                refused = Some("WeatherProvinces slots are not dense");
                break;
            }
            let state = match row.temperature.record() {
                Ok(TemperatureRecord::Anchored(state)) => {
                    anchored = true;
                    state
                }
                Ok(TemperatureRecord::AtHour(temperature)) => {
                    hourly = true;
                    State {
                        temperature,
                        ..State::default()
                    }
                }
                Ok(TemperatureRecord::NotRecorded) => {
                    refused = Some("WeatherProvinces temperature not recorded");
                    break;
                }
                Err(reason) => {
                    refused = Some(reason);
                    break;
                }
            };
            same &= previous.is_some_and(|previous| match &*previous.words {
                Words::Anchored(words) if !hourly => words.get(slot) == Some(&state),
                Words::Hourly(words) if !anchored => words.get(slot) == Some(&state),
                _ => false,
            });
        }
        if refused.is_none() && anchored && hourly {
            refused = Some("WeatherProvinces mixes anchored and hourly temperatures");
        }
        let words = match refused {
            Some(reason) => Arc::new(Words::Refused(reason)),
            None => {
                let same = same
                    && previous.is_some_and(|previous| match &*previous.words {
                        Words::Anchored(words) | Words::Hourly(words) => words.len() == rows.len(),
                        Words::Refused(_) => false,
                    });
                if same {
                    Arc::clone(&previous.unwrap().words)
                } else {
                    let mut words = Vec::new();
                    words
                        .try_reserve_exact(rows.len())
                        .map_err(io::Error::other)?;
                    for row in rows {
                        words.push(match row.temperature.record() {
                            Ok(TemperatureRecord::Anchored(state)) => state,
                            Ok(TemperatureRecord::AtHour(temperature)) => State {
                                temperature,
                                ..State::default()
                            },
                            _ => unreachable!("validated temperature operands"),
                        });
                    }
                    Arc::new(if hourly {
                        Words::Hourly(words)
                    } else {
                        Words::Anchored(words)
                    })
                }
            }
        };
        Ok(Self {
            words,
            pass: pass(rows, previous)?,
        })
    }

    /// [`Self::capture`] of rows that differ from those `previous` captured only at the slots
    /// `changed` names, read in their order: the same operands, found by reading those slots
    /// alone wherever they hold the words `previous` holds.
    pub fn capture_changed(
        rows: &[WeatherProvinceRow],
        previous: &Self,
        changed: impl IntoIterator<Item = u128>,
    ) -> io::Result<Self> {
        let (held, anchored) = match &*previous.words {
            Words::Anchored(words) => (words, true),
            Words::Hourly(words) => (words, false),
            Words::Refused(_) => return Self::capture(rows, Some(previous)),
        };
        if held.len() != rows.len() {
            return Self::capture(rows, Some(previous));
        }
        let mut first = false;
        for key in changed {
            let Some(slot) = usize::try_from(key).ok().filter(|&slot| slot < rows.len()) else {
                return Self::capture(rows, Some(previous));
            };
            first |= slot == 0;
            let row = &rows[slot];
            let same = row.id as usize == slot
                && match row.temperature.record() {
                    Ok(TemperatureRecord::Anchored(state)) => anchored && held[slot] == state,
                    Ok(TemperatureRecord::AtHour(temperature)) => {
                        !anchored
                            && held[slot]
                                == State {
                                    temperature,
                                    ..State::default()
                                }
                    }
                    _ => false,
                };
            if !same {
                return Self::capture(rows, Some(previous));
            }
        }
        Ok(Self {
            words: Arc::clone(&previous.words),
            // Only slot zero holds the pass.
            pass: if first {
                pass(rows, Some(previous))?
            } else {
                Arc::clone(&previous.pass)
            },
        })
    }

    /// Shared allocations, including Arc counters, for the caller's unique-owner byte accounting.
    pub fn allocations(&self, mut visit: impl FnMut(*const (), usize)) {
        let words = match &*self.words {
            Words::Anchored(words) | Words::Hourly(words) => words.capacity() * size_of::<State>(),
            Words::Refused(_) => 0,
        };
        visit(
            Arc::as_ptr(&self.words).cast(),
            2 * size_of::<usize>() + size_of::<Words>() + words,
        );
        let hours = match &*self.pass {
            Recorded::Value(Some(pass)) => pass.hours.capacity() * size_of::<Option<u32>>(),
            _ => 0,
        };
        visit(
            Arc::as_ptr(&self.pass).cast(),
            2 * size_of::<usize>() + size_of::<Recorded<Option<TemperaturePass>>>() + hours,
        );
    }
}

/// The pass slot zero of `rows` holds, shared with `previous`'s where equal.
fn pass(
    rows: &[WeatherProvinceRow],
    previous: Option<&Input>,
) -> io::Result<Arc<Recorded<Option<TemperaturePass>>>> {
    let pass = rows
        .first()
        .map(|row| &row.temperature_pass)
        .unwrap_or(&Recorded::NotRecorded);
    Ok(match previous {
        Some(previous) if previous.pass.as_ref() == pass => Arc::clone(&previous.pass),
        _ => Arc::new(match pass {
            Recorded::Value(Some(pass)) => {
                let mut hours = Vec::new();
                hours
                    .try_reserve_exact(pass.hours.len())
                    .map_err(io::Error::other)?;
                hours.extend_from_slice(&pass.hours);
                Recorded::Value(Some(TemperaturePass {
                    anchor: pass.anchor,
                    cursor: pass.cursor,
                    anchor_seed: pass.anchor_seed,
                    hours,
                    checksum: pass.checksum,
                }))
            }
            Recorded::Value(None) => Recorded::Value(None),
            Recorded::NotRecorded => Recorded::NotRecorded,
        }),
    })
}

impl Memo {
    /// The same validation and derivation as `states`, from detached temperature operands.
    pub fn states_input(
        &mut self,
        branch: u64,
        hour: i32,
        input: &Input,
        content: Result<&Content, &str>,
    ) -> Result<&[State], String> {
        self.incremental = false;
        self.changed.clear();
        let words = Arc::downgrade(&input.words);
        let same = self.branch == branch
            && matches!(input.pass.as_ref(), Recorded::Value(Some(pass))
                if self.anchor == Some((pass.anchor, pass.cursor)))
            && self
                .detached_words
                .as_ref()
                .is_some_and(|held| held.ptr_eq(&words));
        self.detached_words = None;
        match &*input.words {
            Words::Refused(reason) => Err((*reason).into()),
            Words::Hourly(words) => {
                self.anchor = None;
                self.held.clear();
                self.simulation = Some(Simulation::new(hour, 0, words.clone()));
                Ok(&self.simulation.as_ref().unwrap().states)
            }
            Words::Anchored(anchored) => {
                self.states_anchored(
                    branch,
                    hour,
                    (!same).then_some(Cow::Borrowed(anchored)),
                    Some(input.pass.as_ref()),
                    content,
                    true,
                )?;
                self.detached_words = Some(words);
                Ok(&self.simulation.as_ref().unwrap().states)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::ProvinceTemperature;

    fn rows() -> Vec<WeatherProvinceRow> {
        (0..1500)
            .map(|id| WeatherProvinceRow {
                id,
                temperature: ProvinceTemperature::anchored(State {
                    temperature: i64::from(id) - 750,
                    ..State::default()
                }),
                ..WeatherProvinceRow::default()
            })
            .collect()
    }

    #[test]
    fn equal_temperature_operands_share_without_retaining_weather_rows() {
        let mut rows = rows();
        let before = Input::capture(&rows, None).unwrap();
        rows[17].temperature_pass = Recorded::Value(Some(TemperaturePass::default()));
        let after = Input::capture(&rows, Some(&before)).unwrap();
        assert!(Arc::ptr_eq(&before.words, &after.words));
        assert!(Arc::ptr_eq(&before.pass, &after.pass));
        let pass = TemperaturePass {
            anchor: 17,
            cursor: 3,
            anchor_seed: Some(91),
            hours: vec![Some(7), None],
            checksum: 123_456,
        };
        rows[0].temperature_pass = Recorded::Value(Some(pass.clone()));
        let changed = Input::capture(&rows, Some(&after)).unwrap();
        assert!(Arc::ptr_eq(&after.words, &changed.words));
        assert!(!Arc::ptr_eq(&after.pass, &changed.pass));
        assert_eq!(*changed.pass, Recorded::Value(Some(pass)));
        rows[1499].temperature = ProvinceTemperature::historical(0);
        let mixed = Input::capture(&rows, Some(&after)).unwrap();
        assert!(matches!(
            &*mixed.words,
            Words::Refused("WeatherProvinces mixes anchored and hourly temperatures")
        ));
        rows[1499].id = 1500;
        let sparse = Input::capture(&rows, Some(&after)).unwrap();
        assert!(matches!(
            &*sparse.words,
            Words::Refused("WeatherProvinces slots are not dense")
        ));
    }

    /// Reading only the changed slots gives what reading every row gives, sharing what it can.
    #[test]
    fn changed_slots_capture_what_every_row_captures() {
        let same = |left: &Input, right: &Input| {
            assert_eq!(*left.words, *right.words);
            assert_eq!(*left.pass, *right.pass);
        };
        let mut rows = rows();
        let before = Input::capture(&rows, None).unwrap();
        rows[0].temperature_pass = Recorded::Value(Some(TemperaturePass {
            anchor: 17,
            cursor: 3,
            anchor_seed: Some(91),
            hours: vec![Some(7), None],
            checksum: 123_456,
        }));
        let passed = Input::capture_changed(&rows, &before, [0]).unwrap();
        same(&passed, &Input::capture(&rows, Some(&before)).unwrap());
        assert!(Arc::ptr_eq(&before.words, &passed.words));
        assert!(!Arc::ptr_eq(&before.pass, &passed.pass));
        // A slot written to the words it held, and no slot at all, keep everything.
        let kept = Input::capture_changed(&rows, &passed, [5]).unwrap();
        assert!(Arc::ptr_eq(&passed.words, &kept.words));
        assert!(Arc::ptr_eq(&passed.pass, &kept.pass));
        let quiet = Input::capture_changed(&rows, &kept, []).unwrap();
        assert!(Arc::ptr_eq(&kept.words, &quiet.words));
        rows[9].temperature = ProvinceTemperature::anchored(State {
            temperature: 4,
            drift: 1,
            ..State::default()
        });
        let anchored = Input::capture_changed(&rows, &quiet, [9]).unwrap();
        same(&anchored, &Input::capture(&rows, Some(&quiet)).unwrap());
        assert!(!Arc::ptr_eq(&quiet.words, &anchored.words));
        assert!(Arc::ptr_eq(&quiet.pass, &anchored.pass));
        rows[1499].temperature = ProvinceTemperature::historical(0);
        let mixed = Input::capture_changed(&rows, &anchored, [1499]).unwrap();
        assert!(matches!(
            &*mixed.words,
            Words::Refused("WeatherProvinces mixes anchored and hourly temperatures")
        ));
        let grown = Input::capture_changed(&rows[..1400], &anchored, []).unwrap();
        same(&grown, &Input::capture(&rows[..1400], Some(&anchored)).unwrap());
        let refused = Input::capture_changed(&rows, &mixed, []).unwrap();
        same(&refused, &Input::capture(&rows, Some(&mixed)).unwrap());
    }

    #[test]
    fn detached_inputs_match_full_row_validation_across_resets_and_refusals() {
        let content = super::super::tests::content();
        let anchor = super::super::tests::date(1936, 100, 7);
        let states = vec![State::default(); content.provinces.len()];
        let mut frame = states
            .iter()
            .enumerate()
            .map(|(id, &state)| WeatherProvinceRow {
                id: id as u32,
                temperature: ProvinceTemperature::anchored(state),
                ..WeatherProvinceRow::default()
            })
            .collect::<Vec<_>>();
        let seeds = [Some(17), None, Some(33), Some(7)];
        let mut full = Memo::default();
        let mut compact = Memo::default();
        let mut previous = None;
        for (branch, step, corrupt, missing) in [
            (1, 0, false, false),
            (1, 1, false, false),
            (1, 2, true, false),
            (1, 3, false, true),
            (1, 4, false, false),
            (2, 1, false, false),
        ] {
            let mut expected = Simulation::new(anchor, 1, states.clone());
            for &seed in &seeds[..step] {
                expected.advance(&content, seed).unwrap();
            }
            frame[0].temperature_pass = if missing {
                Recorded::NotRecorded
            } else {
                Recorded::Value(Some(TemperaturePass {
                    anchor,
                    cursor: 1,
                    anchor_seed: None,
                    hours: seeds[..step].to_vec(),
                    checksum: expected.checksum() ^ u64::from(corrupt),
                }))
            };
            let input = Input::capture(&frame, previous.as_ref()).unwrap();
            let hour = anchor + step as i32;
            assert_eq!(
                compact
                    .states_input(branch, hour, &input, Ok(&content))
                    .map(<[State]>::to_vec),
                full.states(branch, hour, &frame, Ok(&content))
                    .map(<[State]>::to_vec),
            );
            if branch == 1 && step == 1 {
                assert!(compact.changes().is_some());
                assert_eq!(compact.held(), 0);
            }
            if corrupt || missing || branch == 2 {
                assert!(compact.changes().is_none());
            }
            previous = Some(input);
        }
        for (slot, temperature) in [
            (0, ProvinceTemperature::historical(0)),
            (1, ProvinceTemperature::NOT_RECORDED),
        ] {
            frame[slot].temperature = temperature;
            let input = Input::capture(&frame, previous.as_ref()).unwrap();
            assert_eq!(
                compact
                    .states_input(2, anchor, &input, Ok(&content))
                    .map(<[State]>::to_vec),
                full.states(2, anchor, &frame, Ok(&content))
                    .map(<[State]>::to_vec),
            );
        }
    }
}
