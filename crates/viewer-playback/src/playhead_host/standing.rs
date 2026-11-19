//! What each layer's product stands at, publication by publication. The map side's publication
//! waits for nothing that may be one late: it settles what is ready, takes what is published and
//! ends. This module is what keeps that honest — per layer, how far behind the publication's hour
//! its product stands, in publications and in milliseconds, so that nothing starves (a product at
//! [`BOUND`] publications behind is waited for by name) and nothing goes backwards.
//!
//! Everything counted here is counted in **publications**: one of the map side's steps
//! ([`crate::playhead_host::MapSide::frame`]), which applies one of the playhead's own steps and
//! carries that step's hourly change sets. [`BOUND`] and a layer's
//! [`crate::layers::Layer::behind_bound`] are publications too, so the one word covers the bound,
//! the wait and the refusal.
//!
//! The judge behind `BATTLE_PLAN_VIEWER_CHECK_STEP=1` refuses a step by name on a product past the bound
//! or one that moved backwards, and, once the hours have stopped, on a layer that does not stand
//! at the hour the picture shows. `BATTLE_PLAN_VIEWER_CHECK_RATES=1` is the same judge for the layers
//! whose product has a bound of its own ([`crate::layers::Layer::behind_bound`]: a rate's, or a
//! maker's own pace): what is on screen is then no older than that bound allows, and a stopped
//! picture stands at the hour it shows.

use crate::layers::{BOUND, Published};
use replay_data::Hour;
use std::{collections::VecDeque, io, time::Instant};

/// Publications kept to measure how far behind a product stands; older than all of them counts
/// as this.
const KEPT: usize = 64;

/// Publications with the hours standing still after which the picture must be whole: a thing an
/// hour brings has about 50 ms to reach the screen after the hours stop, and this is about 130 ms
/// at the rate the simulation holds. A publication the head stands still for
/// waits for every product ([`crate::layers::Settling::still`]), so the picture is whole at the
/// first of them; past this many a layer still behind is starving, not pacing.
const STOPPED: u32 = 4;

/// One layer's product as the publications left it.
#[derive(Default)]
struct Standing {
    /// The hour the newest product it published stands at. A layer that owes nothing publishes
    /// [`Published::Current`], which is a claim about the step and not about a product's hour, so
    /// it does not move this.
    at: Option<Hour>,
    /// The hour it stood at after the publication before, whatever it published.
    last: Option<Hour>,
    /// Publications at which the hour it stood at fell without a product going backwards: a layer
    /// that owed nothing and then drew something of an older hour. Counted and reported, not refused.
    fell: u64,
    /// Publications its product stood behind at the last publication that settled it.
    behind: u32,
    /// Publications it settled in, and of those the ones it stood behind in.
    publications: u64,
    stood: u64,
    /// Publications at which it reached the bound, so the publication waited for it.
    held: u64,
    worst: u32,
    /// Publications behind and milliseconds behind, each publication it stood behind (the flight
    /// record only).
    behind_publications: Vec<u32>,
    behind_ms: Vec<f64>,
}

/// Every layer's standing, and the hours the recent publications applied.
pub struct Standings {
    layers: Vec<Standing>,
    /// The last [`KEPT`] publications' hours, oldest first, with the instant each was applied.
    publications: VecDeque<(Hour, Instant)>,
    /// The hour this publication applies through.
    hour: Option<Hour>,
    /// Publications since the hours last moved: the picture must be whole once they have stopped.
    stopped: u32,
    check: bool,
    /// The rated layers alone are judged (`BATTLE_PLAN_VIEWER_CHECK_RATES`).
    check_rates: bool,
    /// The stopped picture has been judged since the hours stopped.
    judged: bool,
    /// Times the stopped picture was judged whole, so a run can say the judge reached it.
    judges: u64,
}

/// Whether the judge runs (`BATTLE_PLAN_VIEWER_CHECK_STEP=1`).
fn checking() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_STEP").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_STEP")).is_some())
}

/// Whether the rated layers' judge runs (`BATTLE_PLAN_VIEWER_CHECK_RATES=1`).
fn checking_rates() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_RATES").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_RATES")).is_some())
}

impl Standings {
    pub fn new(layers: usize) -> Self {
        Self { layers: (0..layers).map(|_| Standing::default()).collect(), publications: VecDeque::new(), hour: None, stopped: 0, check: checking(), check_rates: checking_rates(), judged: false, judges: 0 }
    }

