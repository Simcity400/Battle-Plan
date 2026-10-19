use super::*;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionResourceDetail {
    pub ordinal: u32,
    pub token: Option<String>,
    pub name: Option<String>,
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
                    .and_then(|accounts| {
                        row.influence_total(accounts).map_err(|why| why.to_string())
                    }) {
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
            let mut detail = FactionResourceDetail {
                ordinal: row.ordinal,
                token: None,
                name: None,
                raw: row.raw.to_string(),
                reason: None,
            };
            let result = (|| -> Result<(), String> {
                if row.ordinal == 0 && row.raw == 0 {
                    detail.name = Some("Null resource".into());
                    return Ok(());
                }
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
                detail.token = Some(info.token.clone());
                detail.name = info.name.clone();
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
    Ok(())
}
