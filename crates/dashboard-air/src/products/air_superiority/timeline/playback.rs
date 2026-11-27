use super::*;
use dashboard_follow::panel::{Panel, ProductStep};
use replay_frame::frame::TableView;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};

fn covers(input: ProductStep<'_>, table: TableId, columns: u64) -> bool {
    input.lanes.get(table.index()).is_some_and(|lane| {
        let held = lane.requested().iter().fold(0u64, |mask, &column| mask | (1 << column));
        held & columns == columns && (input.step.first() == input.step.last() || lane.passing())
    })
}

struct Held {
    piece: usize,
    through: i32,
    bucket_proven: bool,
    reply: AirTimelineReply,
}

impl Held {
    fn adjacent(&self, input: ProductStep<'_>) -> bool {
        let step = input.step;
        !step.reset()
            && step.piece() == Some(self.piece)
            && step.first().checked_sub(1) == Some(self.reply.last_hour)
    }

    fn continues(&self, hour: i32, input: ProductStep<'_>, columns: [(TableId, u64); 2], bucket_proven: bool) -> bool {
        let step = input.step;
        let previous = self.reply.last_hour;
        self.adjacent(input)
            && previous.div_euclid(DAY) == hour.div_euclid(DAY)
            && (hour < bucket_end(previous)
                || previous.checked_add(1) == Some(hour)
                    && if previous == bucket_end(previous) { self.bucket_proven } else { bucket_proven })
            && hour <= self.through
            && columns.into_iter().all(|(table, mask)| {
                covers(input, table, mask) && step.table(table.index()).is_some_and(|change| {
                    change.head() == Some(hour)
                        && !change.reset()
                        && change.status_left().is_none()
                        && change.inserted().is_empty()
                        && change.deleted().is_empty()
                        && change.passing().is_empty()
                        && change.columns().iter().all(|column| column.is_empty() || mask & (1 << column.column()) == 0)
                })
            })
    }

    fn date(&mut self, hour: i32) {
        self.reply.last_hour = hour;
        self.reply.days[0] = Some(hour);
        for row in &mut self.reply.rows {
            match &mut row.readout {
                Some(AirReadout::Leading { as_of_hour, .. }
                    | AirReadout::Contested { as_of_hour }
                    | AirReadout::Viewpoint { as_of_hour, .. }
                    | AirReadout::NoActivity { as_of_hour }) => *as_of_hour = hour,
                _ => {}
            }
        }
    }
}

pub fn panel(source: &Arc<Source>, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    let branch = dashboard_query::branch(asked).map_err(|error| error.to_string())?;
    let held = Mutex::new(None::<Held>);
    let styles = OnceLock::new();
    let side_index = OnceLock::new();
    let score_columns = at_hour::field_mask(TableId::RaidAirScores, &["friendly", "enemy"]);
    let link_columns = at_hour::field_mask(TableId::CountryLinks, &["canonical_slot"]);
    let columns = [(TableId::RaidAirScores, score_columns), (TableId::CountryLinks, link_columns)];
    at_hour::stepped_day_panel::<AirSuperiorityTimeline>(source, load, asked, ("firstHour", "lastHour"),
        |params, _| params.last_hour, |_, _| {},
        move |source, params, load, cancelled, mut step| {
            let mut held = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let previous = held.take();
            dashboard_text::metadata::check_cancel(cancelled)?;
            let hour = params.last_hour;
            let query = dashboard_base::queries::query(source, branch)?;
            let stretch = query.timeline.at(hour);
            let input = step.step.filter(|input| input.step.last() == hour);
            let reads = [Read::all(TableId::RaidAirScores), Read::all(TableId::CountryLinks)];
            let projected = if params.every != Some(true) && stretch.is_some() {
                Some(step.frame(source, &query, hour, &reads, AirSuperiorityAt::columns)?)
            } else {
                None
            };
            let complete = projected.is_some_and(|frame| reads.iter().all(|read| {
                matches!(frame.tables[read.table.index()].held(), TableView::Complete(_))
            }));
            let bucket_proven = complete && (hour.rem_euclid(BUCKET) == 0 || previous.as_ref().is_some_and(|prior| {
                prior.bucket_proven && prior.reply.last_hour.div_euclid(BUCKET) == hour.div_euclid(BUCKET)
                    && input.is_some_and(|input| prior.adjacent(input)
                        && covers(input, TableId::RaidAirScores, score_columns)
                        && input.step.table(TableId::RaidAirScores.index()).is_some_and(|change| {
                        change.head() == Some(hour) && !change.reset() && change.status_left().is_none()
                            && change.deleted().is_empty() && change.passing().is_empty()
                    }))
            }));
            if complete && let Some(input) = input
                && let Some(mut previous) = previous
                && previous.continues(hour, input, columns, bucket_proven)
            {
                previous.bucket_proven = bucket_proven;
                previous.date(hour);
                let reply = previous.reply.clone();
                *held = Some(previous);
                return Ok(reply);
            }

            let reply = timeline(source, params, load, projected, cancelled)?;
            if complete && params.theatres.as_ref().is_none_or(Vec::is_empty)
                && reply.days.first() == Some(&Some(hour))
                && let Some(stretch) = stretch
                && let Some(piece) = input.and_then(|input| input.step.piece())
                && let Ok(index) = side_index.get_or_init(|| index::index(source, branch))
            {
                let mut through = stretch.last.min(super::super::epoch_through(index, hour)).min(at_hour::before(query.holders.next_change(hour)));
                if !reply.sides.is_empty() {
                    let styles = styles.get_or_init(|| super::super::BranchSideResolver::load(source, &query, load, &false.into(), &mut Vec::new()));
                    through = through.min(styles.as_ref().map_or(hour, |styles| styles.through(hour, &reply.sides, None)));
                }
                *held = Some(Held { piece, through, bucket_proven, reply: reply.clone() });
            }
            Ok(reply)
        })
}