    /// Nothing stood before: a compose or a seek's reset starts the record over, so a product of
    /// the picture before is never read as one gone backwards.
    pub fn reset(&mut self) {
        for layer in &mut self.layers {
            (layer.at, layer.last, layer.behind) = (None, None, 0);
        }
        self.publications.clear();
        (self.hour, self.stopped, self.judged) = (None, 0, false);
    }

    /// The publication applying through `hour` begins.
    pub fn step(&mut self, hour: Option<Hour>, now: Instant) {
        let moved = hour.is_some() && hour != self.hour;
        if let Some(hour) = hour.filter(|_| moved) {
            if self.publications.len() == KEPT {
                self.publications.pop_front();
            }
            self.publications.push_back((hour, now));
            self.judged = false;
        }
        (self.hour, self.stopped) = (hour, if moved { 0 } else { self.stopped.saturating_add(1) });
    }

    /// How far layer `at`'s product stood behind at the publication before: what
    /// [`crate::layers::Settling::behind_bound`] decides this publication's wait for it by.
    pub fn behind(&self, at: usize) -> u32 {
        self.layers.get(at).map_or(0, |layer| layer.behind)
    }

    /// Whether the hours have stopped for longer than the bound: a pause, the end of the
    /// recording, a stepped frame's held head.
    pub fn hours_stopped(&self) -> bool {
        self.stopped > STOPPED
    }

    /// Layer `at`, named `name`, settled or joined: where its product now stands, waited for at
    /// `bound` publications behind ([`BOUND`], or the layer's own rate's). Refuses by name under
    /// the judge when it is past that bound or has gone backwards.
    pub fn settled(&mut self, at: usize, name: &'static str, published: Published, now: Instant, bound: u32) -> io::Result<()> {
        if at >= self.layers.len() {
            self.layers.resize_with(at + 1, Standing::default);
        }
        let (stands, product) = match published {
            Published::Current => (self.hour, false),
            Published::At(hour) | Published::Cadence(hour) => (Some(hour), true),
            // Nothing made yet: it stands nowhere, and what it has not made counts no lateness.
            Published::Nothing => {
                self.layers[at].behind = 0;
                return Ok(());
            }
        };
        let (behind, since) = self.standing(stands);
        let record = viewer_foundation::view_record::on();
        // A rated layer is judged by its own rate under its own switch, as well as under the step's.
        let (check, hour) = (self.check || (bound != BOUND && self.check_rates), self.hour);
        let layer = &mut self.layers[at];
        // A product older than an earlier product is a thing on the map going backwards. A layer
        // that owed nothing and now draws something of an older hour is not: it published
        // `Current` because it had nothing to show, which is a claim about the step and not about
        // a product's hour, so that fall is counted and reported rather than refused.
        if let (Some(product_at), Some(stands)) = (layer.at, stands)
            && stands < product_at
        {
            if product && check {
                return Err(io::Error::other(format!("The {name} layer's product went backwards: it stood at {product_at} and now stands at {stands}")));
            }
            layer.fell += u64::from(!product);
        } else if let (Some(last), Some(stands)) = (layer.last, stands)
            && stands < last
        {
            layer.fell += 1;
        }
        layer.last = stands.or(layer.last);
        if product {
            layer.at = stands.max(layer.at);
        }
        layer.behind = behind;
        layer.publications += 1;
        layer.worst = layer.worst.max(behind);
        if behind > 0 {
            layer.stood += 1;
            if record {
                layer.behind_publications.push(behind);
                layer.behind_ms.push(since.map_or(0.0, |since| now.saturating_duration_since(since).as_secs_f64() * 1e3));
            }
        }
        if behind >= bound {
            layer.held += 1;
            if check && behind > bound {
                return Err(io::Error::other(format!("The {name} layer's product stood {behind} publications behind the publication's hour {hour:?}, past the bound of {bound}: it stands at {stands:?}")));
            }
        }
        Ok(())
    }

    /// Layer `at` settled whole between publications (a saved frame, a settled plan): the next
    /// publication waits by where its product now stands, not by where the last one left it. No
    /// publication is counted.
    pub fn settled_whole(&mut self, at: usize, published: Published) {
        if at >= self.layers.len() {
            self.layers.resize_with(at + 1, Standing::default);
        }
        let stands = match published {
            Published::Current => self.hour,
            Published::At(hour) | Published::Cadence(hour) => Some(hour),
            Published::Nothing => None,
        };
        self.layers[at].behind = self.standing(stands).0;
    }

