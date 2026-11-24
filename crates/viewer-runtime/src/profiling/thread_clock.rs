use std::time::Duration;

#[derive(Clone, Copy)]
struct CalibrationReading {
    before: Duration,
    cycles: u64,
    after: Duration,
}

#[derive(Clone, Copy)]
struct CalibrationSample {
    start: CalibrationReading,
    end: CalibrationReading,
}

impl CalibrationSample {
    fn estimate(self) -> Option<(Duration, u64)> {
        let start_skew = self.start.after.checked_sub(self.start.before)?;
        let end_skew = self.end.after.checked_sub(self.end.before)?;
        let skew = start_skew.checked_add(end_skew)?;
        let interval = self.end.before.checked_sub(self.start.after)?;
        if interval < Duration::from_millis(10) || skew > interval / 1000 {
            return None;
        }
        let elapsed = interval.checked_add(skew / 2)?;
        Some((
            skew,
            calibrated_hz(self.start.cycles, self.end.cycles, elapsed)?,
        ))
    }
}

fn select_calibration(samples: impl IntoIterator<Item = CalibrationSample>) -> Option<u64> {
    samples
        .into_iter()
        .filter_map(CalibrationSample::estimate)
        .min_by_key(|(skew, _)| *skew)
        .map(|(_, hz)| hz)
}

pub fn cpu_clock_hz() -> Option<u64> {
    #[cfg(all(windows, any(target_arch = "x86", target_arch = "x86_64")))]
    {
        static HZ: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
        *HZ.get_or_init(|| {
            #[cfg(target_arch = "x86")]
            use std::arch::x86::{__cpuid, _mm_lfence, _rdtsc};
            #[cfg(target_arch = "x86_64")]
            use std::arch::x86_64::{__cpuid, _mm_lfence, _rdtsc};
            use std::time::Instant;
            if __cpuid(0x8000_0000).eax < 0x8000_0007 || __cpuid(0x8000_0007).edx & (1 << 8) == 0 {
                return None;
            }
            let stamp = || unsafe {
                _mm_lfence();
                let cycles = _rdtsc();
                _mm_lfence();
                cycles
            };
            let origin = Instant::now();
            let read = || {
                let before = origin.elapsed();
                let cycles = stamp();
                let after = origin.elapsed();
                CalibrationReading {
                    before,
                    cycles,
                    after,
                }
            };
            let samples: [_; 5] = std::array::from_fn(|_| {
                let start = read();
                std::thread::sleep(Duration::from_millis(10));
                CalibrationSample { start, end: read() }
            });
            select_calibration(samples)
        })
    }
    #[cfg(not(all(windows, any(target_arch = "x86", target_arch = "x86_64"))))]
    None
}

pub(super) fn current() -> Option<Duration> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{Foundation::HANDLE, System::Threading::GetCurrentThread};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn QueryThreadCycleTime(thread: HANDLE, cycles: *mut u64) -> i32;
        }
        let hz = cpu_clock_hz()?;
        let mut cycles = 0;
        if unsafe { QueryThreadCycleTime(GetCurrentThread(), &raw mut cycles) } == 0 {
            return None;
        }
        cycles_duration(cycles, hz)
    }
    #[cfg(not(windows))]
    None
}

fn calibrated_hz(before: u64, after: u64, elapsed: Duration) -> Option<u64> {
    if elapsed < Duration::from_millis(10) {
        return None;
    }
    let cycles = after.checked_sub(before)?;
    let hz = u128::from(cycles) * 1_000_000_000 / elapsed.as_nanos();
    u64::try_from(hz).ok().filter(|hz| *hz != 0)
}

