//! Artwork only for mounted leaders. History scans never decode portraits.
use super::*;

pub struct UnitLeaderPortraits;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnitLeaderPortraitsParams {
    pub branch_id: String,
    pub hour: i32,
    pub references: Vec<String>,
    pub edge: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnitLeaderPortrait {
    pub reference: String,
    pub image: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnitLeaderPortraitsReply {
    pub portraits: Vec<UnitLeaderPortrait>,
    pub warnings: Vec<String>,
}

fn reply_bytes(reply: &UnitLeaderPortraitsReply) -> io::Result<usize> {
    dashboard_core::transport::json_len(reply, dashboard_base::products::MAX_PRODUCT_REPLY_BYTES).ok_or_else(|| {
        io::Error::other(
            "Leader portrait reply exceeds its transport budget; request fewer portraits or smaller thumbnails",
        )
    })
}

pub(super) fn artwork(
    frame: &DecodedFrame,
    reference: u64,
    politics: &politics_content::PoliticsContent,
    edge: u32,
) -> Result<String, String> {
    let rows = frame
        .complete::<tables::UnitLeaders>()
        .map_err(|e| e.to_string())?;
    let row = rows
        .binary_search_by_key(&reference, handle)
        .ok()
        .map(|i| &rows[i])
        .ok_or("Leader not recorded at this hour")?;
    let holders = frame
        .complete::<tables::CharacterPortraits>()
        .map_err(|e| e.to_string())?;
    let holder = holders
        .binary_search_by_key(&row.character_id, |p| p.character)
        .ok()
        .map(|i| &holders[i])
        .ok_or("Character portrait holder not recorded")?;
    let category = if row.kind == engine::cunit_leader::KIND_NAVY {
        PORTRAIT_NAVY
    } else {
        PORTRAIT_ARMY
    };
    let entry =
        replay::rows::portrait_entry(&holder.portraits, category, replay::rows::PORTRAIT_LARGE)
            .filter(|e| e.is_set != 0)
            .ok_or("The game generates this portrait from unrecorded inputs")?;
    politics.recorded_portrait_sized(entry.kind, &entry.identity, edge)
}

pub fn check_content(
    politics: &politics_content::PoliticsContent,
) -> Vec<dashboard_core::findings::Finding> {
    use dashboard_core::findings::{Finding, Kind};
    use installed_content::characters::{Authored, PORTRAIT_SIZES};
    let mut findings = Vec::new();
    for token in politics.characters.declared_tokens() {
        let portraits = match politics.characters.portraits(token) {
            Ok(Some(portraits)) => portraits,
            Ok(None) => continue,
            Err(reason) => {
                findings.push(Finding::new(Kind::Refused, token, reason.to_string()));
                continue;
            }
        };
        for ((role, size), portrait) in portraits
            .iter()
            .filter(|((_, size), _)| size.as_str() == PORTRAIT_SIZES[1])
        {
            let subject = format!("leader portrait {token}/{role}/{size}");
            match portrait {
                Authored::Ambiguous(_) => findings.push(Finding::new(
                    Kind::Offline,
                    &subject,
                    "Portrait alternatives need the recorded portrait holder",
                )),
                Authored::Unknown(reason) => {
                    findings.push(Finding::new(Kind::Refused, subject, reason));
                    continue;
                }
                Authored::Missing => findings.push(Finding::new(
                    Kind::Offline,
                    &subject,
                    "Missing authored portrait needs recorded fallback or native generation inputs",
                )),
                Authored::Asset(_) => {}
            }
            for asset in portrait.names() {
                match politics.sprite_png_sized(&asset, 86) {
                    Ok(Some(_)) => {}
                    Ok(None) => findings.push(Finding::new(
                        Kind::Offline,
                        &subject,
                        format!("{asset}: native portrait generation needs recorded inputs"),
                    )),
                    Err(reason) => {
                        findings.push(Finding::new(Kind::Refused, &subject, reason.to_string()))
                    }
                }
            }
        }
    }
    findings
}

fn missing_hour(params: &UnitLeaderPortraitsParams) -> Vec<UnitLeaderPortrait> {
    params
        .references
        .iter()
        .map(|reference| UnitLeaderPortrait {
            reference: reference.clone(),
            image: None,
            missing: Some("Portrait hour not recorded".into()),
        })
        .collect()
}

impl Product for UnitLeaderPortraits {
    const KEY: &'static str = "unit-leader-portraits";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = UnitLeaderPortraitsParams;
    type Reply = UnitLeaderPortraitsReply;

    fn types() -> Vec<String> {
        vec![
            UnitLeaderPortraitsParams::decl(),
            UnitLeaderPortrait::decl(),
            UnitLeaderPortraitsReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err()
            || params.references.is_empty()
            || params.edge == 0
            || params.references.iter().any(|r| r.parse::<u64>().is_err())
        {
            return Err("Invalid leader portrait request");
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let replay = source.replay();
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let selected = [
            TableId::Campaign,
            TableId::UnitLeaders,
            TableId::CharacterPortraits,
        ];
        let mut portraits = missing_hour(params);
        let block = replay.index.iter().find(|b| {
            b.branch == branch && b.first_hour <= params.hour && params.hour <= b.last_hour
        });
        let Some(block) = block else {
            let reply = UnitLeaderPortraitsReply {
                portraits,
                warnings: Vec::new(),
            };
            reply_bytes(&reply)?;
            return Ok(reply);
        };
        let frames = replay.read_block_selected(block, &selected)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Portrait request cancelled",
                ));
            }
            if frame.hour > params.hour {
                break;
            }
            let decoded = decoder.advance(&frame)?;
            validate_campaign(decoded, &replay)?;
            if frame.hour != params.hour {
                continue;
            }
            let politics = context
                .content()
                .and_then(|content| politics_content::politics_content(source, content));
            portraits.clear();
            for reference in &params.references {
                let result = match &politics {
                    Ok(politics) => artwork(
                        decoded,
                        reference.parse().expect("validated reference"),
                        politics,
                        params.edge,
                    ),
                    Err(e) => Err(e.to_string()),
                };
                let (image, missing) = match result {
                    Ok(image) => (Some(image), None),
                    Err(e) => (None, Some(e)),
                };
                portraits.push(UnitLeaderPortrait {
                    reference: reference.clone(),
                    image,
                    missing,
                });
            }
            break;
        }
        let reply = UnitLeaderPortraitsReply {
            portraits,
            warnings: Vec::new(),
        };
        reply_bytes(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_frame_returns_each_requested_portrait_as_unknown() {
        let replay = Arc::new(
            Replay::from_records(
                b"analytics-test".to_vec(),
                &[dashboard_core::test_fixtures::records(
                    1,
                    vec![
                        dashboard_core::test_fixtures::source(1, 100, None),
                        dashboard_core::test_fixtures::source(1, 102, None),
                    ],
                )],
            )
            .unwrap(),
        );
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "portraits".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay,
            &AtomicBool::new(false),
        )
        .unwrap();
        for hour in [99, 101] {
            let params = UnitLeaderPortraitsParams {
                branch_id: "1".into(),
                hour,
                references: vec!["1".into(), "2".into()],
                edge: 86,
            };
            let reply = UnitLeaderPortraits::load(
                &source,
                &params,
                &LoadContext { content: None },
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(reply.portraits.len(), 2);
            assert_eq!(reply.portraits[1].reference, "2");
            assert!(
                reply
                    .portraits
                    .iter()
                    .all(|portrait| portrait.image.is_none()
                        && portrait.missing.as_deref() == Some("Portrait hour not recorded"))
            );
        }
    }

    #[test]
    fn requests_are_not_limited_by_reference_count_or_thumbnail_edge() {
        assert!(
            UnitLeaderPortraits::validate(&UnitLeaderPortraitsParams {
                branch_id: "1".into(),
                hour: 100,
                references: (1..=5000).map(|n| n.to_string()).collect(),
                edge: u32::MAX,
            })
            .is_ok()
        );
    }

    #[test]
    fn replies_refuse_the_transport_budget_without_truncating() {
        let reply = UnitLeaderPortraitsReply {
            portraits: vec![UnitLeaderPortrait {
                reference: "1".into(),
                image: Some("x".repeat(dashboard_base::products::MAX_PRODUCT_REPLY_BYTES)),
                missing: None,
            }],
            warnings: Vec::new(),
        };
        assert!(
            reply_bytes(&reply)
                .unwrap_err()
                .to_string()
                .contains("transport budget")
        );
        assert_eq!(reply.portraits.len(), 1);
    }
}
