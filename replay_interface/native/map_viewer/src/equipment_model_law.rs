use crate::Result;

pub fn normalize_equipment_category(mask: u64) -> u64 {
    if mask & 0x80004003c1 != 0 {
        if mask & 0x40 != 0 {
            return mask & 0xfffffffffffffe7e;
        }
        if mask & 0x80 != 0 {
            return mask & 0xfffffffffffffefe;
        }
        if mask & 0x100 != 0 {
            return mask & 0xfffffffffffffffe;
        }
    }
    mask
}

pub fn equipment_model_category(
    definition_mask: u64,
    archetype_mask: Option<u64>,
    additional_mask: u64,
) -> Result<u64> {
    let base = if definition_mask == 0 {
        archetype_mask.ok_or("equipment category requires the authenticated archetype mask")?
    } else {
        definition_mask
    };
    Ok(normalize_equipment_category(base | additional_mask))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_normalization_keeps_unsigned_bits_and_the_engine_hierarchy() {
        for high in [0, 1 << 39, 1 << 63, 0x1234567800000000] {
            for low in 0u64..1024 {
                let mask = high | low;
                let expected = if low & 0x40 != 0 {
                    mask & !0x181
                } else if low & 0x80 != 0 {
                    mask & !0x101
                } else if low & 0x100 != 0 {
                    mask & !1
                } else {
                    mask
                };
                assert_eq!(normalize_equipment_category(mask), expected);
            }
        }
        assert_eq!(equipment_model_category(0x80, None, 0x100).unwrap(), 0x80);
        assert_eq!(
            equipment_model_category(0, Some(0x100), 0x40).unwrap(),
            0x40
        );
        assert_eq!(equipment_model_category(0, Some(0), 0).unwrap(), 0);
        assert!(equipment_model_category(0, None, 0x40).is_err());
    }

    #[test]
    fn category_model_inputs_follow_the_pinned_loader_and_normalizer() {
        let exports = portable_engine_evidence::xmap13_model_selection::EXPORTS;
        let definition = exports
            .iter()
            .find(|(name, _)| *name == "func_140c85fe0.decompile.c")
            .unwrap()
            .1;
        assert!(definition.contains("param_1 + 0x540"));
        assert!(definition.contains("uVar1 == 0"));
        assert!(definition.contains("param_1 + 0x4d8"));
        assert!(definition.contains("param_1 + 0x548"));
        let normalize = exports
            .iter()
            .find(|(name, _)| *name == "func_140f75ea0.decompile.c")
            .unwrap()
            .1;
        for token in [
            "0x80004003c1",
            "0xfffffffffffffe7e",
            "0xfffffffffffffefe",
            "0xfffffffffffffffe",
        ] {
            assert!(normalize.contains(token), "missing {token}");
        }
    }
}
