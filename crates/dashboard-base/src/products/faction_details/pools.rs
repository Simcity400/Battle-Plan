use super::*;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionResourceDetail {
    pub ordinal: u32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub raw: String,
    pub reason: Option<String>,
}
pub(super) fn attach(
    source: &Source,
    context: &LoadContext,
    cancelled: &AtomicBool,
    frame: &DecodedFrame,
    faction: u64,
    reply: &mut FactionDetailReply,
) -> io::Result<()> {
    match frame.complete::<tables::FactionProjection>() {
        Ok(rows) => {
            if let [row] = companions::selected(rows, faction, |row| row.faction) {
                reply.projection_raw = Some(row.projection_raw.to_string());
                match frame
                    .complete::<tables::FactionAccounts>()
                    .map_err(|why| why.to_string())
                    .and_then(|accounts| selectors::influence_total(row, accounts))
                {
                    Ok(value) => reply.influence_total_raw = Some(value.to_string()),
                    Err(why) => reply
                        .warnings
                        .push(format!("Faction influence total: {why}")),
                }
                reply.projection_effects_raw = Some(row.effects_raw.to_string());
            } else {
                reply
                    .warnings
                    .push("Faction projection owner absent or repeated".into());
            }
        }
        Err(why) => reply.warnings.push(format!("Faction projection: {why}")),
    }
    let rows = match frame.complete::<tables::FactionResources>() {
        Ok(rows) => rows,
        Err(why) => {
            reply
                .warnings
                .push(format!("Faction extracted resources: {why}"));
            return Ok(());
        }
    };
    let definitions = frame.complete::<tables::ResourceDefinitions>();
    let lexical = frame.complete::<tables::LexicalTokens>();
    let mut names = super::super::resources::Learned::default();
    if let (Ok(definitions), Ok(lexical)) = (&definitions, &lexical) {
        names.learn(definitions, lexical);
    }
    let resources = names.list();
    let (infos, warnings) =
        super::super::resources::resource_infos(source, context, &resources, cancelled)?;
    reply.warnings.extend(warnings);
    let read = |owner| {
        let rows = companions::selected(rows, owner, |row| row.faction);
        let mut values = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            if row.ordinal as usize != index {
                return Err("Faction resource slots are not contiguous".to_owned());
            }
            // The engine's empty resource slot is no resource; the game lists none for it.
            if row.ordinal == 0 && row.raw == 0 {
                continue;
            }
            let mut detail = FactionResourceDetail {
                ordinal: row.ordinal,
                token: None,
                name: None,
                icon: None,
                raw: row.raw.to_string(),
                reason: None,
            };
            let result = (|| -> Result<(), String> {
                let definition = definitions
                    .as_ref()
                    .map_err(ToString::to_string)?
                    .iter()
                    .find(|definition| definition.id == row.ordinal)
                    .ok_or("Faction resource slot has no recorded definition")?;
                if definition.token != row.token {
                    return Err(
                        "Faction resource slot token disagrees with recorded definition".into(),
                    );
                }
                let info = infos
                    .iter()
                    .find(|info| info.id == row.ordinal)
                    .ok_or("Faction resource token spelling unavailable")?;
                resource_presentation(&mut detail, info);
                if detail.name.is_none() {
                    return Err("Faction resource name unavailable in recorded content".into());
                }
                Ok(())
            })();
            if let Err(why) = result {
                detail.reason = Some(why);
            }
            values.push(detail);
        }
        Ok(values)
    };
    let known_owner = frame
        .complete::<tables::FactionProjection>()
        .is_ok_and(|rows| rows.iter().any(|row| row.faction == faction))
        || frame
            .complete::<tables::Factions>()
            .is_ok_and(|rows| rows.iter().any(|row| row.id == faction))
        || frame
            .complete::<tables::FactionGoalStatus>()
            .is_ok_and(|rows| rows.iter().any(|row| row.faction == faction))
        || frame
            .complete::<tables::FactionUnlocks>()
            .is_ok_and(|rows| rows.iter().any(|row| row.faction == faction))
        || rows.iter().any(|row| row.faction == faction);
    if known_owner {
        match read(faction) {
            Ok(values) => reply.resources = Some(values),
            Err(why) => reply.warnings.push(why),
        }
    } else {
        reply
            .warnings
            .push("Faction resource owner is unavailable; empty pool is not certified".into());
    }
    match read(0) {
        Ok(values) => reply.system_resources = Some(values),
        Err(why) => reply.warnings.push(why),
    }
    let mut refused = std::collections::BTreeMap::<String, u32>::new();
    for value in reply
        .resources
        .iter()
        .chain(&reply.system_resources)
        .flatten()
    {
        if value.icon.is_none() && !(value.ordinal == 0 && value.raw == "0") {
            *refused
                .entry(format!(
                    "Faction resource icon slot {} ({:?}) is unavailable",
                    value.ordinal, value.token
                ))
                .or_default() += 1;
        }
    }
    reply.refusals.extend(
        refused
            .into_iter()
            .map(|(reason, count)| FactionRefusalDetail {
                fact: "resource artwork".into(),
                count,
                reason,
            }),
    );
    Ok(())
}

