//! Held member balances, gains and selections in the existing faction view.
use super::*;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionAccountDetail {
    pub ordinal: u32,
    pub country: u32,
    pub tag: Option<String>,
    pub current_member: bool,
    pub influence: Vec<String>,
    pub projection: Vec<String>,
    pub influence_share_raw: String,
    pub projection_share_raw: String,
    pub projection_rank: i32,
    pub initiative_raw: String,
    pub contribution_raw: String,
    pub contribution_gain: Vec<String>,
    pub previous_war_score_raw: String,
    pub upgrades: Vec<FactionUpgradeDetail>,
    pub completed: Vec<FactionGoalDefinition>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionUpgradeDetail {
    pub group: u32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub reason: Option<String>,
}
pub(super) fn attach(
    frame: &DecodedFrame,
    faction: u64,
    cat: Option<&catalogue::Catalogue>,
    definitions: Option<&installed_content_core::faction_goals::Catalog>,
    lexical: &faction_names::LexicalNames,
    reply: &mut FactionDetailReply,
) {
    let result = (|| -> Result<Vec<FactionAccountDetail>, String> {
        let rows = frame
            .complete::<tables::FactionAccounts>()
            .map_err(|why| why.to_string())?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|why| why.to_string())?;
        let countries = frame
            .complete::<tables::Countries>()
            .map_err(|why| why.to_string())?;
        let members = frame
            .complete::<tables::FactionMembers>()
            .map_err(|why| why.to_string())?;
        let mut result = Vec::new();
        for row in rows.iter().filter(|row| row.kind == 0) {
            let influence = replay_data::rows::FactionAccountRow::statistics(rows, row.account, 1)
                .collect::<Vec<_>>();
            let projection = row.projection(rows).map_err(|why| why.to_string())?;
            let slot = crate::metadata::link_slot(links, row.country);
            let owner = slot
                .and_then(|slot| {
                    members.iter().find(|member| {
                        crate::metadata::link_slot(links, member.country) == Some(slot)
                    })
                })
                .map(|member| member.faction);
            if owner.is_some_and(|owner| owner != faction) {
                continue;
            }
            // Retained accounts without current membership stay visible. Empty
            // unused slots carry no faction-screen fact beyond their index.
            if owner.is_none()
                && influence.iter().all(|value| *value == 0)
                && projection.iter().all(|value| *value == 0)
                && row.influence_share() == 0
                && row.projection_share() == 0
                && row.rank(rows).map_err(|why| why.to_string())? == 0
                && row.initiative() == 0
                && row.contribution_raw == 0
                && row.contribution_gain_a_raw == 0
                && row.contribution_gain_b_raw == 0
                && row.previous_war_score_raw == 0
                && row.upgrade_tokens.is_empty()
                && row.completed_goals.is_empty()
            {
                continue;
            }
            let tag = slot
                .and_then(|slot| {
                    countries
                        .iter()
                        .find(|country| country.canonical_slot == slot)
                })
                .map(|row| String::from_utf8_lossy(&row.tag).into_owned());
            let mut upgrades = Vec::new();
            for (&group, &token) in row.upgrade_groups.iter().zip(&row.upgrade_tokens) {
                let mut value = FactionUpgradeDetail {
                    group,
                    token: None,
                    name: None,
                    reason: None,
                };
                let rendered = (|| -> Result<(), String> {
                    let spelling = faction_names::template_spelling(frame, Some(token))?
                        .ok_or("Faction member upgrade token is absent")?;
                    value.token = Some(spelling.clone());
                    let cat = cat.ok_or("Faction member upgrade needs recorded localisation")?;
                    let text = cat
                        .localisation
                        .localise(spelling.as_bytes())
                        .map_err(|why| why.to_string())?;
                    // Dynamic upgrade names require their member's environment.
                    if text.contains('[') {
                        return Err(format!(
                            "Faction upgrade {spelling}: dynamic member name environment is unavailable"
                        ));
                    }
                    value.name = Some(catalogue::plain_text(&text)?);
                    Ok(())
                })();
                if let Err(why) = rendered {
                    reply.warnings.push(why.clone());
                    value.reason = Some(why);
                }
                upgrades.push(value);
            }
            upgrades.sort_unstable_by_key(|value| value.group);
            result.push(FactionAccountDetail {
                ordinal: row.account,
                country: row.country,
                tag,
                current_member: owner == Some(faction),
                influence: influence.iter().map(ToString::to_string).collect(),
                projection: projection.iter().map(ToString::to_string).collect(),
                influence_share_raw: row.influence_share().to_string(),
                projection_share_raw: row.projection_share().to_string(),
                projection_rank: row.rank(rows).map_err(|why| why.to_string())?,
                initiative_raw: row.initiative().to_string(),
                contribution_raw: row.contribution_raw.to_string(),
                contribution_gain: vec![
                    row.contribution_gain_a_raw.to_string(),
                    row.contribution_gain_b_raw.to_string(),
                ],
                previous_war_score_raw: row.previous_war_score_raw.to_string(),
                upgrades,
                completed: row
                    .completed_goals
                    .iter()
                    .map(|token| {
                        super::definition(
                            frame,
                            faction,
                            (*token != u64::MAX).then_some(*token as u32),
                            cat,
                            definitions,
                            lexical,
                        )
                    })
                    .collect(),
            });
        }
        Ok(result)
    })();
    match result {
        Ok(values) => reply.accounts = Some(values),
        Err(why) => reply
            .warnings
            .push(format!("Faction member accounting: {why}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{FactionAccountRow, TableRows};
    use replay_frame::frame::TableView;
    #[test]
    fn faction_account_view_keeps_retained_projection_only_facts_with_exact_integers() {
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![]));
        *frame.tables[TableId::FactionAccounts.index()] =
            TableView::Complete(TableRows::FactionAccounts(vec![
                FactionAccountRow {
                    account: 4,
                    country: u32::MAX,

                    ..Default::default()
                },
                FactionAccountRow {
                    account: 4,
                    kind: 2,
                    ordinal: 0,
                    statistic_raw: i64::MIN,
                    ..Default::default()
                },
            ]));
        let lexical =
            faction_names::LexicalNames::new(frame.tables[TableId::LexicalTokens.index()].clone());
        let mut reply = FactionDetailReply::default();
        attach(&frame, 7, None, None, &lexical, &mut reply);
        let rows = reply.accounts.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].projection, [i64::MIN.to_string()]);
        assert!(!rows[0].current_member);
    }
}
