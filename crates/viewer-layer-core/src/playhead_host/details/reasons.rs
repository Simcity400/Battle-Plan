use super::super::{Entry, lock};
use playhead::pages::Pages;
use replay_columnar::unit::reader;
use replay_data::rows::TableId;
use std::{collections::HashMap, io, sync::{Arc, Condvar, Mutex, Weak, atomic::{AtomicU64, Ordering}}, time::{Duration, Instant}};

static READ: AtomicU64 = AtomicU64::new(0);

pub(crate) fn landed() -> u64 {
    READ.load(Ordering::Acquire)
}

pub const BEING_READ: &str = "its reason is being read from the recording";

const NOT_IN_TIME: &str = "its reason could not be read in time";

enum Text {
    Words(String),
    Unheld(u32),
    Failed(String),
}

#[derive(Default)]
struct Reason {
    text: Mutex<Option<Text>>,
    read: Condvar,
}

impl Reason {
    fn set(&self, read: io::Result<Option<Vec<u8>>>, id: u32) {
        let text = match read {
            Ok(Some(bytes)) => Text::Words(String::from_utf8_lossy(&bytes).into_owned()),
            Ok(None) => Text::Unheld(id),
            Err(error) => Text::Failed(error.to_string()),
        };
        *lock(&self.text) = Some(text);
        READ.fetch_add(1, Ordering::AcqRel);
        self.read.notify_all();
    }

    fn words(&self, wait: Duration) -> String {
        let deadline = Instant::now().checked_add(wait);
        let mut text = lock(&self.text);
        while text.is_none() {
            text = if let Some(deadline) = deadline {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                self.read.wait_timeout(text, left).unwrap_or_else(std::sync::PoisonError::into_inner).0
            } else {
                self.read.wait(text).unwrap_or_else(std::sync::PoisonError::into_inner)
            };
        }
        match &*text {
            None if wait.is_zero() => BEING_READ.to_owned(),
            None => NOT_IN_TIME.to_owned(),
            Some(Text::Words(words)) => words.clone(),
            Some(Text::Unheld(id)) => format!("the recording holds no text for its reason (string {id})"),
            Some(Text::Failed(error)) => format!("its reason cannot be read from the recording ({error})"),
        }
    }
}

type Held = (Weak<dyn Pages>, HashMap<(TableId, u32), Arc<Reason>>);

static REASONS: Mutex<Vec<Held>> = Mutex::new(Vec::new());

fn reason(pages: &Arc<dyn Pages>, table: TableId, id: u32, (run, block): (usize, usize)) -> Arc<Reason> {
    let mut held = lock(&REASONS);
    held.retain(|(pages, _)| pages.strong_count() > 0);
    let at = match held.iter().position(|(of, _)| std::ptr::addr_eq(of.as_ptr(), Arc::as_ptr(pages))) {
        Some(at) => at,
        None => {
            held.push((Arc::downgrade(pages), HashMap::new()));
            held.len() - 1
        }
    };
    if let Some(reason) = held[at].1.get(&(table, id)) {
        return Arc::clone(reason);
    }
    let reason = Arc::new(Reason::default());
    held[at].1.insert((table, id), Arc::clone(&reason));
    drop(held);
    let (pages, slot) = (Arc::clone(pages), Arc::clone(&reason));
    let spawned = std::thread::Builder::new().name("limitation reason".into()).spawn(move || {
        viewer_runtime::worker_priority::background_priority();
        slot.set(read(&*pages, run, block, table.index(), id), id);
    });
    if let Err(error) = spawned {
        reason.set(Err(error), id);
    }
    reason
}

fn read(pages: &dyn Pages, run: usize, block: usize, table: usize, id: u32) -> io::Result<Option<Vec<u8>>> {
    let held = pages.runs().get(run).ok_or_else(|| io::Error::other(format!("no run {run}")))?;
    let kinds = pages.kinds(table).ok_or_else(|| io::Error::other(format!("table {table} is withheld")))?;
    let kinds = &kinds[..pages.held(table).min(kinds.len())];
    let source = pages.extents(run);
    let block = block.min(held.blocks().saturating_sub(1));
    let restored = (0..block)
        .rev()
        .find_map(|at| {
            let entry = pages.table_entry(run, at, table).ok()?;
            entry.restore.is_some().then_some((at + 1, entry, false))
        })
        .or_else(|| {
            (0..=block).rev().find_map(|at| {
                let entry = pages.table_entry(run, at, table).ok()?;
                entry.first.is_some().then_some((at, entry, true))
            })
        });
    let from = match restored {
        Some((from, entry, is_first)) => {
            let restore = if is_first { entry.first.as_ref().expect("first") } else { entry.restore.as_ref().expect("restore") };
            let (_, meta) = reader::restore(source, restore, kinds, &[])?;
            if let Some((_, text)) = meta.strings.into_iter().find(|(named, _)| *named == id) {
                return Ok(Some(text));
            }
            from
        }
        None => 0,
    };
    for at in from..=block {
        let Some(hours) = held.block_hours(at) else { continue };
        let Ok(entry) = pages.table_entry(run, at, table) else { continue };
        let mut prefix = None;
        for (at, part) in entry.parts.iter().enumerate() {
            let last = entry.parts.get(at + 1).map_or(hours - 1, |next| next.first - 1);
            let part = reader::fetch(source, part, last, &[], prefix.as_ref())?;
            if let Some((_, text)) = part.strings().iter().find(|(named, _)| *named == id) {
                return Ok(Some(text.clone()));
            }
            prefix = Some(part.prefix());
        }
    }
    Ok(None)
}

pub(super) fn words(entry: &Entry, table: TableId, id: u32, place: (usize, usize), wait: Duration) -> String {
    reason(entry.pages(), table, id, place).words(wait)
}

#[cfg(test)]
mod tests {
    use super::{BEING_READ, NOT_IN_TIME, Reason};
    use crate::{test_support::limitations::REASON};
    use std::time::Duration;

    #[test]
    fn a_reason_still_unread_is_being_read_on_a_frame_and_not_read_in_time_in_a_report() {
        let reason = Reason::default();
        assert_eq!(reason.words(Duration::ZERO), BEING_READ, "a frame never waits");
        assert_eq!(reason.words(Duration::from_nanos(1)), NOT_IN_TIME, "a report that waited says so by name");
        reason.set(Ok(Some(REASON.as_bytes().to_vec())), 7);
        assert_eq!(reason.words(Duration::ZERO), REASON);
        assert_eq!(reason.words(Duration::MAX), REASON, "an indefinite wait still returns a cached reason");
    }
}
