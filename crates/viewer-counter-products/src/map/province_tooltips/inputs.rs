//! Demand-local country names and certified, immutable hour temperature inputs.
use installed_content_country_names::country_names::CountryNames;
use installed_content_localisation::localisation::Localisation;
use replay_data::{
    rows::WeatherProvinceRow,
    rows::tables,
    temperature::{Content, Memo},
};
use replay_frame::frame::DecodedFrame;
use std::sync::{Arc, Mutex, Weak};

/// Read only a named country and the overlords its name can reach. A hover's
/// immutable frame and localisation bind every lookup.
pub(super) struct Names<'a> {
    frame: &'a DecodedFrame,
    localisation: &'a Localisation,
}

impl<'a> Names<'a> {
    pub(super) fn new(frame: &'a DecodedFrame, localisation: &'a Localisation) -> Self {
        Self {
            frame,
            localisation,
        }
    }

    fn with<T>(&self, slot: u32, read: impl FnOnce(&CountryNames<'_>) -> T) -> T {
        let names = {
            let _span = viewer_foundation::profiling::span("pt_country_names");
            CountryNames::read_selected(self.frame, self.localisation, [slot])
        };
        read(&names)
    }

    pub(super) fn tag(&self, slot: u32, absent: &str) -> Result<String, String> {
        self.with(slot, |names| {
            names
                .operands(slot)
                .ok_or_else(|| absent.to_owned())?
                .as_ref()
                .map(|operands| operands.tag.clone())
                .map_err(Clone::clone)
        })
    }

    pub(super) fn name(&self, slot: u32) -> Result<String, String> {
        self.with(slot, |names| names.name(slot))
    }

    pub(super) fn name_with(&self, slot: u32, suffix: &str) -> Result<String, String> {
        self.with(slot, |names| names.name_with(slot, suffix))
    }
}

type Temperatures = Result<Arc<[i64]>, String>;
type Ready = (Weak<DecodedFrame>, Temperatures);

/// Successful results certify the entire map, including the checksum, not just
/// the hovered slot. Errors are immutable inputs too. Weak frame identities keep
/// results only while their exact decoded frames remain alive; a copied frame
/// falls back to the same derivation rather than borrowing another hour's proof.
#[derive(Default)]
pub(super) struct TemperaturesByFrame {
    worker: Mutex<Memo>,
    fallback: Mutex<Memo>,
    ready: Mutex<Vec<Ready>>,
}

impl TemperaturesByFrame {
    fn ready(&self, frame: &DecodedFrame) -> Option<Temperatures> {
        self.ready
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find_map(|(key, value)| {
                // Do not upgrade unrelated keys: their last owner might be
                // retiring now, leaving this lookup to drop a whole frame.
                std::ptr::eq(key.as_ptr(), frame)
                    .then(|| key.upgrade().map(|_| value.clone()))
                    .flatten()
            })
    }

    fn derive(
        memo: &mut Memo,
        frame: &DecodedFrame,
        rows: &[WeatherProvinceRow],
        content: Result<&Content, &str>,
    ) -> Temperatures {
        memo.states(frame.branch, frame.hour, rows, content)
            .map(|states| states.iter().map(|state| state.temperature).collect())
    }

    pub(super) fn prepare(&self, frame: &Arc<DecodedFrame>, content: Result<&Content, &str>) {
        if self.ready(frame).is_some() {
            return;
        }
        // Never hold the ready-result lock through a simulation. The window's
        // ready lookup cannot wait for a different hour's builder work.
        let mut memo = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.ready(frame).is_some() {
            return;
        }
        let result = frame
            .complete::<tables::WeatherProvinces>()
            .map_err(|error| error.to_string())
            .and_then(|rows| Self::derive(&mut memo, frame, rows, content));
        let mut ready = self
            .ready
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ready.retain(|(key, _)| key.strong_count() != 0);
        ready.push((Arc::downgrade(frame), result));
    }

    pub(super) fn at(
        &self,
        frame: &DecodedFrame,
        rows: &[WeatherProvinceRow],
        content: Result<&Content, &str>,
        province: u32,
    ) -> Result<i64, String> {
        if let Some(result) = self.ready(frame) {
            return Self::slot(result, province);
        }
        let mut memo = self
            .fallback
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::slot(Self::derive(&mut memo, frame, rows, content), province)
    }

    fn slot(result: Temperatures, province: u32) -> Result<i64, String> {
        result?
            .get(province as usize)
            .copied()
            .ok_or_else(|| "province weather absent".into())
    }
}

#[cfg(test)]
mod tests;
