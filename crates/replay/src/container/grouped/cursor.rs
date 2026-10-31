use super::*;
use engine_allocation as allocation;

pub(crate) struct DiagnosticsCursor {
    raw: Vec<u8>,
    offset: usize,
    reads: Vec<u8>,
    pool: u64,
    dropped: u64,
    tables: Vec<TableId>,
}

impl DiagnosticsCursor {
    pub(super) fn new(
        raw: Vec<u8>,
        present: &Present,
        frames: usize,
        check: &mut Check<'_>,
    ) -> io::Result<Self> {
        let mut cursor = Self {
            raw,
            offset: 0,
            reads: allocation::filled(present.width(), 0)?,
            pool: 0,
            dropped: 0,
            tables: allocation::copy_vec(&present.tables)?,
        };
        for _ in 0..frames {
            cursor.next(check)?;
        }
        if cursor.offset != cursor.raw.len() {
            return Err(invalid_static("trailing bytes"));
        }
        cursor.offset = 0;
        cursor.reads.fill(0);
        cursor.pool = 0;
        cursor.dropped = 0;
        Ok(cursor)
    }

    pub(crate) fn next(&mut self, check: &mut Check<'_>) -> io::Result<Metrics> {
        check.run()?;
        let mut input = Decoder::new(&self.raw[self.offset..]);
        let u64_from = |value: i128| {
            u64::try_from(value).map_err(|_| invalid_static("grouped metric outside its range"))
        };
        let total_ns = u64_from(i128::try_from(input.varint()?).unwrap_or(-1))?;
        self.pool = metric_after(self.pool, input.varint()?)?;
        self.dropped = metric_after(self.dropped, input.varint()?)?;
        for (byte, change) in self
            .reads
            .iter_mut()
            .zip(input.take(self.tables.len().div_ceil(8))?)
        {
            *byte ^= change;
        }
        let tail = self.tables.len() % 8;
        if tail != 0 && self.reads[self.reads.len() - 1] >> tail != 0 {
            return Err(invalid_static("grouped read bits name an absent table"));
        }
        let mut table_ns = [0; TABLE_COUNT];
        for (ordinal, table) in self.tables.iter().enumerate() {
            if !get(&self.reads, ordinal) {
                continue;
            }
            table_ns[table.index()] = READ_TIME_NOT_RECORDED;
        }
        self.offset = self.raw.len() - input.remaining();
        Ok(Metrics {
            total_ns,
            table_ns,
            pool_bytes: self.pool,
            dropped_frames: self.dropped,
        })
    }
}

pub(crate) struct TableCursor {
    table: TableId,
    format: FormatVersion,
    mode: CaptureMode,
    layout: Layout,
    fields: usize,
    gap: Option<String>,
    later: chunks::Columns,
    chunk: usize,
    chunks: usize,
    coding_pending: bool,
    shadow: Shadow,
    predictor: predicted::Predictor,
    keys: KeySet,
    masks: MaskList,
    model: Option<Model>,
    changes: KeyChanges,
    plain: Vec<u8>,
    budget: Budget,
    first: Hour,
    next_at: usize,
    branch: BranchId,
    _credit: chunks::Credit,
}

impl TableCursor {
    pub(crate) fn begin(
        replay: &Replay,
        block: &mut chunks::BlockChunks,
        table: TableId,
        anchors: &mut anchors::TableAnchors,
        check: &mut Check<'_>,
    ) -> io::Result<(Self, TableUpdate, Option<Vec<u8>>)> {
        #[cfg(test)]
        let _step = super::super::check::Scope::enter(super::super::check::Step::Raw(table, 0));
        check.run()?;
        let format = replay.format;
        let fields = HISTORY[table.index()].fields(table, format.version());
        let key = block
            .take_key(table)
            .ok_or_else(|| invalid(format!("grouped block has no keyframe entry for {table:?}")))?;
        key.validate(keyframe_section(table), 0, 1)?;
        let later = block.take_later(table);
        let stored_later = later.is_some();
        let nonempty = later.as_ref().is_some_and(|chunk| !chunk.bytes.is_empty());
        let mut later = chunks::Columns::new(later, table, fields, format.version())?;
        let layout = block.directory.layout;
        let modelled = nonempty && later.with(|input| input.modelled_rows(table, layout))?;
        let budget = Budget(AtomicUsize::new(MAX_BLOCK_RAW_BYTES));
        let anchors::Keyframe {
            update,
            plain,
            mut shadow,
            marks,
            anchored,
        } = anchors.begin(replay, block, table, &key.bytes, &budget, check)?;
        let mut keys = KeySet::default();
        if !anchored && layout.keyed() {
            keys.advance(table, format.version(), &plain)?;
        }
        settle_advance(
            table,
            layout,
            &mut shadow,
            match &update {
                TableUpdate::Snapshot(rows) => Some(rows),
                _ => None,
            },
        );
        Ok((
            Self {
                table,
                format,
                mode: replay.capture_mode,
                layout,
                fields,
                gap: HISTORY[table.index()].table_gap(table, format.version()),
                later,
                chunk: 0,
                chunks: block.later_count(table),
                coding_pending: stored_later && !nonempty,
                shadow,
                predictor: predicted::Predictor::default(),
                keys,
                masks: MaskList::default(),
                model: modelled.then(|| Model::new(fields)),
                changes: KeyChanges::default(),
                plain,
                budget,
                first: block.hours[0],
                next_at: 1,
                branch: block.index.branch,
                _credit: block.credit(),
            },
            update,
            marks,
        ))
    }

