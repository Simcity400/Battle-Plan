use replay_data::{rows::VariantDefinitionRow, rows::WingEquipmentRow, rows::tables};
use replay_frame::frame::DecodedSource;

pub fn primary(rows: &[WingEquipmentRow], wing: u64) -> Option<u64> {
    let mut best: Option<&WingEquipmentRow> = None;
    for row in rows.iter().filter(|row| row.wing == wing) {
        if best.is_none_or(|old| {
            row.amount_raw > old.amount_raw
                || (row.amount_raw == old.amount_raw && row.ordinal < old.ordinal)
        }) {
            best = Some(row);
        }
    }
    best.map(|row| row.variant)
}

pub fn wing_primary(frame: &dyn DecodedSource, wing: u64) -> Result<Option<u64>, String> {
    Ok(primary(
        frame
            .complete::<tables::WingEquipment>()
            .map_err(|error| error.to_string())?,
        wing,
    ))
}

pub fn resolve_lazy(
    variant: &VariantDefinitionRow,
    dlc_mask: u32,
    designer: &dyn Fn() -> Result<Option<String>, String>,
    graphical: &dyn Fn() -> Result<Option<String>, String>,
    prefixed: &dyn Fn() -> Result<Option<String>, String>,
    declared: &dyn Fn(&str) -> bool,
) -> Result<Option<String>, String> {
    if dlc_mask & 0x100 != 0
        && let Some(name) = designer()?
        && declared(&name)
    {
        return Ok(Some(name));
    }
    let saved = std::str::from_utf8(&variant.explicit_entity)
        .map_err(|_| format!("equipment variant {} saved model is not text", variant.id))?;
    if !saved.is_empty() && declared(saved) {
        return Ok(Some(saved.to_owned()));
    }
    if dlc_mask & 0x100 != 0
        && let Some(name) = graphical()?
    {
        return Ok(declared(&name).then_some(name));
    }
    prefixed()
}

