use collector_capture::sampling::HourPhases;

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
pub struct SpoolFrame {
    pub hour: Option<i32>,
    pub(crate) wall_ns: u64,
    pub(crate) cycles: Option<u64>,
    pub spool: [(&'static str, u64); 4],
    pub helper: [(&'static str, u64); 4],
    pub(crate) conversion: Option<collector_capture::pack_conversion::HourTop>,
}

pub struct SpoolWatch {
    begin: i64,
    cycles: Option<u64>,
    spool: [u64; collector_workers::spool_phases::WALL_NAMES.len()],
    helper: [u64; collector_workers::spool_phases::WALL_NAMES.len()],
    thread: std::marker::PhantomData<*mut ()>,
}

impl SpoolWatch {
    pub fn start() -> Self {
        Self {
            begin: collector_primitives::clock::qpc(),
            cycles: collector_workers::spool_phases::thread_cycles(),
            spool: collector_workers::spool_phases::SPOOL.walls(),
            helper: collector_workers::spool_phases::HELPER.walls(),
            thread: std::marker::PhantomData,
        }
    }

    pub fn finish(self, hour: Option<i32>, ns: impl Fn(i64) -> u64) -> Option<SpoolFrame> {
        let wall_ns = ns(collector_primitives::clock::qpc() - self.begin);
        if wall_ns <= SPOOL_BUDGET_NS {
            return None;
        }
        let top =
            |before: &[u64; collector_workers::spool_phases::WALL_NAMES.len()],
             after: [u64; collector_workers::spool_phases::WALL_NAMES.len()]| {
                let mut top = [("", 0); 4];
                for (name, (a, b)) in collector_workers::spool_phases::WALL_NAMES
                    .iter()
                    .zip(before.iter().zip(after))
                {
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
            cycles: collector_workers::spool_phases::cycles_since(self.cycles),
            spool: top(&self.spool, collector_workers::spool_phases::SPOOL.walls()),
            helper: top(
                &self.helper,
                collector_workers::spool_phases::HELPER.walls(),
            ),
            conversion: None,
        })
    }
}

impl SpoolFrame {
    fn status(&self) -> impl serde::Serialize + '_ {
        fn steps<'a>(steps: &'a [(&'static str, u64); 4]) -> impl serde::Serialize + 'a {
            collector_status::status::map(steps.iter().filter(|(_, ns)| *ns != 0).copied())
        }
        collector_status::status::status!({
            "hour": self.hour,
            "wall_ns": self.wall_ns,
            "cycles": self.cycles,
            "spool_steps_ns": steps(&self.spool),
            "helper_steps_ns": steps(&self.helper),
            "conversion": self.conversion.as_ref().map(collector_capture::pack_conversion::HourTop::status),
        })
    }
}

#[derive(Default)]
pub struct Recorder {
    histogram: [u64; HISTOGRAM_FLOOR_MS.len()],
    slow: Vec<Slow>,
    spool: Vec<SpoolFrame>,
    variants: [u64; 5],
}

impl Recorder {
    pub fn record(
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

    pub fn status(&self) -> impl serde::Serialize + '_ {
        collector_status::status::status!({
            "histogram": self.histogram.as_slice(),
            "budget_ns": BUDGET_NS,
            "over_budget": collector_status::status::sequence(self.slow.iter().map(Slow::status)),
            "variant_totals": collector_status::status::status!({
                "drain_ns": self.variants[0],
                "designs": self.variants[1],
                "unchanged": self.variants[2],
                "walked": self.variants[3],
                "source_notes": self.variants[4],
            }),
            "spool_budget_ns": SPOOL_BUDGET_NS,
            "spool_over_budget": collector_status::status::sequence(self.spool.iter().map(SpoolFrame::status)),
        })
    }
}

impl Slow {
    fn status(&self) -> impl serde::Serialize + '_ {
        let phases = &self.phases;
        collector_status::status::status!({
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
            "slowest_join": phases.slowest_join.map(|(role, wait_ns)| collector_status::status::status!({
                "role": collector_status::status::DebugValue(collector_workers::roles::ROLES[role]),
                "wait_ns": wait_ns,
            })),
            "slowest_ready": phases.slowest_ready.map(|(step, ns)| collector_status::status::status!({
                "step": collector_status::status::DebugValue(step),
                "wait_ns": ns,
            })),
            "slowest_step": phases.slowest_step.map(|(step, ns)| collector_status::status::status!({
                "step": collector_status::status::DebugValue(step),
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
            "full_reasons": collector_status::status::sequence(
                [
                    (collector_capture::sampling::FULL_FIRST, "first"),
                    (collector_capture::sampling::FULL_SIGNAL, "load signal"),
                    (collector_capture::sampling::FULL_CAMPAIGN, "campaign"),
                    (collector_capture::sampling::FULL_GAP, "gap"),
                    (collector_capture::sampling::FULL_WORLD, "world"),
                    (collector_capture::sampling::FULL_RECOVERY, "recovery"),
                    (collector_capture::sampling::FULL_REFRESH, "refresh"),
                ]
                .into_iter()
                .filter(|(bit, _)| phases.full_reasons & bit != 0)
                .map(|(_, name)| name),
            ),
            "top_tables": collector_status::status::sequence(self.tables.iter().filter(|(_, ns)| *ns != 0).map(|(table, ns)| collector_status::status::status!({
                "table": collector_status::status::DebugValue(table),
                "ns": ns,
            }))),
            "variant_drain": collector_status::status::status!({
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
            "holders": phases.holders.map(|holders| collector_status::status::status!({
                "stages": collector_status::status::sequence(
                    collector_capture::frame_pool::FramePool::STAGES
                        .into_iter()
                        .zip(holders.stages)
                        .filter(|(_, (frames, _))| *frames != 0)
                        .map(|(stage, (frames, ns))| collector_status::status::status!({
                            "stage": collector_status::status::DebugValue(stage),
                            "frames": frames,
                            "longest_ns": ns,
                        })),
                ),
                "spool_step": collector_status::status::DebugValue(holders.spool),
                "helper_step": collector_status::status::DebugValue(holders.helper),
                "sizing_step": collector_status::status::DebugValue(holders.sizing),
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_over_budget_frame_names_its_hours_four_costliest_packs_in_order() {
        use collector_capture::pack_conversion::{
            Conversion,
            test_support::{hour, pack},
        };
        use collector_intel::intel::VariableWork;

        let mut conversion = Conversion::default();
        let mut costs = hour(
            &[
                ("air", 9, 1),
                ("intel", 70, 1),
                ("economy", 30, 1),
                ("production", 50, 0),
                ("fog", 40, 1),
                ("weather", 1, 0),
            ],
            &[],
            &[(0, 4), (1, 200)],
        );
        costs.script_variables = Some(VariableWork {
            scopes_compared: 4,
            entry_sets_changed: 1,
            ..VariableWork::default()
        });
        conversion.record(&costs);
        let mut tail = Recorder::default();
        tail.record_spool(SpoolFrame {
            hour: Some(12),
            wall_ns: 250,
            cycles: None,
            spool: [("", 0); 4],
            helper: [("", 0); 4],
            conversion: conversion.take_latest(),
        })
        .unwrap();
        assert!(conversion.take_latest().is_none());
        let status = serde_json::to_value(tail.status()).unwrap();
        let frame = &status["spool_over_budget"][0]["conversion"];
        assert_eq!(frame["script_variables"]["path"], "changes");
        assert_eq!(frame["script_variables"]["scopes_compared"], 4);
        assert_eq!(frame["script_variables"]["entry_sets_changed"], 1);
        assert_eq!(
            frame["top_packs"],
            serde_json::json!([
                {"pack": "intel", "ns": 70},
                {"pack": "production", "ns": 50},
                {"pack": "fog", "ns": 40},
                {"pack": "economy", "ns": 30},
            ])
        );
        let mut first = vec!["air", "intel", "economy", "fog"];
        first.sort_by_key(|name| pack(name));
        let mut spool = vec!["production", "weather"];
        spool.sort_by_key(|name| pack(name));
        assert_eq!(
            frame["threads"],
            serde_json::json!([
                {"thread": "spool", "wall_ns": 4, "packs": spool},
                {"thread": "helper_1", "wall_ns": 200, "packs": first},
            ])
        );
    }

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
        recorder
            .record(49_000_000, Some(7), 3, &entry, None)
            .unwrap();
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
