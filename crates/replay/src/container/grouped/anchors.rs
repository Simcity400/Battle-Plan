use super::*;
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct TableAnchors {
    snapshot: Option<Arc<Baseline>>,
    latest: Option<Arc<Baseline>>,
    pub bootstrap_reads: AnchorReads,
    local: bool,
    ordinal: Option<usize>,
}

struct Baseline {
    position: usize,
    ordinal: usize,
    index: BlockIndex,
    state: AnchorState,
    root: (usize, usize),
    charge: usize,
    depth: usize,
}

pub(super) struct Keyframe {
    pub update: TableUpdate,
    pub plain: Vec<u8>,
    pub shadow: Shadow,
    pub marks: Option<Vec<u8>>,
    pub anchored: bool,
}

impl TableAnchors {
    pub(crate) fn local() -> Self {
        Self {
            local: true,
            ..Self::default()
        }
    }

    pub(super) fn begin(
        &mut self,
        replay: &Replay,
        block: &chunks::BlockChunks,
        table: TableId,
        raw: &[u8],
        budget: &Budget,
        check: &mut Check<'_>,
    ) -> io::Result<Keyframe> {
        #[cfg(test)]
        let _memory = super::super::reader::ownership::category(
            super::super::reader::ownership::Kind::Bootstrap,
        );
        check.run()?;
        let format = replay.format;
        let version = format.version();
        let change = &HISTORY[table.index()];
        let fields = change.fields(table, version);
        let discarded = change.table_gap(table, version).is_some();
        let layout = block.directory.layout;
        let from = From {
            position: Some(block.position),
            index: &block.index,
            layout,
        };
        let mut input = ColumnReader::stored(raw, fields, table, version)?;
        let marks = input.1.map(engine_allocation::copy_vec).transpose()?;
        let mut plain = Vec::new();
        let head = take_head(&mut input, None, block.index.first_hour, &mut plain, layout)?;
        let mut shadow = Shadow::new();
        let mut rows = None;
        let mut root = (block.position, raw.len());
        let mut charge = 0;
        let mut depth = 0;
        if let Some(anchor) = head.anchor {
            let at = match anchor {
                AnchorRef::Back(back) => replay.ordinals[block.position]
                    .checked_sub(back)
                    .and_then(|ordinal| replay.ordinals.binary_search(&ordinal).ok()),
            }
            .ok_or_else(|| invalid_static("keyframe anchor names no earlier block of its file"))?;
            let retained = self.snapshot.iter().chain(self.latest.iter()).find(|base| {
                base.position == at
                    && base.ordinal == replay.ordinals[at]
                    && base.index == replay.index[at]
            });
            let bound = || invalid_static("keyframe anchor chain exceeds its size bound");
            if let Some(base) = retained {
                root = base.root;
                depth = base.depth + 1;
                charge = base
                    .charge
                    .checked_add(raw.len())
                    .and_then(|value| value.checked_add(root.1 / REBUILD_SHARE))
                    .ok_or_else(bound)?;
                let links = charge
                    .checked_sub(
                        depth
                            .checked_mul(root.1 / REBUILD_SHARE)
                            .ok_or_else(bound)?,
                    )
                    .ok_or_else(bound)?;
                if links > MAX_BLOCK_RAW_BYTES || (layout.anchors_back() && charge > root.1) {
                    return Err(bound());
                }
                rows = base
                    .state
                    .rows
                    .as_ref()
                    .map(TableRows::try_clone)
                    .transpose()?;
                shadow = base.state.shadow.clone();
            } else {
                if !self.local {
                    return Err(invalid_static(
                        "keyframe anchor is not the latest snapshot or latest keyframe",
                    ));
                }
                let resolved = anchored_keyframe_counted(
                    replay,
                    &from,
                    table,
                    fields,
                    discarded,
                    anchor,
                    raw.len(),
                    format,
                    false,
                    check,
                    &mut self.bootstrap_reads,
                )?;
                root = (resolved.root.0, resolved.root.1);
                charge = resolved
                    .charge
                    .checked_add(raw.len())
                    .and_then(|value| value.checked_add(root.1 / REBUILD_SHARE))
                    .ok_or_else(bound)?;
                depth = resolved.depth + 1;
                rows = resolved.state.rows;
                shadow = resolved.state.shadow;
            }
        }
        take_body_checked(
            table,
            fields,
            head.status,
            &mut input,
            &shadow,
            &mut plain,
            budget,
            layout,
            None,
            check,
        )?;
        input.finish()?;
        let mut wire = Decoder::new(&plain);
        let update = {
            #[cfg(test)]
            let _memory = super::super::reader::ownership::category(
                super::super::reader::ownership::Kind::Updates,
            );
            decode_update(table, change, format, head.anchor.is_none(), &mut wire)?
        };
        wire.finish()?;
        let update = match rows {
            Some(rows) => TableUpdate::Snapshot(apply_anchor_change(rows, update)?),
            None => update,
        };
        advance_shadow(table, version, &plain, &mut shadow)?;
        if !self.local
            && (matches!(update, TableUpdate::Snapshot(_))
                || (discarded && (head.status == SNAPSHOT || head.anchor.is_some())))
        {
            #[cfg(test)]
            let _memory = super::super::reader::ownership::category(
                super::super::reader::ownership::Kind::Anchors,
            );
            let rows = match &update {
                TableUpdate::Snapshot(rows) => Some(rows.try_clone()?),
                _ => None,
            };
            let base = Arc::new(Baseline {
                position: block.position,
                ordinal: replay.ordinals[block.position],
                index: block.index.clone(),
                state: AnchorState {
                    rows,
                    shadow: shadow.clone(),
                },
                root,
                charge,
                depth,
            });
            if head.anchor.is_none() {
                self.snapshot = Some(Arc::clone(&base));
            }
            self.latest = Some(base);
        }
        Ok(Keyframe {
            update,
            plain,
            shadow,
            marks,
            anchored: head.anchor.is_some(),
        })
    }

    pub(crate) fn admit_ordinal(&mut self, ordinal: usize) {
        if self.ordinal.and_then(|previous| previous.checked_add(1)) != Some(ordinal) {
            self.release();
        }
        self.ordinal = Some(ordinal);
    }

    pub(crate) fn release(&mut self) {
        self.snapshot = None;
        self.latest = None;
    }

    pub(crate) fn retained(&self) -> usize {
        match (&self.snapshot, &self.latest) {
            (Some(a), Some(b)) => 1 + usize::from(!Arc::ptr_eq(a, b)),
            (None, None) => 0,
            _ => 1,
        }
    }
}
