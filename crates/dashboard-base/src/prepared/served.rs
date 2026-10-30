//! How a product's served replies are checked between two preparations of one
//! recording: the requests its [`Plan`] asks of both, chosen from the
//! recording's timeline ([`Branch`]) and the reference store's own replies.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;

/// Hours a plan reads each branch at, at most.
pub const HOURS: usize = 24;
/// Windows a plan reads of each branch, at most.
pub const WINDOWS: usize = 12;
/// Keys a plan reads one at a time on each branch, beside its every-key reads.
pub const KEYS: usize = 8;
/// Keys one request names at most.
pub const BATCH: usize = 64;
/// Pages one cursor is followed through at most.
pub const PAGES: usize = 256;

/// A product's served-reply check.
pub struct Plan {
    /// Asks every request to compare, in an order fixed by the recording and
    /// the reference store's replies.
    pub queries: fn(&mut dyn Ask) -> io::Result<()>,
    /// Arrays of its replies' data whose order means nothing, by their path
    /// from the data: `*` is every element or member.
    pub unordered: &'static [&'static [&'static str]],
}

/// The reference store, as a plan asks it.
pub trait Ask {
    /// The recording's branches.
    fn branches(&self) -> &[Branch];
    /// `product`'s reply data to `params`, kept to compare; None when the
    /// store refused it.
    fn ask(&mut self, product: &str, params: Value) -> io::Result<Option<Value>>;
    /// Every country `branch`'s identity history names, in tag order.
    fn countries(&mut self, branch: u64) -> io::Result<Vec<String>>;
}

/// One branch of the recording, as plans choose its hours, windows and spans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub id: u64,
    pub first_hour: i32,
    pub last_hour: i32,
    /// Its replay blocks' hours, in order: the windows products page by.
    pub windows: Vec<(i32, i32)>,
    /// Its spans the recording holds no hour of.
    pub gaps: Vec<(i32, i32)>,
    /// Its columnar segments' first hours, when the check was given them.
    pub segments: Vec<i32>,
}

impl Branch {
    /// Its id as requests name it.
    pub fn name(&self) -> String {
        self.id.to_string()
    }

    /// The hours to read it at, [`HOURS`] at most: its first and last, both
    /// sides of some of its window edges and segment edges, the hours around
    /// some of its gaps, and hours spread between.
    pub fn hours(&self) -> Vec<i32> {
        let (first, last) = (self.first_hour, self.last_hour);
        let edges: Vec<[i32; 2]> = self
            .windows
            .windows(2)
            .map(|pair| [pair[0].1, pair[1].0])
            .collect();
        let gaps: Vec<[i32; 3]> = self
            .gaps
            .iter()
            .map(|&(from, to)| [from.saturating_sub(1), from, to.saturating_add(1)])
            .collect();
        let segments: Vec<[i32; 2]> = self
            .segments
            .iter()
            .map(|&from| [from.saturating_sub(1), from])
            .collect();
        let mut hours = vec![first, last];
        hours.extend(spread(&edges, HOURS / 8).into_iter().flatten());
        hours.extend(spread(&gaps, HOURS / 12).into_iter().flatten());
        hours.extend(spread(&segments, HOURS / 8).into_iter().flatten());
        hours.retain(|hour| (first..=last).contains(hour));
        hours.sort_unstable();
        hours.dedup();
        let between = HOURS.saturating_sub(hours.len()) as i64;
        let length = i64::from(last) - i64::from(first);
        for at in 1..=between {
            hours.push((i64::from(first) + length * at / (between + 1)) as i32);
        }
        hours.sort_unstable();
        hours.dedup();
        hours
    }

    /// The spans to read it over: all of it, its first and last hour alone, a
    /// day, a week and a month, a span across a window edge, a gap and a span
    /// over it, and the empty spans just before and after it.
    pub fn spans(&self) -> Vec<(i32, i32)> {
        let (first, last) = (self.first_hour, self.last_hour);
        let lasting = |hour: i32, hours: i32| (hour, hour.saturating_add(hours - 1).min(last));
        let middle = self.middle();
        let mut spans = vec![
            (first, last),
            (first, first),
            (last, last),
            lasting(first, 24),
            lasting(middle, 24 * 7),
            lasting(middle.saturating_sub(24 * 15).max(first), 24 * 30),
        ];
        let at = self.windows.len() / 2;
        if at > 0 {
            let (end, start) = (self.windows[at - 1].1, self.windows[at].0);
            spans.push((
                end.saturating_sub(12).max(first),
                start.saturating_add(12).min(last),
            ));
        }
        if let Some(&(from, to)) = self.gaps.first() {
            spans.push((from, to));
            spans.push((
                from.saturating_sub(24).max(first),
                to.saturating_add(24).min(last),
            ));
        }
        spans.push((first.saturating_sub(48), first.saturating_sub(1)));
        spans.push((last.saturating_add(1), last.saturating_add(48)));
        let mut unique = Vec::with_capacity(spans.len());
        for span in spans {
            if !unique.contains(&span) {
                unique.push(span);
            }
        }
        unique
    }

