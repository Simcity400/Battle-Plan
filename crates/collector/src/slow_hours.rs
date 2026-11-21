
use crate::sampling::HourPhases;

const HISTOGRAM_FLOOR_MS: [u64; 12] = [0, 2, 4, 8, 16, 32, 50, 64, 100, 128, 256, 1000];
const BUDGET_NS: u64 = 50_000_000;
const TOP_TABLES: usize = 4;

pub(crate) fn over_budget(callback_ns: u64, phases: &HourPhases) -> bool {
    callback_ns.saturating_add(phases.entry_ns) > BUDGET_NS
}

pub(crate) fn top_tables(
    table_ns: &[u64; replay_data::rows::TABLE_COUNT],
) -> [(replay_data::rows::TableId, u64); TOP_TABLES] {
    let mut top = [(replay_data::rows::TableId::Clock, 0); TOP_TABLES];
    for table in replay_data::rows::TableId::ALL {
        let ns = table_ns[table.index()];
        if let Some(at) = top.iter().position(|&(_, held)| ns > held) {
            top.copy_within(at..TOP_TABLES - 1, at + 1);
            top[at] = (table, ns);
        }
    }
    top
}

#[derive(Clone, Copy)]
struct Slow {
    hour: i32,
    seq: u64,
    held_ns: u64,
    phases: HourPhases,
    tables: [(replay_data::rows::TableId, u64); TOP_TABLES],
}

const SPOOL_BUDGET_NS: u64 = 100_000_000;

