use super::*;

/// The tables the line output cross-check decodes along the branch.
pub(super) const LINE_OUTPUT_TABLES: [TableId; 3] = [
    TableId::ProductionLineOutput,
    TableId::IndustrialOrganisationHistory,
    TableId::IndustrialOrganisations,
];

/// The save keeps no per-line count, so ProductionLineOutput is checked through
/// the organisation history, which the game credits in the same call the
/// branch's observation began whose frame read both tables and the compared
/// hour, each (organisation, variant) history slot must change by the change
/// of the lines' `mio_credit` for that pair. Each row counts with its last
/// publication up to the compared hour, ended rows included, less its value at
/// R. The history at the compared hour is compared with the save separately.
pub(in crate::oracle) fn compare_line_output(
    replay: &Replay,
    frame: &DecodedFrame,
    report: &mut Report,
) -> io::Result<()> {
    report.limitation(
        "the save keeps no per-line output; each line's credits are checked against its organisation's history, and its delivered units are not saved at all",
        "ProductionLineOutput",
    );
    report.limitation(
        "country_reports.equipment_production also counts refits and missiles, and a type of several categories in each, so it is at most an approximate check of the line totals and is not compared",
        "ProductionLineOutput",
    );
    let rows = complete::<tables::ProductionLineOutput>(frame)?;
    // Every row of a branch shares its observation start. After the last
    // line ended and its final rows were dropped, the table is empty, but
    // the earlier publications still hold credits the history must match.
    let since = match rows.first() {
        Some(row) => Some(row.since_hour),
        None => observation_start(replay, frame)?,
    };
    let Some(since) = since else {
        report.limitation(
            "no production line output was published on this branch up to this hour",
            "ProductionLineOutput",
        );
        return Ok(());
    };
    if let Some(gap) = replay.gaps.iter().find(|gap| {
        gap.branch == frame.branch && gap.first_hour <= frame.hour && gap.last_hour >= since
    }) {
        report.limitation(
            "a gap since the observation began may hide a publication of an ended line; the organisation credits are not cross-checked",
            format!(
                "ProductionLineOutput: hours {}..={}: {}",
                gap.first_hour, gap.last_hour, gap.reason
            ),
        );
        return Ok(());
    }
    let mut blocks = replay
        .index
        .iter()
        .filter(|block| {
            block.branch == frame.branch
                && block.first_hour <= frame.hour
                && block.last_hour >= since
        })
        .collect::<Vec<_>>();
    blocks.sort_by_key(|block| block.first_hour);
    let mut check = CreditCheck::new(since);
    for block in blocks {
        // Every block starts with a keyframe.
        let mut decoder = FrameDecoder::for_tables(&LINE_OUTPUT_TABLES);
        for encoded in replay.read_block_selected(block, &LINE_OUTPUT_TABLES)? {
            if encoded.hour > frame.hour {
                break;
            }
            let decoded = decoder.advance(&encoded)?;
            if decoded.hour >= since {
                check.observe(decoded)?;
            }
        }
    }
    check.finish(frame.hour, report)
}

/// The observation start of the branch's latest publication with rows at or
/// before the frame's hour, searching its blocks from the latest back; `None`
/// when no publication up to that hour held a row.
fn observation_start(replay: &Replay, frame: &DecodedFrame) -> io::Result<Option<Hour>> {
    let mut blocks = replay
        .index
        .iter()
        .filter(|block| block.branch == frame.branch && block.first_hour <= frame.hour)
        .collect::<Vec<_>>();
    blocks.sort_by_key(|block| std::cmp::Reverse(block.first_hour));
    for block in blocks {
        let mut decoder = FrameDecoder::for_tables(&LINE_OUTPUT_TABLES);
        let mut since = None;
        for encoded in replay.read_block_selected(block, &LINE_OUTPUT_TABLES)? {
            if encoded.hour > frame.hour {
                break;
            }
            let decoded = decoder.advance(&encoded)?;
            if let Some(row) = complete::<tables::ProductionLineOutput>(decoded)
                .ok()
                .and_then(|rows| rows.first())
            {
                since = Some(row.since_hour);
            }
        }
        if since.is_some() {
            return Ok(since);
        }
    }
    Ok(None)
}

/// ProductionLineOutput and the organisation history along one branch, from
/// the observation start to the compared hour.
pub(super) struct CreditCheck {
    since: Hour,
    /// R, with its history and the rows' credits by (organisation, variant).
    base: Option<(Hour, History, Credits)>,
    /// Each row's last publication.
    last: BTreeMap<(u64, u32), ProductionLineOutputRow>,
    /// The latest hour whose frame read the history, with it.
    latest: Option<(Hour, History)>,
    /// Organisations recorded at R or later.
    organisations: BTreeSet<u64>,
}

