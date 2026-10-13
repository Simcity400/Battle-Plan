//! What-if for `DivisionStatus.weather_factor` (`--weather-factor`): the
//! table written again with the current writer, as recorded and with the
//! factor truncated toward zero to each candidate step ([`stepped`]), and how
//! often the attack list's region weather entry would then print differently.
//!
//! The movement-availability factor is never shown. It scales the target
//! region's weather `army_attack_factor` and `breakthrough_factor`
//! ([`attack_derive::weather`]); the selected-army attack list includes the
//! entry when either product is nonzero. It prints the first product as a
//! fixed5 percent with precision 2 and colour enabled, after testing its sign:
//! the value times 100, truncated to two decimals, green and `+` when positive.
//! The printed digits are therefore `trunc(a * f / 10^6)` for an attack factor
//! `a` and acclimatization factor `f`, and the entry is listed from
//! `|a f| >= 10^5`. A step `s` keeps both when the digit boundaries
//! `10^6 / |a|` are multiples of `s` and the listing boundary
//! `ceil(10^5 / |a|)` is one too or at most `s`, a factor below one step
//! staying exact: [`DEFAULT_STEPS`].
//!
//! The entry's target is the province the hovered route ends in; this
//! estimate uses each division's own province, whose region is usually the
//! target's.

use std::collections::BTreeMap;

use replay::{
    attack_derive::{self, Context, Fact, FrameInputs, Inputs, Terrain},
    display::{Rounding, Step},
    frame::{DecodedFrame, TableView},
    rows::{DivisionStatusRow, Recorded, TableId, tables},
};

/// 25 raw (0.00025) keeps the printed term for vanilla's single weather
/// attack entries (`army_attack_factor` -0.1, -0.2 and -0.4 in
/// `00_static_modifiers.txt`: boundaries every 100, 50 and 25 raw), the
/// coarsest such step; 5 raw also keeps it for 0.05 multiples whose
/// boundary is a whole multiple of 5 (0.05, 0.25, 0.5, 1). Sums of entries
/// such as -0.6 (boundaries every 16.7 raw) are kept by no step.
pub(super) const DEFAULT_STEPS: [i64; 2] = [25, 5];

/// The factor truncated toward zero to `step`, a factor below one step (or a
/// sentinel word) staying exact; the recorded value without a step.
pub(super) fn stepped(factor: i64, step: Option<Step>) -> i64 {
    match step {
        Some(step) if factor.unsigned_abs() >= step.raw().unsigned_abs() => step.round(factor),
        _ => factor,
    }
}

/// What the game prints for a region weather pair: nothing when both
/// products are zero, else the sign branch and the percent text.
pub(super) fn shown(pair: [i64; 2]) -> Option<(bool, String)> {
    (pair != [0, 0]).then(|| {
        (
            pair[0] > 0,
            installed_content::fixed5::fixed5_percent(pair[0], 2, true),
        )
    })
}

/// Province to strategic region, from the recorded content.
pub(super) struct Regions(pub(super) Result<BTreeMap<u32, u32>, String>);

const UNREAD: &str = "not read by the weather_factor what-if";

impl Context for Regions {
    fn terrain(&self, _province: u32) -> Fact<Terrain> {
        Err(UNREAD.into())
    }
    fn crossing(&self, _from: u32, _to: u32) -> Fact<Option<u8>> {
        Err(UNREAD.into())
    }
    fn region(&self, province: u32) -> Fact<Option<u32>> {
        let regions = self.0.as_ref().map_err(Clone::clone)?;
        Ok(regions.get(&province).copied())
    }
    fn land_fort(&self, _building_type: u32) -> Fact<i32> {
        Err(UNREAD.into())
    }
    fn night_factor(&self, _province: u32) -> Fact<i64> {
        Err(UNREAD.into())
    }
    fn at_war(&self, _receiver: u32, _target: u32) -> Fact<bool> {
        Err(UNREAD.into())
    }
}

