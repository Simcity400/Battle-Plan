use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    log::occurrence::{Answer, Direction, Order},
    series::PAGE_LIMIT,
};
use serde::{Deserialize, Serialize};
use std::{
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccurrenceSort {
    pub column: String,
    #[serde(default)]
    pub direction: Direction,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OccurrenceCount {
    pub key: String,
    pub total: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub missing: Option<String>,
}

pub trait OccurrenceReply: Clone + Serialize + Send + Sync + 'static {
    fn next_change(&self) -> Option<i32>;
    fn row_count(&self) -> usize;
}

pub fn cutoff(query: &Query<'_>, hour: Option<i32>) -> io::Result<i32> {
    let hour = hour.unwrap_or(query.timeline.last);
    if hour < query.timeline.first {
        return Err(invalid("The hour asked is before the branch"));
    }
    Ok(hour.min(query.timeline.last))
}

pub fn window(offset: Option<&str>, limit: u32) -> io::Result<(u64, u32)> {
    let offset = offset.unwrap_or("0").parse::<u64>().map_err(|_| invalid("Invalid offset"))?;
    if !(1..=PAGE_LIMIT).contains(&limit) {
        return Err(invalid("Invalid window limit"));
    }
    Ok((offset, limit))
}

pub fn order(sort: Option<&OccurrenceSort>, columns: &[&str]) -> io::Result<(Order, bool)> {
    match sort {
        None => Ok((Order::Date(Direction::Descending), true)),
        Some(sort) if sort.column == "date" => Ok((Order::Date(sort.direction), sort.direction == Direction::Descending)),
        Some(sort) => columns
            .iter()
            .position(|name| *name == sort.column)
            .map(|at| (Order::Key(at, sort.direction), false))
            .ok_or_else(|| invalid(format!("This log cannot be sorted by {}", sort.column))),
    }
}

pub fn counts(answer: &Answer, keys: &[&str]) -> Vec<OccurrenceCount> {
    keys.iter().zip(&answer.chips).map(|(key, total)| OccurrenceCount { key: (*key).to_owned(), total: total.to_string(), missing: None }).collect()
}

pub fn occurrence_panel<R, F>(executable: Option<engine_executable::executable_tables::Tables>, finish: super::Finish, read: F) -> Box<dyn Panel>
where
    R: OccurrenceReply + super::ReplyWarnings,
    F: Fn(i32, &AtomicBool) -> io::Result<R> + Send + Sync + 'static,
{
    super::at_hour::interval_panel(executable, move |hour, cancelled| {
        let reply = finish.reply(|| read(hour, cancelled))?;
        let through = reply.next_change().map_or(i32::MAX, |next| next.saturating_sub(1));
        let rows = reply.row_count() as u64;
        Ok((serde_json::to_value(reply)?, hour..=through, rows))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    #[derive(Clone, Serialize)]
    struct StubReply {
        hour: i32,
        next_change: Option<i32>,
        rows: usize,
        warnings: Vec<String>,
    }

    crate::reply_warnings!(StubReply);

    impl OccurrenceReply for StubReply {
        fn next_change(&self) -> Option<i32> {
            self.next_change
        }

        fn row_count(&self) -> usize {
            self.rows
        }
    }

    #[test]
    fn occurrence_panel_reuses_held_reply_across_quiet_forward_steps_and_rereads_at_boundary_seed_or_seek_back() {
        let calls = Arc::new(AtomicUsize::new(0));
        let reader_calls = Arc::clone(&calls);
        let panel = occurrence_panel(None, crate::products::Finish::default(), move |hour, _cancelled| {
            reader_calls.fetch_add(1, Ordering::SeqCst);
            let next_change = if hour < 20 { Some(20) } else { None };
            Ok(StubReply { hour, next_change, rows: 3, warnings: Vec::new() })
        });
        let exact = panel.exact().expect("exact reader");
        let cancel = AtomicBool::new(false);

        let initial = (exact.advance)(10, true, &cancel, None);
        assert!(initial.fresh);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        for quiet_hour in 11..20 {
            let step = (exact.advance)(quiet_hour, false, &cancel, None);
            assert!(!step.fresh, "quiet hour {quiet_hour} must return fresh=false");
            assert_eq!(step.rows, 3);
            assert!(Arc::ptr_eq(&step.value, &initial.value));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "quiet forward steps inside proved interval must not call inner reader");

        let at_boundary = (exact.advance)(20, false, &cancel, None);
        assert!(at_boundary.fresh);
        assert_eq!(calls.load(Ordering::SeqCst), 2, "boundary hour must call inner reader once");

        for tail_hour in 21..=25 {
            let step = (exact.advance)(tail_hour, false, &cancel, None);
            assert!(!step.fresh);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2, "unbounded proved tail must not call inner reader");

        let seek_back = (exact.advance)(15, false, &cancel, None);
        assert!(seek_back.fresh);
        assert_eq!(calls.load(Ordering::SeqCst), 3, "backward seek must read afresh");

        let jumped = (exact.advance)(25, false, &cancel, None);
        assert!(jumped.fresh);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        let reseed = (exact.advance)(26, true, &cancel, None);
        assert!(reseed.fresh);
        assert_eq!(calls.load(Ordering::SeqCst), 5, "seed delivery must read afresh");
    }

    #[test]
    fn a_reply_that_does_not_prove_its_own_hour_is_never_held() {
        let calls = Arc::new(AtomicUsize::new(0));
        let reader_calls = Arc::clone(&calls);
        let panel = occurrence_panel(None, move |hour, _cancelled| {
            reader_calls.fetch_add(1, Ordering::SeqCst);
            Ok(StubReply { hour, next_change: Some(hour), rows: 1 })
        });
        let exact = panel.exact().expect("exact reader");
        let cancel = AtomicBool::new(false);
        let first = (exact.advance)(10, true, &cancel, None);
        let again = (exact.advance)(10, false, &cancel, None);
        assert_eq!(calls.load(Ordering::SeqCst), 2, "an unsettled reply is read again at the same hour");
        assert!(again.fresh);
        assert_eq!((first.from, again.from), (10, 10));
    }
}
