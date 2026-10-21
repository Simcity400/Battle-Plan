//! Captured target inputs and event-driven jobs/assembly.
use super::*;

pub(super) struct DispatchPlan {
    pub ticket: u64,
    pub want: Want,
    pub epoch: u64,
    pub hour: Hour,
    pub started: Instant,
    pub trace: trace::TargetTrace,
    pub previous: Option<(Hour, Arc<Slot<Political>>)>,
    pub neighbours: Vec<Slots>,
    // Only sequential colour state survives a native-hour fold, never its frame.
    pub colour: Option<io::Result<Colour>>,
    pub history_timing: Option<PartTiming>,
    pub next_source: Hour,
}

pub(super) struct HeldJob {
    pub name: &'static str,
    pub work: Box<dyn FnOnce() + Send>,
}

pub(super) enum PreparedTarget {
    Parts(Box<Pending>, VecDeque<HeldJob>),
    Ready(Box<Built>),
}

pub(super) type Colour = Option<Arc<viewer_world::map::map_mode::ColourState>>;

pub(super) struct ColourFolded {
    pub colour: io::Result<Colour>,
    pub timing: Option<PartTiming>,
}

/// One accepted fold owns one source frame and the target's existing credit.
/// Dependency waits and authored evaluation execute only on a pool worker.
impl DispatchPlan {
    pub fn colour_work(
        &mut self,
        current: Current,
        events: Sender<Event>,
        mut fill: impl FnMut(
            &replay_frame::frame::DecodedFrame,
            Option<&viewer_world::map::map_mode::ColourState>,
        ) -> io::Result<Colour>
        + Send
        + 'static,
    ) -> Box<dyn FnOnce() + Send> {
        let previous = self.previous.as_ref().map(|(_, slot)| slot.clone());
        let colour = self.colour.take();
        let ticket = self.ticket;
        let key = (self.want.branch, self.next_source);
        Box::new(move || {
            let enabled = viewer_foundation::profile_writer::profiling();
            let started = enabled.then(Instant::now);
            let cpu = enabled
                .then(viewer_foundation::profiling::current_thread_cpu_time)
                .flatten();
            let colour = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let colour = match colour {
                    Some(colour) => colour?,
                    None => previous.and_then(|slot| slot.wait()).and_then(|political| {
                        political.key.mode().and_then(|mode| mode.colour.clone())
                    }),
                };
                carry_colour(colour, [current], &mut fill)
            }))
            .unwrap_or_else(|_| Err(io::Error::other("political: the hour builder panicked")));
            let timing = started.map(|started| PartTiming {
                ms: started.elapsed().as_secs_f64() * 1000.0,
                cpu_ms: cpu
                    .zip(viewer_foundation::profiling::current_thread_cpu_time())
                    .and_then(|(before, after)| after.checked_sub(before))
                    .map(|used| used.as_secs_f64() * 1000.0),
            });
            let _ = events.send(Event::Colour {
                ticket,
                key,
                folded: Box::new(ColourFolded { colour, timing }),
            });
        })
    }

    pub fn folded(&mut self, folded: ColourFolded) {
        self.colour = Some(folded.colour);
        if let Some(timing) = folded.timing {
            self.history_timing = Some(match self.history_timing {
                None => timing,
                Some(before) => PartTiming {
                    ms: before.ms + timing.ms,
                    cpu_ms: before.cpu_ms.zip(timing.cpu_ms).map(|(a, b)| a + b),
                },
            });
        }
        self.next_source += 1; // a folded hour is strictly before the endpoint
    }
}

impl CoordinatorState {
    pub(super) fn plan(&self, hour: Hour) -> DispatchPlan {
        let want = self.want.clone().expect("next_hour found a want");
        let branch = want.branch;
        let epoch = self.epoch;
        let started = Instant::now();
        let ticket = next_id();
        let trace = trace::TargetTrace::new(
            &self.profile,
            epoch,
            branch,
            hour,
            self.profile_want,
            self.profile_received,
        );
        let nearby = neighbour_slots(&self.slots, branch, hour);
        let previous = nearby
            .iter()
            .find(|(key, _)| key.1 < hour)
            .map(|(key, slots)| (key.1, Arc::clone(&slots.political)));
        let neighbours: Vec<Slots> = nearby.into_iter().map(|(_, slots)| slots.clone()).collect();
        trace.ticket(ticket);
        let next_source = if matches!(
            &want.map_mode,
            viewer_world::map::map_mode::MapMode::Scripted(_)
        ) {
            previous
                .as_ref()
                .map_or(hour, |(from, _)| from.saturating_add(1))
        } else {
            hour
        };
        DispatchPlan {
            ticket,
            want,
            epoch,
            hour,
            started,
            trace,
            previous,
            neighbours,
            colour: None,
            history_timing: None,
            next_source,
        }
    }

    pub(super) fn finish_pending(
        &mut self,
        epoch: u64,
        branch: BranchId,
        hour: Hour,
        mut pending: Pending,
        run_started: Instant,
    ) -> Built {
        pending.alive.store(false, Ordering::Relaxed);
        // Laps arrive in pool order; report them in fixed
        // part order so the profile reads the same.
        let mut laps = pending.laps.clone();
        laps.sort_by_key(|(name, _)| lap_rank(name));
        let child_laps = ordered_child_laps(std::mem::take(&mut pending.child_laps));
        if let Some(parts) = pending.reusable()
            && epoch == self.epoch
            && self
                .want
                .as_ref()
                .is_some_and(|want| want.keeps_reusable((branch, hour)))
        {
            self.reusable.insert((branch, hour), parts);
        }
        let view = pending.view;
        let current = pending.current.clone();
        let started = pending.started;
        let trace = pending.trace.clone();
        let layers = pending.layers();
        let built = Built {
            epoch,
            branch,
            hour,
            view,
            current,
            wall: started.elapsed().as_secs_f64() * 1000.0,
            laps,
            child_laps,
            layers,
            trace,
        };
        built.trace.assembled();
        if viewer_foundation::profile_writer::profiling() {
            viewer_foundation::profile_writer::profile_print(format!(
                "PROFILE hour_build hour {hour} wall {:.3} parts_elapsed {:.3} elapsed {:.6}",
                built.wall,
                built.laps.iter().map(|(_, timing)| timing.ms).sum::<f64>(),
                run_started.elapsed().as_secs_f64(),
            ));
        }
        built
    }
}
