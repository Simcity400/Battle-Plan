
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginStatus {
    Current,
    NewOrigin,
    Retired,
}

#[derive(Debug, Default)]
pub struct OriginGate {
    origin: Option<String>,
    retired: HashSet<String>,
}

impl OriginGate {
    pub fn active(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    pub fn is_retired(&self, origin: &str) -> bool {
        self.retired.contains(origin)
    }

    pub fn admit(&mut self, origin: &str) -> OriginStatus {
        if origin.is_empty() {
            return OriginStatus::Current;
        }
        if self.retired.contains(origin) {
            return OriginStatus::Retired;
        }
        if self.origin.as_deref() == Some(origin) {
            return OriginStatus::Current;
        }
        if let Some(previous) = self.origin.replace(origin.to_owned()) {
            self.retired.insert(previous);
        }
        OriginStatus::NewOrigin
    }

    pub fn admit_all<'a>(&mut self, origins: impl IntoIterator<Item = &'a str>) -> OriginStatus {
        let mut seen = Vec::new();
        for origin in origins {
            if origin.is_empty() {
                continue;
            }
            if self.retired.contains(origin) {
                return OriginStatus::Retired;
            }
            if !seen.contains(&origin) {
                seen.push(origin);
            }
        }
        let mut status = OriginStatus::Current;
        for origin in seen {
            if self.admit(origin) == OriginStatus::NewOrigin {
                status = OriginStatus::NewOrigin;
            }
        }
        status
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ClockSample<'a> {
    pub(crate) origin: &'a str,
    pub(crate) composed_at: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Observed {
    pub(crate) age: Duration,
    pub(crate) new_origin: bool,
}

#[derive(Debug, Default)]
pub(crate) struct PageClock {
    origins: OriginGate,
    offset: Option<f64>,
    epoch: Option<Instant>,
}

impl PageClock {
    pub(crate) fn is_retired(&self, origin: &str) -> bool {
        self.origins.is_retired(origin)
    }

    pub(crate) fn observe(
        &mut self,
        sample: Option<ClockSample<'_>>,
        at: Instant,
    ) -> Option<Observed> {
        let Some(sample) = sample else {
            return Some(Observed {
                age: Duration::ZERO,
                new_origin: false,
            });
        };
        if self.origins.is_retired(sample.origin) {
            return None;
        }
        if sample.origin.is_empty()
            || !sample.composed_at.is_finite()
            || sample.composed_at < 0.0
        {
            return Some(Observed {
                age: Duration::ZERO,
                new_origin: false,
            });
        }
        let epoch = *self.epoch.get_or_insert(at);
        let here = at.saturating_duration_since(epoch).as_secs_f64() * 1e3;
        let new_origin = self.origins.admit(sample.origin) == OriginStatus::NewOrigin;
        if new_origin {
            self.offset = None;
        }
        let gap = here - sample.composed_at;
        let offset = self.offset.map_or(gap, |held| held.min(gap));
        self.offset = Some(offset);
        Some(Observed {
            age: Duration::from_secs_f64(((gap - offset) / 1e3).max(0.0)),
            new_origin,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ClockSample, Observed, PageClock};
    use std::time::{Duration, Instant};

    fn epoch() -> Instant {
        Instant::now()
    }

    #[test]
    fn an_anchors_age_is_its_delivery_delay_once_the_pages_clock_is_known() {
        let start = epoch();
        let mut clock = PageClock::default();
        let at = |milliseconds: f64| start + Duration::from_secs_f64(milliseconds / 1e3);
        let age = |clock: &mut PageClock, origin: &str, composed: f64, arrived: f64| {
            clock
                .observe(
                    Some(ClockSample {
                        origin,
                        composed_at: composed,
                    }),
                    at(arrived),
                )
                .map(|observed| observed.age.as_secs_f64() * 1e3)
        };
        assert_eq!(age(&mut clock, "page-1", 10_000.0, 4.0), Some(0.0));
        assert!((age(&mut clock, "page-1", 10_100.0, 124.0).unwrap() - 20.0).abs() < 1e-6);
        assert_eq!(age(&mut clock, "page-1", 10_200.0, 202.0), Some(0.0));
        assert!((age(&mut clock, "page-1", 10_300.0, 307.0).unwrap() - 5.0).abs() < 1e-6);
        assert!((age(&mut clock, "page-1", 10_299.0, 308.0).unwrap() - 7.0).abs() < 1e-6);
        assert_eq!(
            clock.observe(None, at(400.0)),
            Some(Observed {
                age: Duration::ZERO,
                new_origin: false,
            })
        );
        for composed_at in [f64::NAN, f64::INFINITY, -1.0] {
            assert_eq!(
                clock.observe(
                    Some(ClockSample {
                        origin: "page-1",
                        composed_at,
                    }),
                    at(400.0),
                ),
                Some(Observed {
                    age: Duration::ZERO,
                    new_origin: false,
                })
            );
        }
        assert_eq!(age(&mut clock, "page-2", 2.0, 500.0), Some(0.0));
        assert!((age(&mut clock, "page-2", 102.0, 615.0).unwrap() - 15.0).abs() < 1e-6);
    }

    #[test]
    fn a_subsecond_reload_resets_the_page_clock_instead_of_reusing_the_old_offset() {
        let start = epoch();
        let mut clock = PageClock::default();
        let at = |milliseconds: f64| start + Duration::from_secs_f64(milliseconds / 1e3);
        let first = clock
            .observe(
                Some(ClockSample {
                    origin: "page-a",
                    composed_at: 10_000.0,
                }),
                at(4.0),
            )
            .expect("first page accepted");
        assert_eq!(
            first,
            Observed {
                age: Duration::ZERO,
                new_origin: true,
            }
        );
        let reloaded = clock
            .observe(
                Some(ClockSample {
                    origin: "page-b",
                    composed_at: 9_500.0,
                }),
                at(504.0),
            )
            .expect("reloaded page accepted");
        assert_eq!(
            reloaded,
            Observed {
                age: Duration::ZERO,
                new_origin: true,
            },
            "a new page less than a second behind the previous page must reset estimation"
        );
        assert!(clock.is_retired("page-a"));
        assert!(!clock.is_retired("page-b"));
        assert_eq!(
            clock.observe(
                Some(ClockSample {
                    origin: "page-a",
                    composed_at: 10_400.0,
                }),
                at(520.0),
            ),
            None,
            "delayed anchors from a retired origin must be ignored"
        );
        let next = clock
            .observe(
                Some(ClockSample {
                    origin: "page-b",
                    composed_at: 9_600.0,
                }),
                at(629.0),
            )
            .expect("current page still uses its own offset");
        assert!(!next.new_origin);
        assert!((next.age.as_secs_f64() * 1e3 - 25.0).abs() < 1e-6, "{next:?}");
        let third = clock
            .observe(
                Some(ClockSample {
                    origin: "page-c",
                    composed_at: 9_550.0,
                }),
                at(700.0),
            )
            .expect("second subsecond reload accepted");
        assert_eq!(
            third,
            Observed {
                age: Duration::ZERO,
                new_origin: true,
            }
        );
        assert_eq!(
            clock.observe(
                Some(ClockSample {
                    origin: "page-b",
                    composed_at: 9_700.0,
                }),
                at(710.0),
            ),
            None
        );
        assert_eq!(
            clock.observe(
                Some(ClockSample {
                    origin: "page-a",
                    composed_at: 10_500.0,
                }),
                at(715.0),
            ),
            None
        );
    }
}