#[cfg(any(test, feature = "test-support"))]
pub fn candidates(
    cosmetic: &str,
    tag: &str,
    equipment: &str,
    archetype: &str,
    sprite: &str,
) -> Vec<String> {
    let mut names = Vec::new();
    for prefix in [(!cosmetic.is_empty()).then_some(cosmetic), Some(tag)]
        .into_iter()
        .flatten()
        .map(Some)
        .chain([None])
    {
        for key in [equipment, archetype, sprite] {
            let name = match prefix {
                Some(prefix) => format!("{prefix}_{key}_entity"),
                None => format!("{key}_entity"),
            };
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

pub fn prefixed(
    cosmetic: &dyn Fn() -> Result<String, String>,
    tag: &dyn Fn() -> Result<String, String>,
    key: &dyn Fn(usize) -> Result<String, String>,
    declared: &dyn Fn(&str) -> bool,
) -> Result<Option<String>, String> {
    for prefix in 0..3 {
        let prefix = match prefix {
            0 => {
                let value = cosmetic()?;
                if value.is_empty() {
                    continue;
                }
                Some(value)
            }
            1 => Some(tag()?),
            _ => None,
        };
        for index in 0..3 {
            let key = key(index)?;
            let name = match &prefix {
                Some(prefix) => format!("{prefix}_{key}_entity"),
                None => format!("{key}_entity"),
            };
            if declared(&name) {
                return Ok(Some(name));
            }
        }
    }
    Ok(None)
}

#[cfg(any(test, feature = "test-support"))]
fn equipment_names(
    content: &content_reader_key_names::equipment::Database,
    name: &str,
) -> Result<(String, String), String> {
    let kind = content
        .named(name)
        .ok_or_else(|| format!("equipment {name} is not in the content"))?;
    let text = |bytes: &[u8], what: &str| {
        content_reader_parser::parser::text(bytes)
            .map(str::to_owned)
            .map_err(|refused| format!("equipment {name} {what}: {refused}"))
    };
    let archetype = match kind.archetype {
        Some(archetype) => text(&content.equipment[archetype].name, "archetype name")?,
        None => String::new(),
    };
    Ok((archetype, text(&kind.sprite, "sprite")?))
}

#[cfg(any(test, feature = "test-support"))]
pub fn designer_entity(
    variant: &replay_data::rows::VariantDefinitionRow,
    equipment: &str,
    content: &content_reader_key_names::equipment::Database,
    tag: &str,
    dlc_mask: u32,
    variant_model: &dyn Fn() -> Result<Option<String>, String>,
    declared: &dyn Fn(&str) -> bool,
) -> Result<Option<String>, String> {
    resolve_lazy(
        variant,
        dlc_mask,
        variant_model,
        &|| Ok(None),
        &|| {
            let (archetype, sprite) = equipment_names(content, equipment)?;
            Ok(candidates("", tag, equipment, &archetype, &sprite)
                .into_iter()
                .find(|name| declared(name)))
        },
        declared,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn select_fixture(
        variant: &VariantDefinitionRow,
        mask: u32,
        designer: &dyn Fn() -> Result<Option<String>, String>,
        graphical: &dyn Fn() -> Result<Option<String>, String>,
        declared: &dyn Fn(&str) -> bool,
    ) -> Result<Option<String>, String> {
        resolve_lazy(
            variant,
            mask,
            designer,
            graphical,
            &|| {
                prefixed(
                    &|| Ok("COS".into()),
                    &|| Ok("TAG".into()),
                    &|index| Ok(["frame", "family", "sprite"][index].into()),
                    declared,
                )
            },
            declared,
        )
    }
    #[test]
    fn primary_uses_native_ordinal_instead_of_variant_key_order() {
        let rows = [
            (9, 2, 400),
            (7, 0, 400),
            (3, 1, 200),
            (1, 3, 0),
            (2, 4, -100),
        ]
        .map(|(variant, ordinal, amount_raw)| WingEquipmentRow {
            wing: 5,
            variant,
            ordinal,
            amount_raw,
        });
        assert_eq!(primary(&rows, 5), Some(7));
        assert_eq!(primary(&rows, 4), None);
        assert_eq!(primary(&rows[3..], 5), Some(1));
    }
    #[test]
    fn primary_selects_the_greatest_signed_balance_in_a_nonempty_bank() {
        let rows =
            [(9, 0, 0), (7, 1, -100_000)].map(|(variant, ordinal, amount_raw)| WingEquipmentRow {
                wing: 5,
                variant,
                ordinal,
                amount_raw,
            });
        assert_eq!(primary(&rows, 5), Some(9));
        let negative = [(9, 2, i64::MIN), (7, 0, -100_000), (3, 1, -200_000)].map(
            |(variant, ordinal, amount_raw)| WingEquipmentRow {
                wing: 5,
                variant,
                ordinal,
                amount_raw,
            },
        );
        assert_eq!(primary(&negative, 5), Some(7));
        assert_eq!(primary(&negative[..1], 5), Some(9));
        assert_eq!(primary(&[], 5), None);
    }
    #[test]
    fn primary_nonpositive_ties_keep_the_first_native_ordinal() {
        for amount_raw in [0, -100_000, i64::MIN] {
            let rows = [(3, 2), (7, 0), (9, 1)].map(|(variant, ordinal)| WingEquipmentRow {
                wing: 5,
                variant,
                ordinal,
                amount_raw,
            });
            assert_eq!(primary(&rows, 5), Some(7));
        }
    }
    #[test]
    fn recorded_primary_distinguishes_a_nonpositive_bank_from_empty_or_unavailable() {
        use viewer_foundation::test_support::{test_frame, test_put as put};
        use replay_data::{rows::TableId, rows::TableRows};
        use replay_frame::frame::TableView;
        let mut frame = test_frame(42);
        put(
            &mut frame,
            TableRows::WingEquipment(vec![
                WingEquipmentRow {
                    wing: 5,
                    variant: 7,
                    ordinal: 1,
                    amount_raw: -100_000,
                },
                WingEquipmentRow {
                    wing: 5,
                    variant: 9,
                    ordinal: 0,
                    amount_raw: 0,
                },
            ]),
        );
        assert_eq!(wing_primary(&frame, 5), Ok(Some(9)));
        assert_eq!(wing_primary(&frame, 6), Ok(None));
        *frame.tables[TableId::WingEquipment.index()] = TableView::Unavailable {
            reason: "bank acquisition failed".into(),
        };
        assert!(
            wing_primary(&frame, 5)
                .unwrap_err()
                .contains("bank acquisition failed")
        );
    }
    #[test]
    fn winning_branches_do_not_read_lower_operands() {
        let unread = || Err("lower country/content operands are unavailable".into());
        let declared = |name: &str| name == "declared";
        let designed = || Ok(Some("declared".into()));
        let invalid_saved = VariantDefinitionRow {
            explicit_entity: vec![0xff],
            ..Default::default()
        };
        assert_eq!(
            resolve_lazy(
                &invalid_saved,
                0x100,
                &designed,
                &unread,
                &unread,
                &declared
            ),
            Ok(Some("declared".into()))
        );
        let saved = VariantDefinitionRow {
            explicit_entity: b"declared".to_vec(),
            ..Default::default()
        };
        assert_eq!(
            resolve_lazy(&saved, 0, &unread, &unread, &unread, &declared),
            Ok(Some("declared".into()))
        );
        assert_eq!(
            resolve_lazy(&saved, 0x100, &|| Ok(None), &unread, &unread, &declared),
            Ok(Some("declared".into()))
        );
        let empty = VariantDefinitionRow::default();
        assert_eq!(
            resolve_lazy(&empty, 0x100, &|| Ok(None), &designed, &unread, &declared),
            Ok(Some("declared".into()))
        );
        assert_eq!(
            resolve_lazy(
                &empty,
                0x100,
                &|| Ok(None),
                &|| Ok(Some("missing".into())),
                &unread,
                &declared
            ),
            Ok(None)
        );
        assert_eq!(
            resolve_lazy(&empty, 0, &unread, &unread, &unread, &declared),
            Err("lower country/content operands are unavailable".into())
        );
    }
    #[test]
    fn prefix_ladder_uses_each_equipment_archetype_sprite_before_the_next_prefix() {
        let names = candidates("COS", "TAG", "frame", "family", "sprite");
        assert_eq!(
            names,
            [
                "COS_frame_entity",
                "COS_family_entity",
                "COS_sprite_entity",
                "TAG_frame_entity",
                "TAG_family_entity",
                "TAG_sprite_entity",
                "frame_entity",
                "family_entity",
                "sprite_entity"
            ]
        );
        let empty = VariantDefinitionRow::default();
        for (index, expected) in names.iter().enumerate() {
            assert_eq!(
                select_fixture(
                    &empty,
                    0,
                    &|| panic!("designer is DLC gated"),
                    &|| panic!("override is DLC gated"),
                    &|name| names[index..].iter().any(|candidate| candidate == name)
                ),
                Ok(Some(expected.clone()))
            );
        }
        assert_eq!(
            candidates("", "TAG", "frame", "frame", "sprite"),
            [
                "TAG_frame_entity",
                "TAG_sprite_entity",
                "frame_entity",
                "sprite_entity"
            ]
        );
        assert_eq!(
            candidates("", "", "frame", "frame", ""),
            ["_frame_entity", "__entity", "frame_entity", "_entity"]
        );
    }
    #[test]
    fn prefix_hits_leave_later_operands_unread_and_keep_empty_sprite_keys() {
        let cosmetic = || Ok("COS".to_owned());
        let tag = || Err("country tag unavailable".to_owned());
        let keys = |index| {
            if index == 0 {
                Ok("frame".to_owned())
            } else {
                Err("later equipment operand unavailable".to_owned())
            }
        };
        assert_eq!(
            prefixed(&cosmetic, &tag, &keys, &|name| name == "COS_frame_entity"),
            Ok(Some("COS_frame_entity".to_owned()))
        );
        assert_eq!(
            prefixed(&cosmetic, &tag, &keys, &|_| false),
            Err("later equipment operand unavailable".to_owned())
        );
        assert_eq!(
            prefixed(
                &cosmetic,
                &tag,
                &|index| Ok(["frame", "family", ""][index].to_owned()),
                &|name| name == "COS__entity"
            ),
            Ok(Some("COS__entity".to_owned()))
        );
        assert_eq!(
            prefixed(
                &|| Ok(String::new()),
                &|| Ok("TAG".into()),
                &|index| Ok(["frame", "family", "sprite"][index].to_owned()),
                &|name| name == "TAG_frame_entity"
            ),
            Ok(Some("TAG_frame_entity".to_owned()))
        );
    }
    #[test]
    fn full_ladder_stops_at_declared_names_and_nonnull_overrides() {
        let variant = VariantDefinitionRow {
            explicit_entity: b"saved".to_vec(),
            ..Default::default()
        };
        let declared = |name: &str| {
            [
                "designed",
                "saved",
                "graphic",
                "COS_frame_entity",
                "TAG_frame_entity",
                "frame_entity",
            ]
            .contains(&name)
        };
        let select = |mask,
                      designer: &dyn Fn() -> Result<Option<String>, String>,
                      graphic: &dyn Fn() -> Result<Option<String>, String>| {
            select_fixture(&variant, mask, designer, graphic, &declared)
        };
        let absent = || Ok(None);
        let untouched = || panic!("lower ladder must not be evaluated");
        assert_eq!(
            select(0x100, &|| Ok(Some("designed".into())), &untouched)
                .unwrap()
                .as_deref(),
            Some("designed")
        );
        assert_eq!(
            select(0, &untouched, &untouched).unwrap().as_deref(),
            Some("saved")
        );
        let empty = VariantDefinitionRow::default();
        let select = |mask, graphic: &dyn Fn() -> Result<Option<String>, String>| {
            select_fixture(&empty, mask, &absent, graphic, &declared)
        };
        assert_eq!(
            select(0x100, &|| Ok(Some("graphic".into())))
                .unwrap()
                .as_deref(),
            Some("graphic")
        );
        assert_eq!(select(0x100, &|| Ok(Some("missing".into()))).unwrap(), None);
        assert_eq!(
            select(0, &untouched).unwrap().as_deref(),
            Some("COS_frame_entity")
        );
        assert!(select(0x100, &|| Err("unknown pool limit".into())).is_err());
    }
}