    /// The ordinals of the windows to read: those holding [`Self::hours`],
    /// and [`WINDOWS`] more spread over it.
    pub fn window_indices(&self) -> Vec<usize> {
        let mut picked = indices(self.windows.len());
        for hour in self.hours() {
            picked.extend(
                self.windows
                    .iter()
                    .position(|&(first, last)| (first..=last).contains(&hour)),
            );
        }
        picked.sort_unstable();
        picked.dedup();
        picked
    }

    /// Its middle hour.
    pub fn middle(&self) -> i32 {
        (i64::from(self.first_hour) + (i64::from(self.last_hour) - i64::from(self.first_hour)) / 2)
            as i32
    }
}

/// `count` of `items` spread evenly over them, the first and last among them;
/// all of them when there are no more.
pub fn spread<T: Clone>(items: &[T], count: usize) -> Vec<T> {
    if items.len() <= count {
        return items.to_vec();
    }
    match count {
        0 => Vec::new(),
        1 => vec![items[0].clone()],
        _ => (0..count)
            .map(|at| items[at * (items.len() - 1) / (count - 1)].clone())
            .collect(),
    }
}

/// [`WINDOWS`] ordinals of `count` windows, spread over them.
pub fn indices(count: usize) -> Vec<usize> {
    spread(&(0..count).collect::<Vec<_>>(), WINDOWS)
}

/// [`KEYS`] of `keys`, spread over them.
pub fn some(keys: &[String]) -> Vec<String> {
    spread(keys, KEYS)
}

/// `keys` in requests of [`BATCH`] keys at most; none for no keys.
pub fn batches(keys: &[String]) -> Vec<Vec<String>> {
    keys.chunks(BATCH).map(<[String]>::to_vec).collect()
}

/// Every value at `path` below `value`, in order: `*` is every element or member.
pub fn values_at<'a>(value: &'a Value, path: &[&str]) -> Vec<&'a Value> {
    let mut found = Vec::new();
    collect(value, path, &mut found);
    found
}

fn collect<'a>(value: &'a Value, path: &[&str], found: &mut Vec<&'a Value>) {
    match path.split_first() {
        None => found.push(value),
        Some((&"*", rest)) => match value {
            Value::Array(items) => {
                for item in items {
                    collect(item, rest, found);
                }
            }
            Value::Object(members) => {
                for item in members.values() {
                    collect(item, rest, found);
                }
            }
            _ => {}
        },
        Some((key, rest)) => {
            if let Some(next) = value.get(*key) {
                collect(next, rest, found);
            }
        }
    }
}

/// Every text at `path` below `value`, sorted and once each.
pub fn texts_at(value: &Value, path: &[&str]) -> Vec<String> {
    let mut texts: Vec<String> = values_at(value, path)
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    texts.sort_unstable();
    texts.dedup();
    texts
}

/// Asks `product` with `params`, then each next page its reply's `next`
/// cursor names, as `cursor`, until none does or [`PAGES`] are read.
pub fn pages(ask: &mut dyn Ask, product: &str, mut params: Value, next: &str) -> io::Result<()> {
    for _ in 0..PAGES {
        let Some(reply) = ask.ask(product, params.clone())? else {
            return Ok(());
        };
        match reply.get(next).and_then(Value::as_str) {
            Some(cursor) => params["cursor"] = Value::from(cursor),
            None => return Ok(()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch() -> Branch {
        Branch {
            id: 3,
            first_hour: 100,
            last_hour: 1099,
            windows: (0..10).map(|at| (100 + at * 100, 199 + at * 100)).collect(),
            gaps: vec![(450, 459)],
            segments: vec![300, 700],
        }
    }

    #[test]
    fn hours_keep_the_ends_and_both_sides_of_each_kind_of_edge() {
        let hours = branch().hours();
        assert!(hours.len() <= HOURS);
        for hour in [
            100, 1099, 199, 200, 999, 1000, 449, 450, 460, 299, 300, 699, 700,
        ] {
            assert!(hours.contains(&hour), "{hour} missing from {hours:?}");
        }
        assert!(hours.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(hours, branch().hours());
    }

    #[test]
    fn spans_include_the_whole_branch_and_empty_spans_on_each_side() {
        let spans = branch().spans();
        assert_eq!(spans[0], (100, 1099));
        assert!(spans.contains(&(52, 99)));
        assert!(spans.contains(&(1100, 1147)));
        assert!(spans.contains(&(450, 459)));
        assert!(spans.contains(&(587, 612)));
    }

    #[test]
    fn spread_keeps_the_first_and_last() {
        let items: Vec<usize> = (0..100).collect();
        let picked = spread(&items, 5);
        assert_eq!(picked.first(), Some(&0));
        assert_eq!(picked.last(), Some(&99));
        assert_eq!(picked.len(), 5);
        assert_eq!(spread(&items[..3], 5), vec![0, 1, 2]);
    }

    #[test]
    fn values_at_walks_arrays_and_members() {
        let value = serde_json::json!({"series": [{"tag": "GER"}, {"tag": "ENG"}, {"tag": "GER"}]});
        assert_eq!(
            texts_at(&value, &["series", "*", "tag"]),
            vec!["ENG", "GER"]
        );
        assert_eq!(values_at(&value, &["series", "*"]).len(), 3);
    }
}