/// Organisation history units by (organisation, variant).
type History = BTreeMap<(u64, u64), i32>;
/// Line output credits by (organisation, variant).
type Credits = BTreeMap<(u64, u64), i64>;

impl CreditCheck {
    pub(super) fn new(since: Hour) -> Self {
        Self {
            since,
            base: None,
            last: BTreeMap::new(),
            latest: None,
            organisations: BTreeSet::new(),
        }
    }

    /// A frame at or after the observation start. Only frames that read the
    /// lines' daily slot published the table; carried hours repeat the last
    /// read.
    pub(super) fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        if !super::daily_read(frame.hour, &frame.metrics, TableId::ProductionLineOutput) {
            return Ok(());
        }
        // An unpublished hour keeps its ended lines for the next publication.
        let Ok(rows) = complete::<tables::ProductionLineOutput>(frame) else {
            return Ok(());
        };
        if rows.iter().any(|row| row.since_hour != self.since) {
            return Err(invalid(
                "ProductionLineOutput restarted its observation within the branch",
            ));
        }
        let history = complete::<tables::IndustrialOrganisationHistory>(frame)
            .ok()
            .map(|history| {
                history
                    .iter()
                    .map(|row| ((row.organisation, row.variant), row.units))
                    .collect::<History>()
            });
        if self.base.is_none() {
            let Some(history) = &history else {
                return Ok(());
            };
            self.base = Some((frame.hour, history.clone(), credits(rows.iter())));
        }
        for row in rows {
            self.last.insert((row.line, row.ordinal), row.clone());
        }
        if let Ok(organisations) = complete::<tables::IndustrialOrganisations>(frame) {
            self.organisations
                .extend(organisations.iter().map(|row| row.organisation));
        }
        if let Some(history) = history {
            self.latest = Some((frame.hour, history));
        }
        Ok(())
    }

    pub(super) fn finish(self, hour: Hour, report: &mut Report) -> io::Result<()> {
        let Some((base_hour, base_history, base_credits)) = self.base else {
            report.limitation(
                "no hour since the observation began read both ProductionLineOutput and IndustrialOrganisationHistory",
                "ProductionLineOutput",
            );
            return Ok(());
        };
        let history = match self.latest {
            Some((latest, history)) if latest == hour => history,
            _ => {
                return Err(invalid(
                    "IndustrialOrganisationHistory was not read at the compared hour",
                ));
            }
        };
        let mut recorded = credits(self.last.values());
        for (key, credit) in base_credits {
            *recorded.entry(key).or_default() -= credit;
        }
        // A line without an organisation credits none.
        let mut keys = recorded
            .keys()
            .filter(|(organisation, _)| *organisation != 0)
            .copied()
            .collect::<BTreeSet<_>>();
        keys.extend(base_history.keys().chain(history.keys()));
        let mut matched = 0;
        for key @ (organisation, variant) in keys {
            // Reduce the accumulated i64 credit modulo 2^32, then interpret
            // it as the same signed native counter as the history delta.
            let credited = recorded.get(&key).copied().unwrap_or(0) as i32;
            let before = base_history.get(&key).copied().unwrap_or(0);
            // The history slot is a wrapping i32.
            let counted = history.get(&key).copied().unwrap_or(0).wrapping_sub(before);
            if credited == counted {
                matched += 1;
            } else if !history.contains_key(&key)
                && !base_history.contains_key(&key)
                && !self.organisations.contains(&organisation)
            {
                report.limitation(
                    "the lines credited an organisation that no read recorded, so its history cannot be checked",
                    format!("organisation {organisation} variant {variant}"),
                );
            } else {
                report.difference(
                    format!(
                        "organisation {organisation}.history {variant} units credited since hour {base_hour}"
                    ),
                    credited,
                    counted,
                );
            }
        }
        let inexact = self.last.values().filter(|row| !row.exact).count();
        if inexact != 0 {
            report.limitation(
                "a production tick of these rows could not be counted exactly, so their delivered units leave it out; their credits are still exact",
                format!("ProductionLineOutput: {inexact} rows"),
            );
        }
        report.compared("ProductionLineOutput", matched);
        Ok(())
    }
}

/// The rows' organisation credits by (organisation, variant).
fn credits<'a>(rows: impl Iterator<Item = &'a ProductionLineOutputRow>) -> Credits {
    let mut credits = Credits::new();
    for row in rows {
        *credits.entry((row.mio, row.variant)).or_default() += row.mio_credit;
    }
    credits
}
