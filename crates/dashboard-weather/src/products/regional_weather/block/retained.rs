use super::Observer;
use dashboard_base::prepared::prepass::HeldBytes;
use dashboard_keyed::{Observer as _, Values, block::Operands};
use replay_data::{rows::tables, temperature};
use replay_frame::frame::DecodedFrame;
use std::{io, sync::Arc};

#[derive(Default)]
pub(super) struct Keeper {
    temperature: Option<temperature::Input>,
    regional: Option<Arc<Regional>>,
}

struct Regional {
    known: bool,
    values: Values,
}

struct Input {
    branch: u64,
    hour: i32,
    temperature: Result<temperature::Input, String>,
    regional: Arc<Regional>,
}

fn regional_bytes(regional: &Arc<Regional>, bytes: &mut HeldBytes) {
    if bytes.once(Arc::as_ptr(regional)) {
        bytes.add(2 * size_of::<usize>() + size_of::<Regional>());
        regional.values.held(bytes);
    }
}

impl dashboard_keyed::block::Keeper<Observer> for Keeper {
    fn keep(
        &mut self,
        format: replay_data::format::FormatVersion,
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
        // An unbound observer reads only the regional scalars, through their ordinary policy.
        let mut regional = Regional {
            known: false,
            values: Values::default(),
        };
        regional.known = Observer::default().observe(format, frame, &mut regional.values)?;
        let regional = match &self.regional {
            Some(previous)
                if previous.known == regional.known
                    && previous.values.iter().eq(regional.values.iter()) =>
            {
                Arc::clone(previous)
            }
            _ => Arc::new(regional),
        };
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
        let temperature_known = observer.temperature.observe_input(
            self.branch,
            self.hour,
            self.temperature.as_ref().map_err(String::as_str),
            values,
        );
        for (name, value) in self.regional.values.iter() {
            values.push(name.to_owned(), value);
        }
        Ok(self.regional.known || temperature_known)
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
    use dashboard_keyed::block::Keeper as _;
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