    pub(crate) fn prepare(
        &mut self,
        replay: &Replay,
        block: &mut chunks::BlockChunks,
        at: usize,
        alias: Option<&mut super::super::aliases::private::streaming::Resolution>,
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        let hour = block.hours[at];
        let target = usize::try_from(i64::from(hour) - i64::from(self.first) - 1)
            .map_err(|_| invalid_static("invalid chunk hour range"))?
            / LATER_CHUNK_HOURS;
        let mut alias = alias;
        while self.chunk < target && self.chunk + 1 < self.chunks {
            self.later.finish()?;
            if let Some(alias) = &mut alias {
                alias.finish_chunk(self.later.marks())?;
            }
            // Retire the owner before reading its successor; codec state stays in this cursor.
            self.later =
                chunks::Columns::new(None, self.table, self.fields, self.format.version())?;
            check.run()?;
            self.chunk += 1;
            let next = block.load_later(replay, self.table, self.chunk, check)?;
            if next.as_ref().is_some_and(|chunk| chunk.slot != self.chunk) {
                return Err(invalid_static(
                    "later chunk slot differs from its descriptor",
                ));
            }
            let nonempty = next.as_ref().is_some_and(|chunk| !chunk.bytes.is_empty());
            self.later =
                chunks::Columns::new(next, self.table, self.fields, self.format.version())?;
            if self.coding_pending && nonempty {
                self.model = self
                    .later
                    .with(|input| input.modelled_rows(self.table, self.layout))?
                    .then(|| Model::new(self.fields));
                self.coding_pending = false;
            }
        }
        Ok(())
    }

    pub(crate) fn next(
        &mut self,
        previous: Hour,
        entered: bool,
        check: &mut Check<'_>,
    ) -> io::Result<TableUpdate> {
        #[cfg(test)]
        let _memory =
            super::super::reader::ownership::category(super::super::reader::ownership::Kind::Codec);
        let _cpu = crate::cpu::scope("decode", None);
        let _timing = Timing {
            table: self.table,
            branch: self.branch,
            first: self.first,
            started: crate::cpu::enabled().then(crate::cpu::thread_cycles),
        };
        #[cfg(test)]
        let _step = super::super::check::Scope::enter(super::super::check::Step::Raw(
            self.table,
            self.next_at,
        ));
        check.run()?;
        self.next_at += 1;
        let Self {
            table,
            format,
            mode,
            layout,
            fields,
            gap,
            later,
            shadow,
            predictor,
            keys,
            masks,
            model,
            changes,
            plain,
            budget,
            first,
            ..
        } = self;
        let (table, format, mode, layout, fields) = (*table, *format, *mode, *layout, *fields);
        let version = format.version();
        let decode = |plain: &[u8]| -> io::Result<TableUpdate> {
            #[cfg(test)]
            let _memory = super::super::reader::ownership::category(
                super::super::reader::ownership::Kind::Updates,
            );
            let _cpu = crate::cpu::scope("decode_wire", None);
            let mut input = Decoder::new(plain);
            let update =
                decode_recorded_update(table, &HISTORY[table.index()], format, mode, false, &mut input)?;
            input.finish()?;
            Ok(update)
        };
        if !entered {
            if predictor.predicts() {
                plain.clear();
                predicted::take_residual_checked(
                    table, fields, previous, None, predictor, shadow, plain, budget, layout, None,
                    check,
                )?;
                let update = decode(plain)?;
                advance_shadow(table, version, plain, shadow)?;
                return Ok(update);
            }
            predictor.clear();
            budget.spend(5)?;
            return Ok(match gap {
                Some(reason) => TableUpdate::Incompatible {
                    reason: allocation::string(reason)?,
                    source_format: format,
                    recorded_bytes: 5,
                },
                None => TableUpdate::Unchanged { baseline: previous },
            });
        }
        plain.clear();
        later.with(|input| {
            let head = take_head(input, Some(previous), *first, plain, layout)?;
            let coding = layout.keyed().then_some(RowCoding {
                keys,
                masks,
                model: model.as_mut(),
                changes: Some(changes),
            });
            if head.residual {
                predicted::take_residual_checked(
                    table,
                    fields,
                    previous,
                    Some(input),
                    predictor,
                    shadow,
                    plain,
                    budget,
                    layout,
                    coding,
                    check,
                )?;
            } else {
                predictor.clear();
                take_body_checked(
                    table,
                    fields,
                    head.status,
                    input,
                    shadow,
                    plain,
                    budget,
                    layout,
                    coding,
                    check,
                )?;
            }
            let update = decode(plain)?;
            advance_shadow(table, version, plain, shadow)?;
            if layout.keyed() {
                if head.status == DELTA {
                    keys.apply(changes)?;
                } else {
                    keys.advance(table, version, plain)?;
                }
            }
            Ok(update)
        })
    }

    #[cfg(test)]
    pub(super) fn raw_owner(&self) -> std::sync::Weak<Vec<u8>> {
        self.later.owner()
    }

    pub(crate) fn marks(&self) -> Option<&[u8]> {
        self.later.marks()
    }
    pub(crate) fn finish(&self) -> io::Result<()> {
        if self.coding_pending {
            return Err(invalid_static("later section contains only empty chunks"));
        }
        self.later.finish()
    }
}

struct Timing {
    table: TableId,
    branch: BranchId,
    first: Hour,
    started: Option<u64>,
}

impl Drop for Timing {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            crate::cpu::decoded(
                self.branch,
                self.first,
                self.table,
                crate::cpu::thread_cycles().saturating_sub(started),
            );
        }
    }
}
