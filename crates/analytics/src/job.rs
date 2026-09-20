//! Resumable, multi-country projections over the authoritative replay decoder.

use std::{
    mem::size_of,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use replay::{
    container::Replay,
    frame::{Frame, FrameDecoder},
    rows::{CountryLinkRow, CountryRow},
};

use crate::{
    CountryMetadata, CountrySeries, CountrySeriesPoint, PointStatus, QueryError, QueryLimits,
    QueryRequest, QueryResult, SampleProvenance,
    projection::{ProjectedBlock, ProjectedFrame, ProjectionCache},
    query::QueryKey,
    reduce::{Reducer, Values},
    scan::{Catalogue, branch_metadata, country_tables, fill_hole, mapped_slot, validate_campaign},
};

pub enum ScanStep {
    Yielded,
    Ready(QueryResult),
}

struct BlockCursor {
    frames: Vec<Frame>,
    decoder: FrameDecoder,
    next: usize,
    last: i32,
    projected: Option<Vec<Arc<ProjectedFrame>>>,
}

/// One scan has one all-table decoder and compact reducers for every requested
/// country. It never retains complete decoded hours. A quantum can still include
/// one indivisible read_block/decompression; it is not a wall-clock deadline.
pub struct CountryScan {
    replay: Arc<Replay>,
    key: QueryKey,
    limits: QueryLimits,
    reducers: Vec<Reducer>,
    catalogue: Catalogue,
    catalogue_limit: usize,
    blocks: Vec<usize>,
    next_block: usize,
    block: Option<BlockCursor>,
    cached: Option<(Arc<ProjectedBlock>, usize)>,
    cursor: i64,
    done: bool,
}

impl CountryScan {
    pub fn start(
        replay: Arc<Replay>,
        request: &QueryRequest,
        limits: QueryLimits,
    ) -> Result<Self, QueryError> {
        let key = QueryKey::parse(request, &limits)?;
        Self::from_key(replay, key, limits)
    }

    pub(crate) fn from_key(
        replay: Arc<Replay>,
        key: QueryKey,
        limits: QueryLimits,
    ) -> Result<Self, QueryError> {
        validate_range(&replay, &key)?;
        let span = (i64::from(key.last) - i64::from(key.first) + 1) as u64;
        let buckets = span.div_ceil(span.div_ceil(u64::from(key.max_points))) as usize;
        // Reserve for twenty native i64 strings, an optional failure reason and
        // the complete point. Reducer storage is smaller than this DTO bound.
        let per_point = size_of::<CountrySeriesPoint>() + 20 * 32 + 512;
        let base = buckets
            .checked_mul(per_point)
            .and_then(|bytes| bytes.checked_add(size_of::<CountrySeries>() + 4096))
            .and_then(|bytes| bytes.checked_mul(key.countries.len()))
            .and_then(|bytes| bytes.checked_add(size_of::<QueryResult>() + 1024))
            .ok_or_else(|| QueryError::limit("Query output size overflow"))?;
        if base > limits.max_result_bytes {
            return Err(QueryError::limit("Query exceeds its output byte budget"));
        }
        // The legacy DTO repeats the catalogue in each country series. Bound it
        // before insertion, including the working catalogue and selected names.
        let entry_bytes = (size_of::<CountryMetadata>() + 2 * 512 + 128)
            .saturating_mul(key.countries.len().saturating_add(2));
        let catalogue_limit = (limits.max_result_bytes - base) / entry_bytes;
        let blocks = replay
            .index
            .iter()
            .enumerate()
            .filter(|(_, block)| {
                block.branch == key.branch
                    && block.last_hour >= key.first
                    && block.first_hour <= key.last
            })
            .map(|(index, _)| index)
            .collect();
        let reducers = key
            .countries
            .iter()
            .map(|_| Reducer::new(key.first, key.last, key.max_points))
            .collect();
        let cursor = i64::from(key.first);
        Ok(Self {
            replay,
            key,
            limits,
            reducers,
            catalogue: Catalogue::default(),
            catalogue_limit,
            blocks,
            next_block: 0,
            block: None,
            cached: None,
            cursor,
            done: false,
        })
    }

    pub fn at_block_boundary(&self) -> bool {
        self.block.is_none() && self.cached.is_none()
    }

    pub fn advance(
        &mut self,
        frame_quantum: NonZeroUsize,
        cancel: &AtomicBool,
    ) -> Result<ScanStep, QueryError> {
        self.advance_cached(frame_quantum, cancel, &ProjectionCache::new(0))
    }

    pub(crate) fn advance_cached(
        &mut self,
        frame_quantum: NonZeroUsize,
        cancel: &AtomicBool,
        cache: &ProjectionCache,
    ) -> Result<ScanStep, QueryError> {
        if self.done {
            return Err(QueryError::invalid("Query scan already finished"));
        }
        let mut visited = 0;
        while visited < frame_quantum.get() {
            check_cancel(cancel)?;
            if let Some((cached, next)) = &mut self.cached {
                let frame = cached
                    .frames
                    .get(*next)
                    .filter(|frame| frame.hour <= self.key.last);
                if let Some(frame) = frame {
                    if self.cursor < i64::from(frame.hour) {
                        for reducer in &mut self.reducers {
                            fill_hole(
                                &self.replay,
                                self.key.branch,
                                reducer,
                                self.cursor as i32,
                                frame.hour - 1,
                            );
                        }
                    }
                    if frame.hour >= self.key.first {
                        project(
                            frame.hour,
                            self.key.branch,
                            frame
                                .tables
                                .as_ref()
                                .map(|(rows, links)| (rows.as_slice(), links.as_slice()))
                                .map_err(String::as_str),
                            &self.key.countries,
                            &mut self.reducers,
                            &mut self.catalogue,
                            self.catalogue_limit,
                        )?;
                        self.cursor = i64::from(frame.hour) + 1;
                    }
                    *next += 1;
                } else {
                    let last = cached.through.min(self.key.last);
                    self.cached = None;
                    self.finish_block(last, cache, cancel);
                }
                visited += 1;
                continue;
            }
            if self.block.is_none() {
                let Some(&index) = self.blocks.get(self.next_block) else {
                    if self.cursor <= i64::from(self.key.last) {
                        self.hole(self.cursor as i32, self.key.last);
                    }
                    check_cancel(cancel)?;
                    self.done = true;
                    let result = self.finish()?;
                    check_cancel(cancel)?;
                    return Ok(ScanStep::Ready(result));
                };
                let block_index = index;
                let index = self.replay.index[index].clone();
                if self.cursor < i64::from(index.first_hour) {
                    self.hole(self.cursor as i32, index.first_hour - 1);
                    self.cursor = i64::from(index.first_hour);
                }
                if index.raw_length > self.limits.max_block_raw_bytes as u64 {
                    return Err(QueryError::limit(
                        "Replay block exceeds the admitted raw-block budget",
                    ));
                }
                let last = index.last_hour.min(self.key.last);
                if let Some(block) = cache.get(&self.key.source, &self.replay, block_index, last) {
                    self.cached = Some((block, 0));
                    continue;
                }
                cache.block_reads.fetch_add(1, Ordering::Relaxed);
                match self.replay.read_block(&index) {
                    Ok(frames) => {
                        check_cancel(cancel)?;
                        self.block = Some(BlockCursor {
                            frames,
                            decoder: FrameDecoder::new(),
                            next: 0,
                            last,
                            projected: Some(Vec::new()),
                        });
                    }
                    Err(error) => {
                        self.missing(
                            index.first_hour.max(self.key.first),
                            last,
                            PointStatus::Gap,
                            &format!("unreadable block: {error}"),
                        );
                        self.cursor = i64::from(last) + 1;
                        self.next_block += 1;
                        visited += 1;
                        continue;
                    }
                }
            }

            let block = self.block.as_ref().expect("block was loaded");
            let hour = block.frames.get(block.next).map(|frame| frame.hour);
            let last = block.last;
            let Some(hour) = hour.filter(|hour| *hour <= self.key.last) else {
                self.finish_block(last, cache, cancel);
                // A block containing no requested frames still consumes a turn.
                visited += 1;
                continue;
            };
            if self.cursor < i64::from(hour) {
                self.hole(self.cursor as i32, hour - 1);
            }
            let block = self.block.as_mut().expect("block was loaded");
            let frame = &block.frames[block.next];
            let decoded = block.decoder.advance(frame).and_then(|decoded| {
                validate_campaign(decoded, &self.replay)?;
                Ok(decoded)
            });
            block.next += 1;
            visited += 1;
            match decoded {
                Ok(decoded) => {
                    if let Some(projected) = &mut block.projected {
                        if let Some(frame) = cache.capture(decoded) {
                            projected.push(frame);
                        } else {
                            block.projected = None;
                        }
                    }
                    if hour >= self.key.first {
                        let tables = country_tables(decoded).map_err(|error| error.to_string());
                        project(
                            decoded.hour,
                            decoded.branch,
                            tables.as_ref().copied().map_err(String::as_str),
                            &self.key.countries,
                            &mut self.reducers,
                            &mut self.catalogue,
                            self.catalogue_limit,
                        )?;
                        self.cursor = i64::from(hour) + 1;
                    }
                }
                Err(error) => {
                    self.missing(
                        hour.max(self.key.first),
                        last,
                        PointStatus::Gap,
                        &format!("invalid frame baseline at hour {hour}: {error}"),
                    );
                    self.cursor = i64::from(last) + 1;
                    self.block = None;
                    self.next_block += 1;
                }
            }
            if self
                .block
                .as_ref()
                .is_some_and(|block| block.next == block.frames.len())
            {
                self.finish_block(last, cache, cancel);
            }
        }
        check_cancel(cancel)?;
        Ok(ScanStep::Yielded)
    }

    fn finish_block(&mut self, last: i32, cache: &ProjectionCache, cancel: &AtomicBool) {
        if self.cursor <= i64::from(last) {
            self.hole(self.cursor as i32, last);
            self.cursor = i64::from(last) + 1;
        }
        if let Some(block) = self.block.take()
            && let Some(frames) = block.projected
            && !cancel.load(Ordering::Relaxed)
        {
            cache.insert(
                self.key.source.clone(),
                &self.replay,
                self.blocks[self.next_block],
                ProjectedBlock {
                    frames,
                    through: last,
                },
            );
        }
        self.next_block += 1;
    }

    fn hole(&mut self, first: i32, last: i32) {
        for reducer in &mut self.reducers {
            fill_hole(&self.replay, self.key.branch, reducer, first, last);
        }
    }

    fn missing(&mut self, first: i32, last: i32, status: PointStatus, reason: &str) {
        for reducer in &mut self.reducers {
            reducer.missing(first, last, status, reason);
        }
    }

    fn finish(&mut self) -> Result<QueryResult, QueryError> {
        let branch = self
            .replay
            .branches
            .iter()
            .find(|branch| branch.id == self.key.branch)
            .expect("query branch was validated");
        let meta = branch_metadata(&self.replay, branch);
        let mut warnings = Vec::new();
        if !self.replay.content_verified() {
            warnings.push("Unverified recorded content identity; no local game assets were loaded or verified.".into());
        }
        let countries = std::mem::take(&mut self.catalogue).finish(&mut warnings);
        let country_ids: Vec<_> = self.key.countries.iter().map(u32::to_string).collect();
        let series = std::mem::take(&mut self.reducers)
            .into_iter()
            .zip(&country_ids)
            .map(|(reducer, country_id)| {
                let (points, coverage) = reducer.finish();
                CountrySeries {
                    branch_id: meta.id.clone(),
                    session_id: meta.session_id.clone(),
                    country_id: country_id.clone(),
                    first_hour: self.key.first,
                    last_hour: self.key.last,
                    country: countries
                        .iter()
                        .find(|country| country.id == *country_id)
                        .cloned(),
                    countries: countries.clone(),
                    points,
                    coverage,
                    downsampled: i64::from(self.key.last) - i64::from(self.key.first) + 1
                        > i64::from(self.key.max_points),
                    sample_boundary: meta.sample_boundary.clone(),
                    value_scale: "100000".into(),
                    warnings: warnings.clone(),
                }
            })
            .collect();
        let result = QueryResult {
            source_id: self.key.source.source_id.clone(),
            revision: self.key.source.revision.clone(),
            branch_id: meta.id,
            country_ids,
            first_hour: self.key.first,
            last_hour: self.key.last,
            kind: self.key.kind,
            max_points: self.key.max_points,
            series,
            sample_provenance: SampleProvenance::Unknown,
        };
        if result.retained_bytes() > self.limits.max_result_bytes {
            return Err(QueryError::limit(
                "Query result exceeds its output byte budget",
            ));
        }
        Ok(result)
    }
}

pub(crate) fn validate_range(replay: &Replay, key: &QueryKey) -> Result<(), QueryError> {
    let branch = replay
        .branches
        .iter()
        .find(|branch| branch.id == key.branch)
        .ok_or_else(|| QueryError::invalid("branchId is not present in this replay"))?;
    let meta = branch_metadata(replay, branch);
    if key.first < meta.first_hour || key.last > meta.last_hour {
        return Err(QueryError::invalid(
            "firstHour/lastHour must lie inside the branch",
        ));
    }
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), QueryError> {
    if cancel.load(Ordering::Relaxed) {
        Err(QueryError::cancelled())
    } else {
        Ok(())
    }
}

fn project(
    hour: i32,
    branch: u64,
    tables: Result<(&[CountryRow], &[CountryLinkRow]), &str>,
    requested: &[u32],
    reducers: &mut [Reducer],
    catalogue: &mut Catalogue,
    catalogue_limit: usize,
) -> Result<(), QueryError> {
    let (rows, links) = match tables {
        Ok(tables) => tables,
        Err(error) => {
            for reducer in reducers {
                reducer.missing(hour, hour, PointStatus::Unavailable, error);
            }
            return Ok(());
        }
    };
    let mut selected: Vec<Option<Result<Values, String>>> = vec![None; requested.len()];
    for row in rows {
        // Resolve each row once; share the answer with both the catalogue and
        // all selected country projections. Conflicts affect either named slot.
        let linked = links
            .binary_search_by_key(&row.id, |link| link.id)
            .ok()
            .map(|index| links[index].canonical_slot);
        let mapped = if linked == Some(row.canonical_slot) {
            Ok(row.canonical_slot)
        } else {
            mapped_slot(row, links).map_err(|error| error.to_string())
        };
        catalogue.observe_mapped(
            row,
            mapped.as_ref().ok().copied(),
            branch,
            hour,
            catalogue_limit,
        )?;
        let mut slots = [Some(row.canonical_slot), linked];
        if slots[0] == slots[1] {
            slots[1] = None;
        }
        for slot in slots.into_iter().flatten() {
            if let Ok(index) = requested.binary_search(&slot) {
                let value = match &mapped {
                    Ok(_) if selected[index].is_none() => Ok(Values::from_row(row)),
                    Ok(_) => {
                        Err("Multiple country rows resolve to the requested canonical slot".into())
                    }
                    Err(reason) => Err(reason.clone()),
                };
                selected[index] = Some(value);
            }
        }
    }
    for (reducer, value) in reducers.iter_mut().zip(selected) {
        match value {
            Some(Ok(values)) => reducer.record(hour, values),
            Some(Err(reason)) => reducer.missing(hour, hour, PointStatus::Unavailable, &reason),
            None => reducer.missing(
                hour,
                hour,
                PointStatus::Absent,
                "Country is absent from complete Countries and CountryLinks tables",
            ),
        }
    }
    Ok(())
}
