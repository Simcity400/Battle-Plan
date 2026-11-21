use super::*;
use crate::frame::endpoint_step::EndpointStep;

impl FrameDecoder {
    pub fn resume_for_tables(tables: &[TableId], mut frame: DecodedFrame) -> io::Result<Self> {
        let mut decoder = Self::for_tables(tables);
        let selected = decoder
            .selected
            .as_ref()
            .expect("table selection initialized");
        for table in TableId::ALL {
            let view = &frame.tables[table.index()];
            match &**view {
                TableView::Complete(rows) => validate_complete(table, rows, frame.hour)?,
                TableView::Truncated { rows, missing } => {
                    validate_rows(table, rows)?;
                    if *missing == 0 {
                        return Err(invalid("truncated checkpoint has no missing rows"));
                    }
                }
                TableView::Unavailable { reason } => {
                    if reason.is_empty() {
                        return Err(invalid("unavailable checkpoint needs a reason"));
                    }
                }
                TableView::NotRecorded { .. } => {}
            }
            if view
                .sampled_at()
                .is_some_and(|hour| hour > frame.hour || !matches!(&**view, TableView::Complete(_)))
            {
                return Err(invalid("invalid checkpoint sample hour"));
            }
            if !selected[table.index()] {
                frame.tables[table.index()] = not_selected();
            } else {
                frame.tables[table.index()].2 = None;
                frame.tables[table.index()].3 = None;
            }
        }
        decoder.current = Some(Arc::new(frame));
        Ok(decoder)
    }

    pub fn resume_for_tables_with_change_facts(
        tables: &[TableId],
        frame: DecodedFrame,
    ) -> io::Result<Self> {
        let mut decoder = Self::resume_for_tables(tables, frame)?.with_change_facts();
        let frame = Arc::get_mut(decoder.current.as_mut().expect("resumed snapshot"))
            .expect("unpublished resumed snapshot");
        for table in tables {
            frame.tables[table.index()].3 = Some(super::field_stamp(None, None, frame.hour));
        }
        Ok(decoder)
    }

    pub fn advance_endpoint(&mut self, step: EndpointStep) -> io::Result<&DecodedFrame> {
        self.advance_endpoint_parallel(step, 1, &|| {})
    }

    pub fn advance_endpoint_parallel(
        &mut self,
        step: EndpointStep,
        workers: usize,
        on_worker: &(dyn Fn() + Sync),
    ) -> io::Result<&DecodedFrame> {
        self.advance_endpoint_parallel_cancellable(step, workers, on_worker, &|| false)
    }

    pub fn advance_endpoint_parallel_cancellable(
        &mut self,
        step: EndpointStep,
        workers: usize,
        on_worker: &(dyn Fn() + Sync),
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<&DecodedFrame> {
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let selected = self.selected.as_ref();
        let tables =
            (0..TABLE_COUNT).filter(|&index| selected.is_none_or(|selected| selected[index]));
        if !step.tables().map(TableId::index).eq(tables) {
            return Err(invalid("endpoint tables differ from the decoder's"));
        }
        if step.from
            != self
                .current
                .as_deref()
                .map(|frame| crate::frame::endpoint_step::Point {
                    branch: frame.branch,
                    hour: frame.hour,
                })
        {
            return Err(invalid("endpoint baseline differs"));
        }
        let prepared = if self.change_facts {
            super::reclaim_changes(&mut self.current, &step.replaced_change_tables());
            Some(step.prepare_change_facts(self.current.as_deref())?)
        } else {
            None
        };
        let mut reached = step.apply_shared_parallel(self.current.take(), workers, on_worker)?;
        if let Some((mut changes, mut stamps)) = prepared {
            for index in 0..TABLE_COUNT {
                let view = &mut reached.tables[index];
                if matches!(&**view, TableView::Complete(_)) {
                    if let Some(stamp) = stamps[index].take() {
                        view.2 = changes[index].take();
                        view.3 = Some(stamp);
                    }
                }
            }
        }
        self.current = Some(Arc::new(reached));
        Ok(self.current.as_deref().unwrap())
    }

    pub fn current(&self) -> Option<&DecodedFrame> {
        self.current.as_deref()
    }
}
