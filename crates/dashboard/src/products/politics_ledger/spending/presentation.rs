use super::*;

pub(super) fn count_reason(
    reasons: &mut BTreeMap<String, u64>,
    reason: String,
    count: u64,
) -> io::Result<()> {
    let old = reasons.entry(reason).or_default();
    *old = old
        .checked_add(count)
        .ok_or_else(|| io::Error::other("Politics warning count overflow"))?;
    Ok(())
}

pub(super) fn intern_tooltip(
    item: &mut LedgerSpendItem,
    info: &mut Option<IdeaInfo>,
    ids: &mut BTreeMap<String, String>,
    tag: &str,
) {
    let Some(identity) = item.tooltip_token.take() else {
        return;
    };
    let next = format!("idea:{tag}:{}", ids.len());
    let token = ids.entry(identity).or_insert(next).clone();
    item.tooltip_token = Some(token.clone());
    if let Some(info) = info {
        info.token = token;
    }
}

/// A failed read does not erase adjacent reads that supplied exact coverage.
pub(super) fn invalidate_read(rows: &mut Vec<LedgerSpendCoverage>, hour: i32, reason: &str) {
    let Some(index) = rows.iter().position(|row| {
        row.since_hour
            .zip(row.through_hour)
            .is_some_and(|(first, last)| first <= hour && hour <= last)
    }) else {
        return;
    };
    let old = rows.remove(index);
    let (first, last) = (old.since_hour.unwrap(), old.through_hour.unwrap());
    let mut replacement = Vec::new();
    if first < hour {
        replacement.push(LedgerSpendCoverage {
            through_hour: Some(hour - 1),
            ..old.clone()
        });
    }
    replacement.push(LedgerSpendCoverage {
        read_hour: hour,
        since_hour: Some(hour),
        through_hour: Some(hour),
        political: false,
        reason: Some(reason.into()),
        ..old.clone()
    });
    if hour < last {
        replacement.push(LedgerSpendCoverage {
            since_hour: Some(hour + 1),
            ..old
        });
    }
    rows.splice(index..index, replacement);
}

fn gap(first: i32, last: i32, absent: bool) -> LedgerSpendCoverage {
    LedgerSpendCoverage {
        read_hour: first,
        since_hour: Some(first),
        through_hour: Some(last),
        political: absent,
        command: absent,
        reason: (!absent).then(|| "politics coverage unavailable".into()),
    }
}
/// A complete Countries census proves no payments by a tag absent that hour.
/// An unavailable census or a live country without its witness proves nothing.
pub(super) fn fill_gap(
    scan: &Scanned,
    tag: &str,
    first: i32,
    last: i32,
    rows: &mut Vec<LedgerSpendCoverage>,
) {
    let mut cursor = first;
    let index = scan.presence.partition_point(|run| run.last < first);
    for run in scan.presence[index..]
        .iter()
        .take_while(|run| run.first <= last)
    {
        let start = run.first.max(first);
        let end = run.last.min(last);
        if cursor < start {
            rows.push(gap(cursor, start - 1, false));
        }
        rows.push(gap(
            start,
            end,
            !run.tags.contains(tag) && !run.refused_tags.contains(tag),
        ));
        cursor = end.saturating_add(1);
    }
    if cursor <= last {
        rows.push(gap(cursor, last, false));
    }
}
