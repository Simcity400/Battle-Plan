//! Prepared idea holdings, sampled lanes and an exact held set at an hour.
use super::*;
use crate::prepared::{
    prefix,
    query::{keyframes, progressive},
};

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdeaHistoryQuery {
    pub branch_id: String,
    pub tag: String,
    pub through: Option<i32>,
    pub hour: Option<i32>,
    pub preview: Option<bool>,
}

struct Observation {
    scanned: ScannedIdeas,
    slot: u32,
    country_id: u32,
}

fn sample(
    source: &Source,
    branch: u64,
    tag: &str,
    frame: &replay_frame::frame::DecodedFrame,
) -> io::Result<Option<Observation>> {
    let mut scan = Scan::default();
    scan.observe_selected(frame, Some(tag), None)?;
    if !scan.active.contains(tag) {
        return Ok(None);
    }
    let mut scanned = scan.country(source.replay_ref(), branch, tag)?;
    scanned.unobserved.clear();
    let ((slot, country_id), _) = scan.states[tag].last.expect("observed idea holder");
    Ok(Some(Observation {
        scanned,
        slot,
        country_id,
    }))
}

/// Sample one block's keyframe for this request.
fn keyframe(
    source: &Source,
    branch: u64,
    tag: &str,
    block: &replay::container::BlockIndex,
    cancelled: &AtomicBool,
) -> io::Result<Option<ScannedIdeas>> {
    crate::metadata::check_cancel(cancelled)?;
    let frame = source.replay_ref().read_keyframe_selected(block, &TABLES)?;
    let mut decoder = FrameDecoder::for_tables(&TABLES);
    Ok(sample(source, branch, tag, decoder.advance(&frame)?)?.map(|sample| sample.scanned))
}

/// The observation owns only one country's keyframe holdings and naming.
fn sample_bytes(sample: &ScannedIdeas) -> usize {
    let naming_bytes = |names: &Vec<NamingCountry>| {
        names.capacity() * std::mem::size_of::<NamingCountry>()
            + names
                .iter()
                .filter_map(|name| name.look.as_ref())
                .map(|look| {
                    look.name_tag.capacity()
                        + look.cosmetic_tag.capacity()
                        + match &look.original_or_own_tag {
                            Ok(tag) | Err(tag) => tag.capacity(),
                        }
                        + look.civil_war.as_ref().err().map_or(0, String::capacity)
                })
                .sum::<usize>()
    };
    std::mem::size_of::<ScannedIdeas>()
        + sample.spans.capacity() * std::mem::size_of::<IdeaSpan>()
        + sample
            .spans
            .iter()
            .map(|span| {
                span.token.capacity()
                    + span.slot.as_ref().map_or(0, String::capacity)
                    + span.character.as_ref().map_or(0, |character| {
                        character.token.capacity() + character.name.capacity()
                    })
            })
            .sum::<usize>()
        + sample.unobserved.capacity() * std::mem::size_of::<HourSpan>()
        + (sample.identity_starts.capacity() + sample.gap_starts.capacity())
            * std::mem::size_of::<i32>()
        + naming_bytes(&sample.identity_naming)
        + naming_bytes(&sample.span_naming)
        + sample
            .templates
            .values()
            .map(|template| {
                2 * std::mem::size_of::<(u32, Result<BuiltTemplate, String>)>()
                    + match template {
                        Ok(template) => template.tag.capacity() + template.file.capacity(),
                        Err(reason) => reason.capacity(),
                    }
            })
            .sum::<usize>()
        + sample.warnings.capacity() * std::mem::size_of::<String>()
        + sample.warnings.iter().map(String::capacity).sum::<usize>()
}

fn append(target: &mut ScannedIdeas, sample: &ScannedIdeas, spans: &[HourSpan]) {
    target.templates.extend(sample.templates.clone());
    for warning in &sample.warnings {
        if !target.warnings.contains(warning) {
            target.warnings.push(warning.clone());
        }
    }
    for span in spans {
        // The sample's naming belongs to the sampled state, never to an
        // invented acquisition at the edge of an approximate block.
        target.identity_starts.push(span.first_hour);
        target
            .identity_naming
            .push(sample.identity_naming.first().cloned().unwrap_or_default());
        target
            .spans
            .extend(sample.spans.iter().map(|held| IdeaSpan {
                first_hour: span.first_hour,
                last_hour: span.last_hour,
                ..held.clone()
            }));
    }
}