    /// How many of the kept publications are newer than the hour `stands`, and when the oldest of
    /// those was applied: the publications and the milliseconds a product standing there is behind.
    fn standing(&self, stands: Option<Hour>) -> (u32, Option<Instant>) {
        let Some(stands) = stands else { return (0, None) };
        let mut count = 0_u32;
        let mut since: Option<Instant> = None;
        for &(hour, at) in &self.publications {
            if hour > stands {
                count += 1;
                since = Some(since.map_or(at, |held| held.min(at)));
            }
        }
        (count, since)
    }

    /// The hours have stopped and every drawn layer has settled: each must stand at the hour the
    /// picture shows. Judged once per stop, by name.
    pub fn stopped_whole(&mut self, drawn: &[(usize, &'static str)]) -> io::Result<()> {
        if !(self.check || self.check_rates) || self.judged || !self.hours_stopped() {
            return Ok(());
        }
        (self.judged, self.judges) = (true, self.judges + 1);
        for &(at, name) in drawn {
            let Some(layer) = self.layers.get(at) else { continue };
            if layer.behind > 0 {
                return Err(io::Error::other(format!(
                    "The hours have stopped for {} publications and the {name} layer's product still stands {} publications behind {:?}, at {:?}",
                    self.stopped, layer.behind, self.hour, layer.at
                )));
            }
        }
        Ok(())
    }

    /// What every layer stood at over the run: how often it was behind, by how many publications
    /// and for how long, and how often the bound made the publication wait for it.
    pub fn report(&self, names: &[&'static str]) -> serde_json::Value {
        use serde_json::json;
        let each: Vec<serde_json::Value> = self
            .layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.publications > 0)
            .map(|(at, layer)| {
                let mut behind: Vec<f64> = layer.behind_publications.iter().map(|&behind| f64::from(behind)).collect();
                let mut ms = layer.behind_ms.clone();
                json!({
                    "name": names.get(at).copied().unwrap_or("?"),
                    "publications": layer.publications,
                    "publications_behind": layer.stood,
                    "share_behind": layer.stood as f64 / layer.publications as f64,
                    "held": layer.held,
                    "worst_publications_behind": layer.worst,
                    "fell_without_a_product_going_back": layer.fell,
                    "behind_publications": distribution(&mut behind),
                    "behind_ms": distribution(&mut ms),
                })
            })
            .collect();
        json!({
            "bound": BOUND,
            "stopped_pictures_judged": self.judges,
            "layers": each,
            "scope": "Per layer, over the publications it settled in (one publication is the map side's step, which applies one of the playhead's with that step's own hourly change sets): publications_behind, the publications at which its product stood behind the publication's hour, with how far behind it stood at those in publications and in milliseconds; held, the publications at which it reached the bound and the publication waited for it by name; fell_without_a_product_going_back, the publications at which the hour it stood at fell because it had owed nothing the publication before and then drew something of an older hour. A layer with no worker of its own is current at every publication and shows zeros. stopped_pictures_judged counts the times the judge (BATTLE_PLAN_VIEWER_CHECK_STEP=1) reached a picture whose hours had stopped and found every drawn layer standing at it.",
        })
    }
}

fn distribution(values: &mut [f64]) -> serde_json::Value {
    use serde_json::json;
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let at = |share: f64| values.get(((values.len() as f64 * share) as usize).min(values.len().saturating_sub(1))).copied();
    json!({"p50": at(0.5), "p90": at(0.9), "p99": at(0.99), "max": values.last().copied(), "count": values.len()})
}

#[cfg(test)]
mod tests {
    use super::{KEPT, Standings};
    use crate::layers::{BOUND, Published, Rate, Settling};
    use std::time::{Duration, Instant};

    #[test]
    fn a_publication_waits_for_nothing_until_a_product_stands_at_the_bound() {
        for behind in 0..BOUND {
            assert_eq!(Settling::step().behind_bound(behind, BOUND), Settling::Step { late: false }, "{behind} publications behind is inside the bound");
            assert!(Settling::step().behind_bound(behind, BOUND).waiting(Duration::from_secs(5)).is_zero());
        }
        assert_eq!(Settling::step().behind_bound(BOUND, BOUND), Settling::Step { late: true }, "at the bound the publication waits for it");
        assert!(Settling::step().behind_bound(BOUND, BOUND).waits());
        assert!(!Settling::step().behind_bound(BOUND, BOUND).strict(), "a product at the bound is waited for, not refused by name");
        assert_eq!(Settling::Whole.behind_bound(0, BOUND), Settling::Whole, "a whole settle waits whatever stands where");
        assert!(Settling::Whole.waits() && Settling::Whole.strict());
        // A publication the head stands still for waits for every product, whatever stands where,
        // and still refuses none by name: the picture it leaves is whole at the first of them.
        for behind in 0..=BOUND {
            assert_eq!(Settling::still().behind_bound(behind, BOUND), Settling::Step { late: true }, "{behind} behind, the head standing still");
        }
        assert!(Settling::still().waits() && !Settling::still().strict());
    }

    #[test]
    fn a_product_that_stands_still_falls_a_publication_behind_a_publication_until_it_is_waited_for() {
        let mut standings = Standings::new(1);
        let now = Instant::now();
        standings.step(Some(10), now);
        standings.settled(0, "counters", Published::Current, now, BOUND).unwrap();
        assert_eq!(standings.behind(0), 0);
        for step in 1..=BOUND {
            standings.step(Some(10 + step as i32), now);
            standings.settled(0, "counters", Published::At(10), now, BOUND).unwrap();
            assert_eq!(standings.behind(0), step, "its product has stood at hour 10 for {step} publications");
        }
        assert!(Settling::step().behind_bound(standings.behind(0), BOUND).waits(), "the bound is reached and the next publication waits for it");
        standings.step(Some(11 + BOUND as i32), now);
        standings.settled(0, "counters", Published::Current, now, BOUND).unwrap();
        assert_eq!(standings.behind(0), 0, "waited for, it is current again");
    }

    /// A product made at a rate of its own stands behind by the publications its interval covers
    /// and is waited for only past them; its own judge refuses it past that bound by name.
    #[test]
    fn a_rated_product_is_waited_for_at_its_rate_and_judged_by_it() {
        let rate = Rate::a_second(4);
        let bound = rate.behind();
        assert!(bound > BOUND, "a rate of 4 a second covers more than one publication: {bound}");
        for behind in 0..bound {
            assert_eq!(Settling::step().behind_bound(behind, bound), Settling::Step { late: false }, "{behind} publications behind is inside its rate");
        }
        assert_eq!(Settling::step().behind_bound(bound, bound), Settling::Step { late: true }, "at its own bound the publication waits for it");
        let mut standings = Standings::new(1);
        standings.check_rates = true;
        let now = std::time::Instant::now();
        for at in 0..=bound as i32 + 1 {
            standings.step(Some(at), now);
            let settled = standings.settled(0, "scripts at the head", Published::Cadence(0), now, bound);
            if at <= bound as i32 {
                settled.expect("inside its rate");
            } else {
                let why = settled.expect_err("past its rate").to_string();
                assert!(why.contains("scripts at the head") && why.contains(&format!("past the bound of {bound}")), "{why}");
            }
        }
    }

    #[test]
    fn a_product_that_has_made_nothing_yet_is_behind_nothing() {
        let mut standings = Standings::new(1);
        let now = Instant::now();
        for at in 0..10 {
            standings.step(Some(at), now);
            standings.settled(0, "units", Published::Nothing, now, BOUND).unwrap();
            assert_eq!(standings.behind(0), 0);
        }
    }

    #[test]
    fn the_judge_refuses_a_product_past_the_bound_and_one_that_went_backwards() {
        let mut standings = Standings::new(1);
        standings.check = true;
        let now = Instant::now();
        for at in 0..=BOUND as i32 + 1 {
            standings.step(Some(at), now);
            let settled = standings.settled(0, "sounds", Published::At(0), now, BOUND);
            if at <= BOUND as i32 {
                settled.expect("inside the bound");
            } else {
                let why = settled.expect_err("past the bound").to_string();
                assert!(why.contains("sounds") && why.contains("past the bound"), "{why}");
            }
        }
        let mut standings = Standings::new(1);
        standings.check = true;
        standings.step(Some(5), now);
        standings.settled(0, "ships", Published::At(5), now, BOUND).unwrap();
        standings.step(Some(6), now);
        let why = standings.settled(0, "ships", Published::At(4), now, BOUND).expect_err("backwards").to_string();
        assert!(why.contains("ships") && why.contains("went backwards"), "{why}");
    }

    #[test]
    fn a_layer_that_owed_nothing_and_then_draws_an_older_hour_is_counted_not_refused() {
        let mut standings = Standings::new(1);
        standings.check = true;
        let now = Instant::now();
        standings.step(Some(20), now);
        standings.settled(0, "ships", Published::Current, now, BOUND).expect("it draws nothing and owes nothing");
        standings.step(Some(21), now);
        standings.settled(0, "ships", Published::Cadence(18), now, BOUND).expect("it now draws something of an older hour");
        assert_eq!(standings.layers[0].fell, 1, "the fall is counted");
        let why = standings.settled(0, "ships", Published::Cadence(17), now, BOUND).expect_err("a product going back").to_string();
        assert!(why.contains("went backwards"), "{why}");
    }

    #[test]
    fn the_judge_refuses_a_layer_that_has_not_caught_up_once_the_hours_have_stopped() {
        let mut standings = Standings::new(1);
        standings.check = true;
        let now = Instant::now();
        standings.step(Some(10), now);
        standings.settled(0, "buildings", Published::At(8), now, BOUND).unwrap();
        for _ in 0..=super::STOPPED {
            standings.step(Some(10), now);
            standings.settled(0, "buildings", Published::At(8), now, BOUND).unwrap();
        }
        assert!(standings.hours_stopped(), "the hours have not moved for more than the bound");
        let why = standings.stopped_whole(&[(0, "buildings")]).expect_err("the hours stopped and it is behind").to_string();
        assert!(why.contains("buildings") && why.contains("stopped"), "{why}");
        standings.judged = false;
        standings.settled(0, "buildings", Published::At(10), now, BOUND).unwrap();
        standings.stopped_whole(&[(0, "buildings")]).expect("caught up, the stopped picture is whole");
    }

    #[test]
    fn a_seek_starts_the_standing_over_so_the_picture_before_is_never_read_as_going_backwards() {
        let mut standings = Standings::new(1);
        standings.check = true;
        let now = Instant::now();
        for at in 100..100 + BOUND as i32 {
            standings.step(Some(at), now);
            standings.settled(0, "counters", Published::At(at - 1), now, BOUND).unwrap();
        }
        // The seek's reset composes the layers afresh, at an hour before the one they showed.
        standings.reset();
        assert_eq!(standings.behind(0), 0, "nothing stands behind the picture a seek has not made yet");
        for at in 20..20 + BOUND as i32 {
            standings.step(Some(at), now);
            standings.settled(0, "counters", Published::At(at - 1), now, BOUND).expect("the sought picture is its own");
            assert_eq!(standings.behind(0), 1);
        }
    }

    #[test]
    fn paused_and_resumed_every_layer_reaches_the_hour_shown_and_falls_behind_again_when_the_hours_run() {
        let mut standings = Standings::new(1);
        standings.check = true;
        let now = Instant::now();
        for at in 1..=10 {
            standings.step(Some(at), now);
            standings.settled(0, "sounds", Published::At(at - 1), now, BOUND).unwrap();
        }
        // Paused: the hours stand still and the product of the hour shown arrives.
        standings.step(Some(10), now);
        standings.settled(0, "sounds", Published::At(10), now, BOUND).unwrap();
        for _ in 0..=super::STOPPED {
            standings.step(Some(10), now);
            standings.settled(0, "sounds", Published::At(10), now, BOUND).unwrap();
        }
        assert_eq!(standings.behind(0), 0, "stopped, it stands at the hour shown");
        standings.stopped_whole(&[(0, "sounds")]).expect("the stopped picture is whole");
        // Resumed: it stands a step behind again and is never refused for it.
        for at in 11..=20 {
            standings.step(Some(at), now);
            standings.settled(0, "sounds", Published::At(at - 1), now, BOUND).unwrap();
            assert_eq!(standings.behind(0), 1);
        }
    }

    #[test]
    fn at_the_end_of_the_recording_the_hours_stop_and_every_layer_reaches_the_hour_shown() {
        let mut standings = Standings::new(2);
        standings.check = true;
        let now = Instant::now();
        standings.step(Some(60), now);
        standings.settled(0, "buildings", Published::At(59), now, BOUND).unwrap();
        standings.settled(1, "political", Published::Current, now, BOUND).unwrap();
        // No hour is ever applied again: within the bound every worker lands the last hour.
        for step in 0..=super::STOPPED {
            standings.step(Some(60), now);
            let stands = if step < BOUND { Published::At(59) } else { Published::At(60) };
            standings.settled(0, "buildings", stands, now, BOUND).unwrap();
            standings.settled(1, "political", Published::Current, now, BOUND).unwrap();
        }
        standings.stopped_whole(&[(0, "buildings"), (1, "political")]).expect("the last hour is whole on every layer");
    }

    #[test]
    fn a_product_older_than_every_publication_kept_counts_as_the_publications_kept() {
        let mut standings = Standings::new(1);
        let now = Instant::now();
        for at in 0..KEPT as i32 * 2 {
            standings.step(Some(at), now);
        }
        standings.settled(0, "labels", Published::At(0), now, BOUND).unwrap();
        assert_eq!(standings.behind(0), u32::try_from(KEPT).expect("the publications kept"), "only the publications kept are counted");
    }
}