#[derive(Clone, Copy)]
pub(crate) struct SpoolFrame {
    pub hour: Option<i32>,
    pub wall_ns: u64,
    pub cycles: Option<u64>,
    pub spool: [(&'static str, u64); 4],
    pub helper: [(&'static str, u64); 4],
    pub conversion: Option<crate::pack_conversion::HourTop>,
}

pub(crate) struct SpoolWatch {
    begin: i64,
    cycles: Option<u64>,
    spool: [u64; crate::spool_phases::WALL_NAMES.len()],
    helper: [u64; crate::spool_phases::WALL_NAMES.len()],
}

impl SpoolWatch {
    pub(crate) fn start() -> Self {
        Self {
            begin: crate::clock::qpc(),
            cycles: crate::spool_phases::thread_cycles(),
            spool: crate::spool_phases::SPOOL.walls(),
            helper: crate::spool_phases::HELPER.walls(),
        }
    }

    pub(crate) fn finish(self, hour: Option<i32>, ns: impl Fn(i64) -> u64) -> Option<SpoolFrame> {
        let wall_ns = ns(crate::clock::qpc() - self.begin);
        if wall_ns <= SPOOL_BUDGET_NS {
            return None;
        }
        let top = |before: &[u64; crate::spool_phases::WALL_NAMES.len()], after: [u64; crate::spool_phases::WALL_NAMES.len()]| {
            let mut top = [("", 0); 4];
            for (name, (a, b)) in crate::spool_phases::WALL_NAMES.iter().zip(before.iter().zip(after)) {
                let ns = ns(b.saturating_sub(*a) as i64);
                if let Some(at) = top.iter().position(|&(_, held)| ns > held) {
                    top.copy_within(at..3, at + 1);
                    top[at] = (*name, ns);
                }
            }
            top
        };
        Some(SpoolFrame {
            hour,
            wall_ns,
            cycles: crate::spool_phases::cycles_since(self.cycles),
            spool: top(&self.spool, crate::spool_phases::SPOOL.walls()),
            helper: top(&self.helper, crate::spool_phases::HELPER.walls()),
            conversion: None,
        })
    }
}

impl SpoolFrame {

    fn status(&self) -> impl serde::Serialize + '_ {
        fn steps<'a>(steps: &'a [(&'static str, u64); 4]) -> impl serde::Serialize + 'a {
            crate::status::map(steps.iter().filter(|(_, ns)| *ns != 0).copied())
        }
        crate::status::status!({
            "hour": self.hour,
            "wall_ns": self.wall_ns,
            "cycles": self.cycles,
            "spool_steps_ns": steps(&self.spool),
            "helper_steps_ns": steps(&self.helper),
            "conversion": self.conversion.as_ref().map(crate::pack_conversion::HourTop::status),
        })
    }
}

#[derive(Default)]
pub(crate) struct Recorder {
    histogram: [u64; HISTOGRAM_FLOOR_MS.len()],
    slow: Vec<Slow>,
    spool: Vec<SpoolFrame>,
    variants: [u64; 5],
}

impl Recorder {
    pub(crate) fn record(
        &mut self,
        callback_ns: u64,
        hour: Option<i32>,
        seq: u64,
        phases: &HourPhases,
        tables: Option<[(replay_data::rows::TableId, u64); TOP_TABLES]>,
    ) -> std::io::Result<()> {
        let bucket =
            HISTOGRAM_FLOOR_MS.partition_point(|&floor| callback_ns >= floor * 1_000_000) - 1;
        self.histogram[bucket] = self.histogram[bucket].saturating_add(1);
        let variant = &phases.variant;
        for (total, value) in self.variants.iter_mut().zip([
            phases.variant_drain_ns,
            u64::from(variant.designs),
            u64::from(variant.unchanged),
            u64::from(variant.lane_walks),
            u64::from(variant.source_notes),
        ]) {
            *total = total.saturating_add(value);
        }
        let held_ns = callback_ns.saturating_add(phases.entry_ns);
        if over_budget(callback_ns, phases)
            && let Some(hour) = hour
        {
            engine_allocation::reserve(&mut self.slow, 1)?;
            self.slow.push(Slow {
                hour,
                seq,
                held_ns,
                phases: *phases,
                tables: tables.unwrap_or([(replay_data::rows::TableId::Clock, 0); TOP_TABLES]),
            });
        }
        Ok(())
    }

    pub(crate) fn record_spool(&mut self, frame: SpoolFrame) -> std::io::Result<()> {
        engine_allocation::reserve(&mut self.spool, 1)?;
        self.spool.push(frame);
        Ok(())
    }

    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "histogram": self.histogram.as_slice(),
            "budget_ns": BUDGET_NS,
            "over_budget": crate::status::sequence(self.slow.iter().map(Slow::status)),
            "variant_totals": crate::status::status!({
                "drain_ns": self.variants[0],
                "designs": self.variants[1],
                "unchanged": self.variants[2],
                "walked": self.variants[3],
                "source_notes": self.variants[4],
            }),
            "spool_budget_ns": SPOOL_BUDGET_NS,
            "spool_over_budget": crate::status::sequence(self.spool.iter().map(SpoolFrame::status)),
        })
    }
}

impl Slow {
    fn status(&self) -> impl serde::Serialize + '_ {
        let phases = &self.phases;
        crate::status::status!({
            "hour": self.hour,
            "seq": self.seq,
            "held_ns": self.held_ns,
            "entry_ns": phases.entry_ns,
            "frame_grow_ns": phases.frame_grow_ns,
            "free_frames": phases.free_frames,
            "setup_ns": phases.setup_ns,
            "parallel_ns": phases.parallel_ns,
            "residual_ns": phases.residual_ns,
            "segments_ns": phases.segments_ns.as_slice(),
            "slowest_join": phases.slowest_join.map(|(role, wait_ns)| crate::status::status!({
                "role": crate::status::DebugValue(crate::acquisition_jobs::ROLES[role]),
                "wait_ns": wait_ns,
            })),
            "slowest_ready": phases.slowest_ready.map(|(step, ns)| crate::status::status!({
                "step": crate::status::DebugValue(step),
                "wait_ns": ns,
            })),
            "slowest_step": phases.slowest_step.map(|(step, ns)| crate::status::status!({
                "step": crate::status::DebugValue(step),
                "ns": ns,
            })),
            "heap_calls": phases.heap_calls,
            "heap_ns": phases.heap_ns,
            "page_calls": phases.page_calls,
            "page_ns": phases.page_ns,
            "cycles": phases.cycles,
            "force_full": phases.force_full,
            "retry_cause": match phases.retry_cause {
                0 => None,
                crate::spooling::RETRY_SLICED => Some(String::from("sliced merge without rows")),
                crate::spooling::RETRY_VALIDATION => Some(String::from("validation refusal")),
                table => replay_data::rows::TableId::ALL
                    .get(usize::from(table) - 1)
                    .map(|table| format!("{table:?}")),
            },
            "full_reasons": crate::status::sequence(
                [
                    (crate::sampling::FULL_FIRST, "first"),
                    (crate::sampling::FULL_SIGNAL, "load signal"),
                    (crate::sampling::FULL_CAMPAIGN, "campaign"),
                    (crate::sampling::FULL_GAP, "gap"),
                    (crate::sampling::FULL_WORLD, "world"),
                    (crate::sampling::FULL_RECOVERY, "recovery"),
                    (crate::sampling::FULL_REFRESH, "refresh"),
                ]
                .into_iter()
                .filter(|(bit, _)| phases.full_reasons & bit != 0)
                .map(|(_, name)| name),
            ),
            "top_tables": crate::status::sequence(self.tables.iter().filter(|(_, ns)| *ns != 0).map(|(table, ns)| crate::status::status!({
                "table": crate::status::DebugValue(table),
                "ns": ns,
            }))),
            "variant_drain": crate::status::status!({
                "ns": phases.variant_drain_ns,
                "events": phases.variant.events,
                "designs": phases.variant.designs,
                "source_writes": phases.variant.source_writes,
                "source_notes": phases.variant.source_notes,
                "events_ns": phases.variant.events_ns,
                "reads_ns": phases.variant.reads_ns,
                "unchanged": phases.variant.unchanged,
                "lane_walks": phases.variant.lane_walks,
                "complete_ns": phases.variant.complete_ns.as_slice(),
                "stages_ns": phases.variant.stages_ns.as_slice(),
                "boundary_ns": phases.variant.boundary_ns.as_slice(),
                "lanes": phases.variant.lanes,
                "lane_ns": phases.variant.lane_ns.as_slice(),
            }),
            "holders": phases.holders.map(|holders| crate::status::status!({
                "stages": crate::status::sequence(
                    crate::frame_pool::FramePool::STAGES
                        .into_iter()
                        .zip(holders.stages)
                        .filter(|(_, (frames, _))| *frames != 0)
                        .map(|(stage, (frames, ns))| crate::status::status!({
                            "stage": crate::status::DebugValue(stage),
                            "frames": frames,
                            "longest_ns": ns,
                        })),
                ),
                "spool_step": crate::status::DebugValue(holders.spool),
                "helper_step": crate::status::DebugValue(holders.helper),
                "sizing_step": crate::status::DebugValue(holders.sizing),
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_hour_lands_in_the_bucket_of_its_floor() {
        let mut recorder = Recorder::default();
        for ms in [0, 1, 2, 49, 50, 999, 1000, 5000] {
            recorder
                .record(ms * 1_000_000, Some(1), 0, &HourPhases::default(), None)
                .unwrap();
        }
        let status = serde_json::to_value(recorder.status()).unwrap();
        assert_eq!(
            status["histogram"],
            serde_json::json!([2, 1, 0, 0, 0, 1, 1, 0, 0, 0, 1, 2])
        );
    }

    #[test]
    fn only_hours_over_budget_with_their_entry_are_named() {
        let mut recorder = Recorder::default();
        let entry = HourPhases {
            entry_ns: 2_000_000,
            ..HourPhases::default()
        };
        recorder.record(49_000_000, Some(7), 3, &entry, None).unwrap();
        recorder
            .record(49_000_000, Some(8), 4, &HourPhases::default(), None)
            .unwrap();
        let status = serde_json::to_value(recorder.status()).unwrap();
        let slow = status["over_budget"].as_array().unwrap();
        assert_eq!(slow.len(), 1);
        assert_eq!(slow[0]["hour"], 7);
        assert_eq!(slow[0]["seq"], 3);
        assert_eq!(slow[0]["held_ns"], 51_000_000);
    }
}