/// A prefix reply with optional lane previews and a held-state request.
pub fn query(
    source: &Source,
    content: &PoliticsContent,
    options: &super::super::ContentOptions,
    params: &serde_json::Value,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let params: IdeaHistoryQuery = serde_json::from_value(params.clone())?;
    let country = CountryParams {
        branch_id: params.branch_id,
        tag: params.tag,
    };
    country.validate().map_err(crate::invalid)?;
    let branch = country.branch();
    let hour = params.hour;
    let through = params.through.or(hour);
    let lanes = params.preview.unwrap_or(true);
    let mut prefix = prefix::read_or_empty(
        source,
        Some(options),
        "idea-history",
        branch,
        through,
        cancelled,
    )?;
    let mut scanned = match store.read(key, &format!("idea-history-{branch}"), 0)? {
        Some(file) => Scan::read(io::BufReader::new(file))?.country(
            source.replay_ref(),
            branch,
            &country.tag,
        )?,
        None => ScannedIdeas::default(),
    };
    prefix.clip(&mut scanned.unobserved);
    let identity_starts = scanned.identity_starts.clone();
    let approximate = if lanes && !prefix.coverage.complete {
        keyframes::blocks_after(
            source,
            "idea-history:keyframe",
            branch,
            prefix.coverage.last_hour,
            through,
            &TABLES,
            &country.tag,
            None,
            cancelled,
            &sample_bytes,
            &|block| keyframe(source, branch, &country.tag, block, cancelled),
            |spans, sample| {
                append(&mut scanned, sample, spans);
                Ok(true)
            },
        )?
    } else {
        Vec::new()
    };
    let mut held = None;
    if let Some(hour) =
        hour.filter(|hour| !prefix.coverage.complete && *hour > prefix.coverage.last_hour)
        && let Some(frame) = progressive::frame_at(source, branch, hour, &TABLES, cancelled)?
        && let Some(sample) = sample(source, branch, &country.tag, &frame)?
    {
        append(
            &mut scanned,
            &sample.scanned,
            &[HourSpan {
                first_hour: hour,
                last_hour: hour,
            }],
        );
        let (tokens, _) = content.ideas.tokens();
        let order: BTreeMap<_, _> = tokens
            .enumerate()
            .map(|(index, token)| (token.to_owned(), index))
            .collect();
        let mut spans = sample.scanned.spans;
        spans.sort_by_key(|span| order.get(&span.token).copied().unwrap_or(usize::MAX));
        held = Some(HeldIdeas {
            hour,
            slot: sample.slot,
            country_id: sample.country_id,
            spans,
            preparing: vec![
                "order".into(),
                "acquired".into(),
                "acquisitionContext".into(),
            ],
        });
    }
    for gap in source
        .replay_ref()
        .gaps
        .iter()
        .filter(|gap| gap.branch == branch && through.is_none_or(|hour| gap.first_hour <= hour))
    {
        scanned.unobserved.push(HourSpan {
            first_hour: gap.first_hour,
            last_hour: through.map_or(gap.last_hour, |hour| gap.last_hour.min(hour)),
        });
    }
    let mut gaps = std::mem::take(&mut scanned.unobserved);
    gaps.sort_by_key(|span| span.first_hour);
    for gap in gaps {
        mark(&mut scanned.unobserved, gap.first_hour, gap.last_hour);
    }
    // Exact prefix naming takes precedence at its own hours; sample identities
    // above are chronological, and the at-hour state is the last observation.
    let mut reply = super::reply(content, source.replay_ref(), &country, &scanned, cancelled)?;
    // Sampling dates name the presentation's state, never an identity's start.
    reply.identity_starts = identity_starts;
    reply.approximate = approximate;
    reply.held = held;
    source.append_content_warnings(&mut reply.warnings);
    prefix.sampled(&reply.approximate);
    let mut data = serde_json::to_value(reply)?;
    prefix.mark(&mut data)?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::super::tests::idea_frame;
    use super::*;
    use crate::test_fixtures::{continuation_records, file_source, replay};

    fn held(
        scanned: &ScannedIdeas,
        hour: i32,
    ) -> Vec<(String, Option<String>, Option<IdeaCharacter>)> {
        let mut held: Vec<_> = scanned
            .spans
            .iter()
            .filter(|span| span.first_hour <= hour && hour <= span.last_hour)
            .map(|span| {
                (
                    span.token.clone(),
                    span.slot.clone(),
                    span.character.clone(),
                )
            })
            .collect();
        held.sort_by(|a, b| a.0.cmp(&b.0));
        held
    }

    #[test]
    fn sampled_idea_states_match_hourly_holdings() {
        let mut recording = replay(vec![
            idea_frame(100, &["mod_a"]),
            idea_frame(101, &["mod_a", "mod_b"]),
        ]);
        recording = recording
            .append(&[continuation_records(
                &recording,
                1,
                vec![idea_frame(102, &["mod_b"]), idea_frame(103, &[])],
            )])
            .unwrap();
        let (source, root) = file_source(recording);
        let flag = AtomicBool::new(false);
        let oracle = super::super::scan(source.replay_ref(), 1, "GER", &flag).unwrap();
        for block in &source.replay_ref().index {
            let sampled = keyframe(&source, 1, "GER", block, &flag).unwrap().unwrap();
            assert_eq!(
                held(&sampled, block.first_hour),
                held(&oracle, block.first_hour)
            );
            assert!(
                sampled
                    .spans
                    .iter()
                    .all(|span| span.first_hour == block.first_hour
                        && span.last_hour == block.first_hour)
            );
        }
        drop(source);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sampled_holdings_match_the_scan_with_many_countries() {
        use crate::test_fixtures::{scale_frame, scale_tag};
        use replay_data::{rows::ActiveIdeaRow, rows::TableId, rows::TableRows};
        use replay_frame::frame::TableView;
        let mut frame = scale_frame(1, 100, 24);
        *frame.tables[TableId::ActiveIdeas.index()] = TableView::Complete(TableRows::ActiveIdeas(
            (0..24)
                .map(|index| ActiveIdeaRow {
                    country: 100_000 + index,
                    ordinal: 0,
                    token: format!("mod_idea_{index}").into_bytes(),
                })
                .collect(),
        ));
        *frame.tables[TableId::HiredAdvisors.index()] =
            TableView::Complete(TableRows::HiredAdvisors(Vec::new()));
        let (source, root) = file_source(replay(vec![frame]));
        let flag = AtomicBool::new(false);
        let tag = String::from_utf8(scale_tag(23)).unwrap();
        let oracle = super::super::scan_all(source.replay_ref(), 1, &flag).unwrap();
        let sampled = keyframe(&source, 1, &tag, &source.replay_ref().index[0], &flag)
            .unwrap()
            .unwrap();
        assert_eq!(held(&sampled, 100), held(oracle.country(&tag), 100));
        drop(source);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn idea_first_views_are_non_null_marked_and_keep_the_exact_prefix_and_held_set() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (_catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/defines/work.lua",
                "NDefines={NFocus={FOCUS_POINT_DAYS=7}, NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
            ),
            (
                "common/ideas/work.txt",
                "ideas={country={mod_b={} mod_a={}}}",
            ),
        ]);
        let input = vec![
            idea_frame(100, &["mod_a"]),
            idea_frame(101, &["mod_a", "mod_b"]),
            idea_frame(102, &["mod_b"]),
        ];
        let (memory, context) = crate::test_fixtures::source_with_content(input.clone(), &folder.0);
        let (source, root) = file_source(
            replay::container::Replay::open(memory.replay_ref().bytes().to_vec()).unwrap(),
        );
        let options = context.content.as_ref().unwrap();
        let content = politics_content(&source, options).unwrap();
        let (store, key) = crate::prepared::files(&source, Some(options)).unwrap();
        let flag = AtomicBool::new(false);
        let params =
            serde_json::json!({ "branchId": "1", "tag": "GER", "hour": 102, "preview": false });
        let cold = query(&source, &content, options, &params, &store, &key, &flag).unwrap();
        assert!(!cold.is_null());
        assert_eq!(
            cold["identityStarts"],
            serde_json::json!([]),
            "a sampled hour is not an invented identity start"
        );
        assert!(!cold["pending"].as_array().unwrap().is_empty());
        assert_eq!(
            cold["held"]["preparing"],
            serde_json::json!(["order", "acquired", "acquisitionContext"])
        );
        let preview_params = serde_json::json!({ "branchId": "1", "tag": "GER", "through": 102 });
        let preview = query(
            &source,
            &content,
            options,
            &preview_params,
            &store,
            &key,
            &flag,
        )
        .unwrap();
        assert_eq!(
            preview["approximate"],
            serde_json::json!([{"firstHour": 100, "lastHour": 102}])
        );
        assert_eq!(
            query(
                &source,
                &content,
                options,
                &preview_params,
                &store,
                &key,
                &flag
            )
            .unwrap(),
            preview
        );
        assert_eq!(
            crate::prepared::query::observations::usage(&source, "idea-history:keyframe").2,
            1
        );
        let mut scan = Scan::default();
        scan.observe(&input[0]).unwrap();
        let mut writer = store.replace(key.clone()).unwrap();
        writer
            .write_at("idea-history-1", 0, scan.saved_bytes().unwrap().as_slice())
            .unwrap();
        crate::prepared::coverage::publish_coverage(&mut writer, 1, 100, 100, false).unwrap();
        writer.commit().unwrap();
        let prefix = query(&source, &content, options, &params, &store, &key, &flag).unwrap();
        assert_eq!(prefix["held"], cold["held"]);
        let clipped_preview = query(
            &source,
            &content,
            options,
            &preview_params,
            &store,
            &key,
            &flag,
        )
        .unwrap();
        assert_eq!(
            clipped_preview["approximate"],
            serde_json::json!([{"firstHour": 101, "lastHour": 102}])
        );
        assert_eq!(
            crate::prepared::query::observations::usage(&source, "idea-history:keyframe").2,
            1,
            "a moving prefix clips the country's cached keyframe"
        );
        let known = scan.country(source.replay_ref(), 1, "GER").unwrap();
        assert_eq!(
            prefix["spans"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|span| span["firstHour"] == 100)
                .cloned()
                .collect::<Vec<_>>(),
            serde_json::to_value(&known.spans)
                .unwrap()
                .as_array()
                .unwrap()
                .clone()
        );
        for frame in &input[1..] {
            scan.observe(frame).unwrap();
        }
        let mut writer = store.extend(&key).unwrap();
        writer
            .write_at("idea-history-1", 0, scan.saved_bytes().unwrap().as_slice())
            .unwrap();
        crate::prepared::coverage::publish_coverage(&mut writer, 1, 100, 102, true).unwrap();
        writer.commit().unwrap();
        let complete = query(&source, &content, options, &params, &store, &key, &flag).unwrap();
        assert!(complete.get("held").is_none());
        assert_eq!(complete["pending"], serde_json::json!([]));
        let oracle = scan.country(source.replay_ref(), 1, "GER").unwrap();
        assert_eq!(
            complete["spans"],
            serde_json::to_value(&oracle.spans).unwrap()
        );
        let mut actual: Vec<_> = cold["held"]["spans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|span| span["token"].clone())
            .collect();
        let mut expected: Vec<_> = oracle
            .spans
            .iter()
            .filter(|span| span.first_hour <= 102 && span.last_hour >= 102)
            .map(|span| serde_json::json!(span.token))
            .collect();
        actual.sort_by_key(ToString::to_string);
        expected.sort_by_key(ToString::to_string);
        assert_eq!(actual, expected);
        store.clear().unwrap();
        drop(source);
        let _ = std::fs::remove_dir_all(root);
    }
}
