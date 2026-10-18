//! Compact block facts and proved windows in the shared observation cache.
use crate::{Source, metadata::check_cancel};
use replay::{container::BlockChange, container::BlockIndex, container::TableFields};
use replay_frame::frame::DecodedFrame;
use serde::{Serialize, de::DeserializeOwned};
use std::{collections::HashSet, io, sync::atomic::AtomicBool};

#[derive(Default)]
pub struct Keys {
    pub values: HashSet<u64>,
    pub available: bool,
}

/// Keep the requested proof, including an unproven result, as compressed
/// bytes under the shared cache's live-block bound and content binding.
pub fn cached<T: Serialize + DeserializeOwned>(
    source: &Source,
    proof: &'static str,
    branch: u64,
    window: usize,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
    read: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let block = source
        .replay_ref()
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .nth(window)
        .ok_or_else(|| crate::invalid("Newest proof window not in branch"))?;
    let cache = super::observations::scope(source, proof, Some(key));
    let bytes = cache.get(
        block,
        &window,
        cancelled,
        |bytes: &Vec<u8>| std::mem::size_of::<Vec<u8>>() + bytes.capacity(),
        || crate::prepared::series::compress(serde_json::to_vec(&read()?)?.as_slice(), 1),
    )?;
    serde_json::from_reader(zstd::stream::Decoder::new(bytes.as_slice())?).map_err(Into::into)
}

/// Change-read each earlier block once while retained. Shorter and longer
/// prefixes combine the same immutable block facts without storing a history.
pub fn keys(
    source: &Source,
    proof: &'static str,
    blocks: &[&BlockIndex],
    fields: &[TableFields],
    collect: impl Fn(&DecodedFrame, &mut Keys) + Sync,
    cancelled: &AtomicBool,
) -> io::Result<Keys> {
    if blocks.is_empty() {
        return Ok(Keys::default());
    }
    // Block workers cannot inherit the calling thread's decode refusal.
    super::progressive::decodes_allowed()?;
    let cache = super::observations::scope(source, proof, None);
    let read = |block: &BlockIndex| {
        cache.get(
            block,
            &(),
            cancelled,
            |facts: &Keys| {
                // Hash buckets hold keys and control bytes, plus a control
                // group; charge from capacity rather than key count.
                std::mem::size_of::<Keys>()
                    + facts.values.capacity().saturating_mul(2) * (std::mem::size_of::<u64>() + 1)
                    + std::mem::size_of::<u128>()
            },
            || {
                let mut facts = Keys::default();
                source.replay_ref().read_block_changes(
                    block,
                    fields,
                    |change| {
                        if let BlockChange::Changed { frame, .. } = change {
                            collect(frame, &mut facts);
                        }
                        Ok(())
                    },
                    || check_cancel(cancelled),
                )?;
                Ok(facts)
            },
        )
    };
    let mut keys = Keys::default();
    let ahead = std::thread::available_parallelism().map_or(1, |cores| (cores.get() / 2).max(1));
    dashboard_core::blocks::scan_blocks_by(blocks.to_vec(), ahead, cancelled, &read, |reads| {
        for (_, facts) in reads {
            let facts = facts?;
            keys.values.extend(facts.values.iter().copied());
            keys.available |= facts.available;
        }
        Ok(())
    })?;
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{continuation_records, file_source, replay, source as frame};
    use replay_data::{rows::CountryRow, rows::TableId, rows::TableRows, rows::tables};

    #[test]
    fn earlier_prefixes_reuse_block_facts_within_the_shared_live_block_bound() {
        let country = |hour, id| {
            let mut frame = frame(1, hour, None);
            *frame.tables[TableId::Countries.index()] =
                replay_frame::frame::TableView::Complete(TableRows::Countries(vec![CountryRow {
                    id,
                    ..Default::default()
                }]));
            frame
        };
        let recorded = replay(vec![country(0, 7)]);
        let recorded = recorded
            .append(&[continuation_records(&recorded, 1, vec![country(1, 9)])])
            .unwrap();
        let (source, root) = file_source(recorded);
        let blocks = source.replay_ref().index.iter().collect::<Vec<_>>();
        let cancelled = AtomicBool::new(false);
        let read = |blocks: &[&BlockIndex]| {
            keys(
                &source,
                "test:earlier",
                blocks,
                &[TableFields::keys(TableId::Countries)],
                |frame, keys| {
                    if let Ok(rows) = frame.complete::<tables::Countries>() {
                        keys.values.extend(rows.iter().map(|row| u64::from(row.id)));
                        keys.available = true;
                    }
                },
                &cancelled,
            )
            .unwrap()
        };
        assert_eq!(read(&blocks[..1]).values, HashSet::from([7]));
        assert_eq!(read(&blocks).values, HashSet::from([7, 9]));
        let (bytes, bound, computations) =
            super::super::observations::usage(&source, "test:earlier");
        assert!(bytes > 0 && bytes <= bound);
        assert_eq!(computations, 2);
        assert!(read(&blocks[..1]).available);
        assert_eq!(read(&blocks).values, HashSet::from([7, 9]));
        assert_eq!(
            super::super::observations::usage(&source, "test:earlier").2,
            2
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn proved_and_unproven_windows_share_retention_and_rebind_on_content_changes() {
        let (source, root) = file_source(replay(vec![frame(1, 0, None)]));
        let content = root.join("content.txt");
        std::fs::write(&content, "first").unwrap();
        let key = store::SourceKey::capture(source.path(), &[content.clone()]).unwrap();
        let flag = AtomicBool::new(false);
        let first: Option<Vec<u32>> =
            cached(&source, "test:proof", 1, 0, &key, &flag, || Ok(None)).unwrap();
        assert!(first.is_none());
        let (again, refused) = super::super::progressive::without_block_decodes(|| {
            cached::<Option<Vec<u32>>>(&source, "test:proof", 1, 0, &key, &flag, || {
                panic!("an unproven result is retained too")
            })
        });
        assert!(!refused);
        assert!(again.unwrap().is_none());
        std::fs::write(&content, "changed").unwrap();
        let next = key.recapture().unwrap();
        let proved = cached(&source, "test:proof", 1, 0, &next, &flag, || {
            Ok(Some(vec![7u32]))
        })
        .unwrap();
        assert_eq!(proved, Some(vec![7]));
        let (bytes, bound, computations) = super::super::observations::usage(&source, "test:proof");
        assert!(bytes > 0 && bytes <= bound);
        assert_eq!(computations, 2);
        let _ = std::fs::remove_dir_all(root);
    }
}
