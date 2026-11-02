use std::io;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct FrameBudgets {
    pub period: Duration,
    pub widget_slot: Duration,
    pub widget_advance: Duration,
    pub widget_render: Duration,
    pub map_decode: Duration,
    pub map_render: Duration,
}

impl FrameBudgets {
    pub fn from_refresh(refresh_hz: f64) -> io::Result<Self> {
        if !refresh_hz.is_finite() || refresh_hz <= 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid display refresh rate",
            ));
        }
        let period = Duration::try_from_secs_f64(1.0 / refresh_hz).map_err(io::Error::other)?;
        if period.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "display period is below clock precision",
            ));
        }
        Ok(Self {
            period,
            widget_slot: period.mul_f64(0.03),
            widget_advance: period.mul_f64(0.01),
            widget_render: period.mul_f64(0.02),
            map_decode: period.mul_f64(0.15),
            map_render: period.mul_f64(0.20),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PresentTime {
    pub local: Duration,
    pub host_offset_seconds: f64,
}

impl PresentTime {
    pub fn next(
        local_now: Duration,
        host_offset_seconds: f64,
        refresh_hz: f64,
    ) -> io::Result<Self> {
        if !host_offset_seconds.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid clock calibration",
            ));
        }
        let period = FrameBudgets::from_refresh(refresh_hz)?.period;
        let local = local_now
            .checked_add(period)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "present time overflow"))?;
        Ok(Self {
            local,
            host_offset_seconds,
        })
    }

    fn host_seconds(self) -> f64 {
        self.local.as_secs_f64() + self.host_offset_seconds
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub t0: Duration,
    pub h0: f64,
    pub speed: f64,
    pub paused: bool,
    pub generation: u64,
}

impl Anchor {
    pub fn hour_at(self, present_time: PresentTime) -> f64 {
        if self.paused {
            self.h0
        } else {
            self.h0 + self.speed * (present_time.host_seconds() - self.t0.as_secs_f64())
        }
    }
}

pub struct Clock {
    anchor: Anchor,
    parts: crate::Buffer<bool>,
    memory: crate::Memory,
    pending: Option<Duration>,
    pause_requested: bool,
}

impl Clock {
    pub fn new(now: Duration, hour: f64, speed: f64, paused: bool) -> io::Result<Self> {
        Self::with_memory(now, hour, speed, paused, crate::Memory::global())
    }

    pub fn with_memory(
        now: Duration,
        hour: f64,
        speed: f64,
        paused: bool,
        memory: crate::Memory,
    ) -> io::Result<Self> {
        valid_mapping(hour, speed)?;
        let parts = crate::Buffer::new(&memory, crate::Owner::Metadata, 0)?;
        Ok(Self {
            anchor: Anchor {
                t0: now,
                h0: hour,
                speed,
                paused,
                generation: 0,
            },
            parts,
            memory,
            pending: None,
            pause_requested: paused,
        })
    }

    pub fn anchor(&self) -> Anchor {
        self.anchor
    }

    pub fn deadline(&self) -> Option<Duration> {
        self.pending
    }

    pub fn register(&mut self) -> io::Result<usize> {
        if self.parts.len() == self.parts.capacity() {
            let capacity = self
                .parts
                .capacity()
                .checked_mul(2)
                .unwrap_or(usize::MAX)
                .max(1);
            let mut parts = crate::Buffer::new(&self.memory, crate::Owner::Metadata, capacity)?;
            for ready in self.parts.iter() {
                parts.push(*ready);
            }
            self.parts = parts;
        }
        let id = self.parts.len();
        self.parts.push(false);
        Ok(id)
    }

    pub fn seek(&mut self, now: Duration, hour: f64) -> io::Result<Anchor> {
        valid_mapping(hour, self.anchor.speed)?;
        let deadline = now
            .checked_add(Duration::from_millis(100))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek deadline overflow"))?;
        let generation = self.next_generation()?;
        self.anchor = Anchor {
            t0: now,
            h0: hour,
            paused: true,
            generation,
            ..self.anchor
        };
        self.parts.fill(false);
        self.pending = Some(deadline);
        Ok(self.anchor)
    }

    pub fn speed(&mut self, now: Duration, speed: f64) -> io::Result<Anchor> {
        valid_mapping(self.anchor.h0, speed)?;
        let _ = self.poll(now)?;
        let hour = self.anchor.hour_at(PresentTime {
            local: now,
            host_offset_seconds: 0.0,
        });
        self.reanchor(now, hour, speed, self.anchor.paused)
    }