/// One candidate: the table written again and what it changed.
pub(super) struct Candidate {
    pub(super) step: Option<Step>,
    pub(super) variant: super::Variant,
    /// The previous hour's rows as this candidate writes them, by branch.
    previous: Option<(u64, Vec<DivisionStatusRow>)>,
    changed_rows: u64,
    factor_changes: u64,
    /// Region weather entries compared, those printed differently (shown
    /// or hidden, then text) and divisions whose entry could not be derived.
    compared: u64,
    presence: u64,
    text: u64,
    underivable: BTreeMap<String, u64>,
}

impl Candidate {
    pub(super) fn new(step: Option<i64>) -> Self {
        Self {
            step: step.map(|raw| Step::fixed5(raw, Rounding::TowardZero)),
            variant: super::Variant::new("weather_factor what-if", false, false),
            previous: None,
            changed_rows: 0,
            factor_changes: 0,
            compared: 0,
            presence: 0,
            text: 0,
            underivable: BTreeMap::new(),
        }
    }

    pub(super) fn push(
        &mut self,
        mut frame: DecodedFrame,
        keyframe: bool,
        content: &super::Content,
        regions: &Regions,
    ) -> std::io::Result<()> {
        if let Some(step) = self.step {
            self.compare_shown(&frame, step, regions);
        }
        if let TableView::Complete(rows) = &mut *frame.tables[TableId::DivisionStatus.index()]
            && let Some(rows) = rows.rows_mut::<tables::DivisionStatus>()
        {
            for row in rows.iter_mut() {
                if let Recorded::Value(Some(factor)) = &mut row.weather_factor {
                    *factor = stepped(*factor, self.step);
                }
            }
            self.count_changes(frame.branch, rows);
        } else {
            self.previous = None;
        }
        self.variant.push(frame, keyframe, content)
    }

    /// Rows and factors that differ from the same division's previous hour.
    fn count_changes(&mut self, branch: u64, rows: &[DivisionStatusRow]) {
        if let Some((previous_branch, previous)) = &self.previous
            && *previous_branch == branch
        {
            for row in rows {
                let Ok(at) = previous.binary_search_by_key(&row.division, |row| row.division)
                else {
                    continue;
                };
                if previous[at] != *row {
                    self.changed_rows += 1;
                }
                if previous[at].weather_factor != row.weather_factor {
                    self.factor_changes += 1;
                }
            }
        }
        self.previous = Some((branch, rows.to_vec()));
    }

    fn compare_shown(&mut self, frame: &DecodedFrame, step: Step, regions: &Regions) {
        let (Ok(statuses), Ok(divisions)) = (
            frame.complete::<tables::DivisionStatus>(),
            frame.complete::<tables::Divisions>(),
        ) else {
            return;
        };
        for status in statuses {
            let Recorded::Value(Some(factor)) = &status.weather_factor else {
                continue;
            };
            let Ok(at) = divisions.binary_search_by_key(&status.division, |row| row.id) else {
                continue;
            };
            let inputs = FrameInputs::new(frame, regions, status.division);
            match inputs.region_weather(divisions[at].province) {
                Ok(Some(pair)) if pair != [0, 0] => {
                    self.compared += 1;
                    let exact = shown(attack_derive::weather(pair, *factor));
                    let candidate =
                        shown(attack_derive::weather(pair, stepped(*factor, Some(step))));
                    if exact.is_some() != candidate.is_some() {
                        self.presence += 1;
                    } else if exact != candidate {
                        self.text += 1;
                    }
                }
                Ok(_) => {}
                Err(reason) => *self.underivable.entry(reason).or_default() += 1,
            }
        }
    }