fn resource_presentation(
    detail: &mut FactionResourceDetail,
    info: &super::super::resources::ResourceInfo,
) {
    detail.token = Some(info.token.clone());
    detail.name = info.name.clone();
    detail.icon = info.icon.clone();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn faction_resource_icons_use_the_shared_resource_result_without_hiding_amounts() {
        for icon in [Some("data:image/png;base64,c3ludGhldGlj".to_owned()), None] {
            let info = super::super::super::resources::ResourceInfo {
                id: 1,
                token: "mod_resource".into(),
                ordinal: 1,
                name: Some("Mod resource".into()),
                icon: icon.clone(),
            };
            let mut detail = FactionResourceDetail {
                ordinal: 1,
                token: None,
                name: None,
                icon: None,
                raw: "-9007199254740993".into(),
                reason: None,
            };
            resource_presentation(&mut detail, &info);
            assert_eq!(detail.icon, icon);
            assert_eq!(detail.raw, "-9007199254740993");
            assert_eq!(detail.name, info.name);
        }
    }
    #[test]
    fn faction_resource_pool_icons_resolve_mod_content_through_the_resource_art_route() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::catalogue::texture_tests::{png_bytes, texture_catalogue};
        use replay_data::rows::{
            FactionResourceRow, LexicalTokenRow, ResourceDefinitionRow, TableRows,
        };
        use replay_frame::frame::TableView;
        use std::sync::Arc;
        let (cat, guards) = texture_catalogue(
            &[
                (
                    "interface/mod.gui",
                    "guiTypes={containerWindowType={name=resources_info_entry buttonType={name=resources_icon spriteType=GFX_mod_sheet}}}",
                ),
                (
                    "interface/art.gfx",
                    "spriteTypes={spriteType={name=GFX_mod_sheet textureFile=gfx/mod.png}}",
                ),
                (
                    "common/resources/mod.txt",
                    "resources={mod_ore={icon_frame=1} mod_without_art={}}",
                ),
            ],
            &[(
                "gfx/mod.png",
                png_bytes(&installed_content_image::Rgba8Image {
                    width: 1,
                    height: 1,
                    pixels: vec![12, 34, 56, 255],
                }),
            )],
        );
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::ResourceDefinitions.index()] =
            TableView::Complete(TableRows::ResourceDefinitions(
                (1..=2)
                    .map(|id| ResourceDefinitionRow {
                        id,
                        token: id,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(
                ["mod_ore", "mod_without_art"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, token)| LexicalTokenRow {
                        token: index as u32 + 1,
                        literal: token.as_bytes().to_vec(),
                    })
                    .collect(),
            ));
        *frame.tables[TableId::FactionResources.index()] =
            TableView::Complete(TableRows::FactionResources(
                [0, 9]
                    .into_iter()
                    .flat_map(|faction| {
                        (0..=2).map(move |ordinal| FactionResourceRow {
                            faction,
                            ordinal,
                            token: ordinal,
                            raw: if ordinal == 0 { 0 } else { -9007199254740993 },
                        })
                    })
                    .collect(),
            ));
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: "resource-artwork".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(crate::test_fixtures::replay(vec![frame.clone()])),
            &AtomicBool::new(false),
        )
        .unwrap();
        let options = super::super::super::ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir: guards[1].0.clone(),
                user_dir: guards[1].0.join("user"),
                cache_dir: guards[1].0.join("cache"),
            },
            allow_unverified: true,
            presentation: Default::default(),
        };
        catalogue::set_test_catalogue(&source, &options, Arc::new(cat));
        let mut reply = FactionDetailReply::default();
        attach(
            &source,
            &LoadContext {
                content: Some(options),
            },
            &AtomicBool::new(false),
            &frame,
            9,
            &mut reply,
        )
        .unwrap();
        for pool in [&reply.resources, &reply.system_resources] {
            let pool = pool.as_ref().unwrap();
            assert!(
                pool[1]
                    .icon
                    .as_ref()
                    .unwrap()
                    .starts_with("data:image/png;base64,")
            );
            assert!(pool[2].icon.is_none());
            assert_eq!(pool[1].raw, "-9007199254740993");
            assert_eq!(pool[2].token.as_deref(), Some("mod_without_art"));
        }
        assert!(
            reply
                .warnings
                .iter()
                .any(|why| why.contains("mod_without_art"))
        );
        assert_eq!(reply.refusals.len(), 1);
        assert_eq!(reply.refusals[0].count, 2);
        assert!(reply.refusals[0].reason.contains("mod_without_art"));
    }
}