    pub fn pause(&mut self, now: Duration, paused: bool) -> io::Result<Anchor> {
        let _ = self.poll(now)?;
        let hour = self.anchor.hour_at(PresentTime {
            local: now,
            host_offset_seconds: 0.0,
        });
        let anchor = self.reanchor(
            now,
            hour,
            self.anchor.speed,
            paused || self.pending.is_some(),
        )?;
        self.pause_requested = paused;
        Ok(anchor)
    }

    pub fn ready(
        &mut self,
        part: usize,
        generation: u64,
        now: Duration,
    ) -> io::Result<Option<Anchor>> {
        let Some(ready) = self.parts.get_mut(part) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unregistered clock part",
            ));
        };
        if self.pending.is_some() && generation == self.anchor.generation {
            *ready = true;
        }
        self.poll(now)
    }

    pub fn poll(&mut self, now: Duration) -> io::Result<Option<Anchor>> {
        if let Some(deadline) = self.pending {
            if now >= deadline || self.parts.iter().all(|ready| *ready) {
                self.pending = None;
                self.anchor.t0 = now.min(deadline);
                self.anchor.paused = self.pause_requested;
                return Ok(Some(self.anchor));
            }
        }
        Ok(None)
    }

    fn next_generation(&self) -> io::Result<u64> {
        self.anchor
            .generation
            .checked_add(1)
            .ok_or_else(|| io::Error::other("clock generation exhausted"))
    }

    fn reanchor(
        &mut self,
        now: Duration,
        hour: f64,
        speed: f64,
        paused: bool,
    ) -> io::Result<Anchor> {
        valid_mapping(hour, speed)?;
        let generation = self.next_generation()?;
        self.anchor = Anchor {
            t0: now,
            h0: hour,
            speed,
            paused,
            generation,
        };
        if self.pending.is_some() {
            self.parts.fill(false);
        }
        Ok(self.anchor)
    }
}

fn valid_mapping(hour: f64, speed: f64) -> io::Result<()> {
    if !hour.is_finite() || !speed.is_finite() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid playback clock mapping",
        ));
    }
    Ok(())
}

pub struct Part {
    anchor: Anchor,
    budgets: FrameBudgets,
    offset: f64,
    dropped: u64,
    drawn: Option<f64>,
    drawn_generation: Option<u64>,
}

impl Part {
    pub fn new(anchor: Anchor, refresh_hz: f64, host_offset_seconds: f64) -> io::Result<Self> {
        valid_mapping(anchor.h0, anchor.speed)?;
        if !host_offset_seconds.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid clock calibration",
            ));
        }
        Ok(Self {
            anchor,
            budgets: FrameBudgets::from_refresh(refresh_hz)?,
            offset: host_offset_seconds,
            dropped: 0,
            drawn: None,
            drawn_generation: None,
        })
    }
    pub fn anchor(&mut self, anchor: Anchor) -> io::Result<bool> {
        valid_mapping(anchor.h0, anchor.speed)?;
        let later = anchor.t0 > self.anchor.t0
            || (anchor.t0 == self.anchor.t0 && (!anchor.paused || self.anchor.paused));
        if anchor.generation > self.anchor.generation
            || (anchor.generation == self.anchor.generation && later)
        {
            self.anchor = anchor;
            return Ok(true);
        }
        Ok(false)
    }
    pub fn budgets(&self) -> FrameBudgets {
        self.budgets
    }
    pub fn hour_at(&self, present_local: Duration) -> f64 {
        self.anchor.hour_at(PresentTime {
            local: present_local,
            host_offset_seconds: self.offset,
        })
    }
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
    pub fn drawn(&self) -> Option<f64> {
        self.drawn
    }
    pub fn ready_generation(&self) -> Option<u64> {
        (self.anchor.paused
            && self.drawn_generation == Some(self.anchor.generation)
            && self.drawn == Some(self.anchor.h0))
        .then_some(self.anchor.generation)
    }
    pub fn drop_frame(&mut self) {
        self.dropped = self.dropped.saturating_add(1);
    }
    pub fn present(&mut self, hour: f64, present_local: Duration, within_budget: bool) -> bool {
        let tolerance = if self.anchor.paused {
            0.0
        } else {
            self.anchor.speed.abs() * self.budgets.period.as_secs_f64()
        };
        if !within_budget
            || !hour.is_finite()
            || (hour - self.hour_at(present_local)).abs() > tolerance + f64::EPSILON
        {
            self.drop_frame();
            return false;
        }
        self.drawn = Some(hour);
        self.drawn_generation = Some(self.anchor.generation);
        true
    }
}