    pub(super) fn print(&self, campaign: &[u8], hours: u64) -> std::io::Result<()> {
        let hours = hours.max(1) as f64;
        let bytes = self.variant.measure(campaign, TableId::DivisionStatus)?;
        let name = self.step.map_or("as recorded".to_owned(), |step| {
            format!("step {} raw", step.raw())
        });
        println!(
            "weather_factor ({name}): DivisionStatus {:.1} B/h, changed rows {:.2}/h, weather_factor changes {:.2}/h",
            bytes as f64 / hours,
            self.changed_rows as f64 / hours,
            self.factor_changes as f64 / hours,
        );
        if self.step.is_some() {
            println!(
                "  region weather entries at the division's own province: {:.2}/h compared, {:.4}/h shown or hidden differently, {:.4}/h printed differently",
                self.compared as f64 / hours,
                self.presence as f64 / hours,
                self.text as f64 / hours,
            );
            for (reason, count) in &self.underivable {
                println!("  underivable ({reason}): {count}");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `step` prints every factor's term as the exact factor does.
    fn keeps(attack: i64, step: i64) -> bool {
        let step = Some(Step::fixed5(step, Rounding::TowardZero));
        (0..=100_000).all(|factor| {
            let pair = [attack, attack];
            shown(attack_derive::weather(pair, factor))
                == shown(attack_derive::weather(pair, stepped(factor, step)))
        })
    }

    #[test]
    fn default_steps_keep_vanillas_single_weather_attack_terms() {
        for attack in [-10_000, -20_000, -40_000] {
            assert!(keeps(attack, DEFAULT_STEPS[0]), "{attack}");
        }
        for attack in [
            -5_000, -10_000, -20_000, -25_000, -40_000, -50_000, -100_000,
        ] {
            assert!(keeps(attack, DEFAULT_STEPS[1]), "{attack}");
        }
        // A coarser step, or a sum of entries, can print another digit.
        assert!(!keeps(-40_000, 50));
        assert!(!keeps(-60_000, DEFAULT_STEPS[0]));
    }

    #[test]
    fn shown_follows_the_native_sign_branch_and_two_decimal_percent() {
        assert_eq!(shown([0, 0]), None);
        assert_eq!(shown([0, -1]), Some((false, "0.00%".to_owned())));
        assert_eq!(shown([-12_345, 0]), Some((false, "-12.34%".to_owned())));
        assert_eq!(shown([12_345, 0]), Some((true, "12.34%".to_owned())));
        assert_eq!(stepped(-12_345, None), -12_345);
        assert_eq!(
            stepped(12_349, Some(Step::fixed5(25, Rounding::TowardZero))),
            12_325
        );
        let step = Some(Step::fixed5(25, Rounding::TowardZero));
        assert_eq!(stepped(24, step), 24);
        assert_eq!(stepped(-26, step), -25);
        assert_eq!(stepped(i64::MAX, step), i64::MAX);
    }

    #[test]
    fn steps_parse_defaults_and_refuse_what_no_display_step_takes() {
        let parse = |list| super::super::weather_factor_steps(Some(list));
        assert_eq!(parse("default").unwrap(), DEFAULT_STEPS);
        assert_eq!(parse("25, 5,100").unwrap(), [25, 5, 100]);
        for bad in ["", "0", "1", "3", "-5", "x", "25,"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn changes_count_rows_and_factors_against_the_same_branch() {
        let row = |division, factor| DivisionStatusRow {
            division,
            weather_factor: Recorded::Value(Some(factor)),
            ..Default::default()
        };
        let mut candidate = Candidate::new(Some(25));
        candidate.count_changes(1, &[row(1, 100), row(2, 200)]);
        candidate.count_changes(1, &[row(1, 100), row(2, 225), row(3, 0)]);
        assert_eq!((candidate.changed_rows, candidate.factor_changes), (1, 1));
        // Another branch starts again.
        candidate.count_changes(2, &[row(1, 0)]);
        assert_eq!((candidate.changed_rows, candidate.factor_changes), (1, 1));
    }
}
