use super::*;
use std::io::BufRead;

pub(super) fn read_replies(path: &Path) -> io::Result<BTreeMap<String, ServedReply>> {
    let mut replies = BTreeMap::new();
    for line in io::BufReader::new(File::open(path)?).lines() {
        let value: Value = serde_json::from_str(&line?)?;
        if value.get("request").is_none() {
            continue;
        }
        let reply: ServedReply = serde_json::from_value(value)?;
        reply.validate()?;
        if replies.insert(reply.label.clone(), reply).is_some() {
            return Err(invalid("Duplicate served request label"));
        }
    }
    if replies.is_empty() {
        return Err(invalid("No served requests in baseline"));
    }
    Ok(replies)
}

pub(super) fn import_store(
    source: &Source,
    from: &Path,
    evidence: &Path,
) -> io::Result<(PathBuf, Vec<u8>)> {
    let from = fs::canonicalize(from)?;
    let before = fs::read(from.join("manifest"))?;
    let id = dashboard_base::prepared::recording_identity(source.path())?;
    let destination = store::local_root()?.join(id.to_string());
    fs::create_dir_all(&destination)?;
    if fs::read_dir(&destination)?.next().is_some() {
        return Err(invalid(
            "Imported-store acceptance requires an empty isolated prepared cache",
        ));
    }
    for entry in fs::read_dir(&from)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(invalid(
                "Imported store contains a directory or filesystem link",
            ));
        }
        if entry.file_name() == "writer.lock" {
            continue;
        }
        fs::copy(entry.path(), destination.join(entry.file_name()))?;
    }
    if before != fs::read(from.join("manifest"))?
        || before != fs::read(destination.join("manifest"))?
    {
        return Err(invalid("Prepared manifest changed during import"));
    }
    fs::write(evidence.join("prepared-manifest"), &before)?;
    Ok((destination.join("manifest"), before))
}

pub(super) fn run(
    source: &Source,
    context: &LoadContext,
    requests: &Requests,
    evidence: &mut Evidence,
    repetitions: usize,
) -> io::Result<()> {
    use dashboard::products::production_lines::DETAIL_HOURS;
    let branch = requests
        .branch_id
        .as_ref()
        .ok_or_else(|| invalid("Missing branch"))?;
    let windows: BTreeSet<_> = requests
        .hours
        .iter()
        .map(|hour| hour.div_euclid(DETAIL_HOURS))
        .collect();
    let factors = [
        "efficiency",
        "start",
        "cap",
        "gain",
        "daily",
        "retention-0",
        "retention-1",
        "retention-2",
        "retention-3",
        "output",
        "power",
        "resources",
        "conversion",
        "refit",
    ];
    if requests.production_line_stride == 0 {
        return Err(invalid("Production-line key stride must be positive"));
    }
    let origin_hours = if requests.production_line_origin_hours.is_empty() {
        &requests.hours
    } else {
        if requests
            .production_line_origin_hours
            .iter()
            .any(|hour| !requests.hours.contains(hour))
        {
            return Err(invalid(
                "Production-line origin hour is absent from the request hours",
            ));
        }
        &requests.production_line_origin_hours
    };
    let mut selection = Vec::new();
    let mut lines_seen = 0;
    for tag in &requests.tags {
        let mut lines = BTreeSet::new();
        for exact in [None, Some(false), Some(true)] {
            let mut params = json!({"branchId":branch,"tag":tag});
            if let Some(exact) = exact {
                params["exact"] = json!(exact);
            }
            let value = evidence.query(
                source,
                context,
                "production-lines",
                params,
                &format!("production-lines/{tag}/exact-{exact:?}"),
                repetitions,
            )?;
            let entries = value["lines"]
                .as_array()
                .ok_or_else(|| invalid("Production lines unavailable"))?;
            for line in entries {
                let key = line["key"]
                    .as_str()
                    .ok_or_else(|| invalid("Line lacks key"))?;
                let (handle, appearance) = key
                    .split_once('#')
                    .ok_or_else(|| invalid("Invalid native line key"))?;
                lines.insert((
                    handle
                        .parse::<u64>()
                        .map_err(|_| invalid("Invalid native line handle"))?,
                    appearance
                        .parse::<u32>()
                        .map_err(|_| invalid("Invalid native line appearance"))?,
                    key.to_owned(),
                ));
            }
        }
        for window in &windows {
            evidence.query(
                source,
                context,
                "production-line-detail",
                json!({"branchId":branch,"tag":tag,"window":window}),
                &format!("production-lines/{tag}/detail/{window}"),
                repetitions,
            )?;
        }
        let count = lines.len();
        let keys: Vec<_> = lines
            .into_iter()
            .enumerate()
            .filter_map(|(at, (_, _, key))| {
                (requests.production_line_all_keys.contains(tag)
                    || at % requests.production_line_stride == 0
                    || at + 1 == count)
                    .then_some(key)
            })
            .collect();
        lines_seen += keys.len();
        selection.push(json!({"tag":tag,"nativeKeys":count,"selectedKeys":keys,
            "breakdownRequests":keys.len() * (factors.len() + origin_hours.len())}));
        for key in keys {
            for factor in factors {
                evidence.query(
                    source,
                    context,
                    "production-line-breakdown",
                    json!({"branchId":branch,"tag":tag,"key":key,"group":factor}),
                    &format!("production-lines/{tag}/breakdown/{key}/{factor}/literal"),
                    repetitions,
                )?;
            }
            for hour in origin_hours {
                evidence.query(source, context, "production-line-breakdown",
                    json!({"branchId":branch,"tag":tag,"key":key,"group":"output","originHour":hour}),
                    &format!("production-lines/{tag}/breakdown/{key}/output/origin-{hour}"), repetitions)?;
            }
        }
    }
    fs::write(
        evidence.path.join("production-line-selection.json"),
        serde_json::to_vec_pretty(&selection)?,
    )?;
    if lines_seen == 0 {
        return Err(invalid(
            "Request set exercised no production lines or modifier histories",
        ));
    }
    Ok(())
}