fn cycles_duration(cycles: u64, hz: u64) -> Option<Duration> {
    if hz == 0 {
        return None;
    }
    let ns = u128::from(cycles) * 1_000_000_000 / u128::from(hz);
    Some(Duration::new(
        u64::try_from(ns / 1_000_000_000).ok()?,
        (ns % 1_000_000_000) as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(before: u64, cycles: u64, after: u64) -> CalibrationReading {
        CalibrationReading {
            before: Duration::from_nanos(before),
            cycles,
            after: Duration::from_nanos(after),
        }
    }

    fn tight_sample() -> CalibrationSample {
        CalibrationSample {
            start: reading(0, 150, 100),
            end: reading(10_000_100, 30_000_450, 10_000_200),
        }
    }

    #[test]
    fn calibration_selects_tightest_brackets_not_the_first_rate() {
        let tight = tight_sample();
        let loose = CalibrationSample {
            start: reading(0, 0, 4_000),
            end: reading(10_004_000, 30_024_000, 10_008_000),
        };
        assert!(select_calibration([loose]).is_some());
        assert_ne!(select_calibration([loose]), Some(3_000_000_000));
        for samples in [[loose, tight], [tight, loose]] {
            assert_eq!(select_calibration(samples), Some(3_000_000_000));
        }
    }

    #[test]
    fn calibration_rejects_preemption_at_either_unmatched_edge() {
        let late_start = CalibrationSample {
            start: reading(0, 30_000_000, 10_000_000),
            end: reading(20_000_000, 60_000_000, 20_000_000),
        };
        let late_end = CalibrationSample {
            start: reading(0, 0, 0),
            end: reading(10_000_000, 30_000_000, 20_000_000),
        };
        assert_eq!(select_calibration([late_start]), None);
        assert_eq!(select_calibration([late_end]), None);
        assert_eq!(select_calibration([late_start, late_end]), None);
        assert_eq!(
            select_calibration([late_start, tight_sample(), late_end]),
            Some(3_000_000_000)
        );
    }

    #[test]
    fn calibration_keeps_delay_inside_the_matched_interval() {
        let delayed = CalibrationSample {
            start: reading(0, 0, 0),
            end: reading(110_000_000, 330_000_000, 110_000_000),
        };
        assert_eq!(select_calibration([delayed]), Some(3_000_000_000));
    }

    #[test]
    fn calibration_selection_rejects_invalid_or_missing_readings() {
        let start = reading(0, 3, 0);
        let end = reading(10_000_000, 30_000_003, 10_000_000);
        for sample in [
            CalibrationSample {
                start: reading(1, 3, 0),
                end,
            },
            CalibrationSample {
                start,
                end: reading(10_000_001, 30_000_003, 10_000_000),
            },
            CalibrationSample {
                start: end,
                end: start,
            },
            CalibrationSample {
                start,
                end: reading(9_000_000, 27_000_003, 9_000_000),
            },
            CalibrationSample {
                start,
                end: reading(10_000_000, 2, 10_000_000),
            },
            CalibrationSample {
                start,
                end: reading(10_000_000, 3, 10_000_000),
            },
            CalibrationSample {
                start,
                end: reading(10_000_000, u64::MAX, 10_000_000),
            },
        ] {
            assert_eq!(select_calibration([sample]), None);
        }
        assert_eq!(select_calibration([]), None);
    }

    #[test]
    fn calibration_rejects_short_reversed_and_zero_intervals() {
        assert_eq!(
            calibrated_hz(3, 33_000_003, Duration::from_millis(10)),
            Some(3_300_000_000)
        );
        assert_eq!(calibrated_hz(3, 33_000_003, Duration::from_millis(9)), None);
        assert_eq!(calibrated_hz(3, 2, Duration::from_millis(10)), None);
        assert_eq!(calibrated_hz(3, 3, Duration::from_millis(10)), None);
    }

    #[test]
    fn cycle_conversion_keeps_sub_millisecond_values_and_large_totals() {
        assert_eq!(
            cycles_duration(825_000, 3_300_000_000),
            Some(Duration::from_micros(250))
        );
        assert_eq!(cycles_duration(0, 3_300_000_000), Some(Duration::ZERO));
        assert_eq!(cycles_duration(1, 0), None);
        assert_eq!(
            cycles_duration(u64::MAX, 1),
            Some(Duration::from_secs(u64::MAX))
        );
    }
}
