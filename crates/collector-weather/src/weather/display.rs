use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static BYTES_HINT: AtomicUsize = AtomicUsize::new(0);

pub(super) fn bytes_hint() -> usize {
    BYTES_HINT.load(Ordering::Relaxed)
}

pub(super) fn observe_capacity(
    memory: &impl Memory,
    base: usize,
) -> Result<usize, &'static str> {
    (|| {
        let [width, height] = cweather_manager::display_grid_size(memory, base)?;
        let bytes = (width as usize)
            .checked_add(height as usize)
            .and_then(|extent| extent.checked_sub(1))
            .and_then(|extent| extent.checked_mul(2))
            .ok_or("weather display sizing byte overflow")?;
        BYTES_HINT.fetch_max(bytes, Ordering::Relaxed);
        Ok(bytes)
    })()
}

#[derive(Debug)]
pub(super) struct DisplayCapture {
    bytes: Vec<u8>,
    pub(super) required: usize,
    pub(super) header: Result<cweather_manager::DisplayGrid, &'static str>,
}

impl Default for DisplayCapture {
    fn default() -> Self {
        Self::new(0)
    }
}

impl DisplayCapture {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            bytes: owned_vec(filled(capacity, 0)),
            required: 0,
            header: Err("weather display boundary unobserved"),
        }
    }
    pub(super) fn clear(&mut self) {
        self.required = 0;
        self.header = Err("weather display boundary unobserved");
    }
    pub(super) fn allocated_bytes(&self) -> usize {
        self.bytes.capacity()
    }
    pub(super) fn needs_growth(&self) -> bool {
        self.required.max(bytes_hint()) > self.bytes.len()
    }
    pub(super) fn grow(&mut self, capacity: &mut usize) {
        *capacity = (*capacity).max(self.required).max(bytes_hint());
        if *capacity > self.bytes.len() {
            let _ = resize(&mut self.bytes, *capacity, 0).map_err(crate::allocation::io);
        }
    }
    pub(super) fn capture(
        &mut self,
        memory: &impl Memory,
        grid: Result<cweather_manager::DisplayGrid, &'static str>,
    ) {
        self.clear();
        self.header = grid.and_then(|header| {
            self.required = header.edge_bytes()?;
            engine_native_memory::walk::Buffer::Growing(&mut self.bytes).ensure(self.required)?;
            header.read_edges(memory, &mut self.bytes)?;
            Ok(header)
        });
    }
    pub(super) fn copy_from(&mut self, other: &Self) {
        self.clear();
        self.required = other.required;
        self.header = other.header.and_then(|header| {
            let destination = self
                .bytes
                .get_mut(..other.required)
                .ok_or("weather display edge decode capacity exhausted")?;
            destination.copy_from_slice(&other.bytes[..other.required]);
            Ok(header)
        });
    }
    #[cfg(test)]
    pub(super) fn equal(&self, previous: Option<&rows::WeatherDisplay>) -> bool {
        match (self.header, previous) {
            (Err(_), None) => true,
            (Ok(header), Some(previous)) => {
                header.size == [previous.width, previous.height]
                    && header.refresh_interval == previous.refresh_interval
                    && header.refresh_identity == previous.refresh_identity
                    && self.bytes[..self.required] == previous.edges
            }
            _ => false,
        }
    }
    pub(super) fn row(&self) -> Option<rows::WeatherDisplay> {
        let header = self.header.ok()?;
        Some(rows::WeatherDisplay {
            width: header.size[0],
            height: header.size[1],
            refresh_interval: header.refresh_interval,
            refresh_identity: header.refresh_identity,
            edges: owned_vec(engine_allocation::copy_vec(&self.bytes[..self.required])),
        })
    }
}
