//! Land raster coverage in map cells (`province_geometry::EngineMap`).
//! The province map is not equal-area: northern territory is enlarged.
use crate::{ResolvedLayers, fields::invalid, province_geometry::engine_map_checked};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug, Default)]
pub struct ProvinceAreas {
    /// Every defined province, including unpainted land (zero) and water (zero).
    pub land: Vec<i64>,
}

impl ProvinceAreas {
    pub const UNIT: &'static str = "map cells";

    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        Self::read_checked(layers, &mut || Ok(()))
    }

    /// Cancellation leaves no partial area vector for a consumer to cache.
    pub fn read_cancellable(layers: &ResolvedLayers, cancelled: &AtomicBool) -> io::Result<Self> {
        Self::read_checked(layers, &mut || {
            if cancelled.load(Ordering::Relaxed) {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Map area cancelled",
                ))
            } else {
                Ok(())
            }
        })
    }

    fn read_checked(
        layers: &ResolvedLayers,
        check: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        check()?;
        let files = crate::terrain::read_default_map(layers)?;
        let definition = files.file("definitions")?;
        let provinces = files.file("provinces")?;
        check()?;
        let definitions = layers.read(&definition)?;
        check()?;
        let raster = layers.read(&provinces)?;
        Self::from_assets_checked(&definitions, &raster, check).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Map area from {definition} and {provinces}: {error}"),
            )
        })
    }

    pub fn from_assets(definitions: &[u8], provinces: &[u8]) -> io::Result<Self> {
        Self::from_assets_checked(definitions, provinces, &mut || Ok(()))
    }

    fn from_assets_checked(
        definitions: &[u8],
        provinces: &[u8],
        check: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        check()?;
        // Refuse ambiguous record positions rather than attach area to a different id.
        let rows = crate::map::definition_rows(definitions)?;
        for (index, row) in rows.iter().enumerate() {
            if index % 4096 == 0 {
                check()?;
            }
            if usize::from(row.id) != index {
                return Err(invalid(
                    "definition.csv: map area requires province ids matching native record positions",
                ));
            }
        }
        let map = engine_map_checked(definitions, provinces, check)?;
        Self::from_map_checked(map, check)
    }

    fn from_map_checked(
        map: crate::province_geometry::EngineMap,
        check: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        check()?;
        // Native records include slot zero; `defined` contains only nonzero ids.
        let mut land = vec![0; map.defined.last().map_or(0, |id| *id as usize + 1)];
        let mut is_land = vec![false; land.len()];
        for (index, id) in map.land.into_iter().enumerate() {
            if index % 4096 == 0 {
                check()?;
            }
            *is_land
                .get_mut(id as usize)
                .ok_or_else(|| invalid(format!("land province id {id} has no native record")))? =
                true;
        }
        for (index, id) in map.ids.into_iter().enumerate() {
            if index % 4096 == 0 {
                check()?;
            }
            if is_land
                .get(id as usize)
                .copied()
                .ok_or_else(|| invalid(format!("province raster id {id} has no native record")))?
            {
                *land.get_mut(id as usize).ok_or_else(|| {
                    invalid(format!("province raster id {id} has no native record"))
                })? += 1;
            }
        }
        check()?;
        Ok(Self { land })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn bitmap(colors: &[[u8; 3]]) -> Vec<u8> {
        let stride = (colors.len() * 3).div_ceil(4) * 4;
        let mut bytes = vec![0; 54 + stride];
        bytes[..2].copy_from_slice(b"BM");
        bytes[2..6].copy_from_slice(&((54 + stride) as u32).to_le_bytes());
        bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
        bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&(colors.len() as i32).to_le_bytes());
        bytes[22..26].copy_from_slice(&1i32.to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
        for (index, [red, green, blue]) in colors.iter().enumerate() {
            bytes[54 + index * 3..57 + index * 3].copy_from_slice(&[*blue, *green, *red]);
        }
        bytes
    }

    #[test]
    fn cancelled_read_stops_before_resolving_assets() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let layers = ResolvedLayers::from_folders(&[]).unwrap();
        let error = ProvinceAreas::read_cancellable(&layers, &AtomicBool::new(true)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn area_count_cancels_after_processing_part_of_the_raster() {
        let map = crate::province_geometry::EngineMap {
            ids: vec![1; 9000],
            width: 9000,
            height: 1,
            defined: [1].into(),
            land: [1].into(),
            lake: Default::default(),
            nonsea_indices: [(1, 0)].into(),
        };
        let mut checks = 0;
        let error = ProvinceAreas::from_map_checked(map, &mut || {
            checks += 1;
            if checks == 4 {
                Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(checks, 4);
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn area_counts_land_coverage_only_and_preserves_unpainted_zero_and_invented_provinces() {
        let definitions = b"0;0;0;0;land\n1;1;2;3;land\n2;2;3;4;sea\n3;3;4;5;lake\n4;4;5;6;land\n";
        let areas = ProvinceAreas::from_assets(
            definitions,
            &bitmap(&[[1, 2, 3], [1, 2, 3], [2, 3, 4], [3, 4, 5], [9, 8, 7]]),
        )
        .unwrap();
        assert_eq!(areas.land[1], 2);
        assert_eq!(areas.land[2], 0);
        assert_eq!(areas.land[3], 0);
        assert_eq!(areas.land[4], 0);
        assert_eq!(areas.land[5], 1);
        assert_eq!(ProvinceAreas::UNIT, "map cells");
        assert!(
            ProvinceAreas::from_assets(b"0;0;0;0;land\n8;1;2;3;land\n", &bitmap(&[[1, 2, 3]]))
                .unwrap_err()
                .to_string()
                .contains("record positions")
        );
    }

    #[test]
    fn modded_layers_resolve_the_authored_map_paths_and_extra_provinces_and_name_refusals() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = std::env::temp_dir().join(format!(
            "territory-map-fixture-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let base = root.join("base");
        let dlc = root.join("dlc");
        let first_mod = root.join("first-mod");
        let last_mod = root.join("last-mod");
        for folder in [&base, &dlc, &first_mod, &last_mod] {
            std::fs::create_dir_all(folder.join("map")).unwrap();
        }
        for (folder, definitions, colors) in [
            (&base, "0;0;0;0;land\n1;1;2;3;land\n", vec![[1, 2, 3]]),
            (
                &dlc,
                "0;0;0;0;land\n1;1;2;3;land\n2;4;5;6;land\n",
                vec![[1, 2, 3], [4, 5, 6]],
            ),
            (
                &first_mod,
                "0;0;0;0;land\n1;1;2;3;land\n2;4;5;6;land\n3;7;8;9;land\n",
                vec![[1, 2, 3], [4, 5, 6], [7, 8, 9]],
            ),
            (
                &last_mod,
                "0;0;0;0;land\n1;1;2;3;land\n2;4;5;6;land\n3;7;8;9;land\n4;10;11;12;land\n",
                vec![[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12], [10, 11, 12]],
            ),
        ] {
            std::fs::write(folder.join("map/custom.csv"), definitions).unwrap();
            std::fs::write(folder.join("map/custom-territory.bmp"), bitmap(&colors)).unwrap();
        }
        std::fs::write(
            base.join("map/default.map"),
            "definitions = \"custom.csv\" provinces = \"custom-territory.bmp\" terrain = \"custom-terrain.bmp\" rivers = \"custom-rivers.bmp\" heightmap = \"custom-height.bmp\" tree_definition = \"custom-trees.bmp\"",
        )
        .unwrap();
        let layers = ResolvedLayers::from_folders(&[
            (&base, &[]),
            (&dlc, &[]),
            (&first_mod, &[]),
            (&last_mod, &[]),
        ])
        .unwrap();
        let areas = ProvinceAreas::read(&layers).unwrap();
        assert_eq!(areas.land.len(), 5);
        assert_eq!(areas.land[0], 0);
        assert_eq!(areas.land[4], 2);
        let mut checks = 0;
        let cancelled = ProvinceAreas::read_checked(&layers, &mut || {
            checks += 1;
            if checks == 4 {
                Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(cancelled.kind(), io::ErrorKind::Interrupted);
        assert!(cancelled.to_string().contains("custom-territory.bmp"));
        std::fs::write(
            last_mod.join("map/custom-territory.bmp"),
            b"unsupported bitmap form",
        )
        .unwrap();
        let changed = ProvinceAreas::read(&layers).unwrap_err().to_string();
        assert!(changed.contains("changed since verification"), "{changed}");
        assert!(changed.contains("custom-territory.bmp"), "{changed}");
        // A new content identity lets the bitmap reader inspect the unsupported form.
        let layers = ResolvedLayers::from_folders(&[
            (&base, &[]),
            (&dlc, &[]),
            (&first_mod, &[]),
            (&last_mod, &[]),
        ])
        .unwrap();
        let error = ProvinceAreas::read(&layers).unwrap_err().to_string();
        assert!(error.contains("custom-territory.bmp"), "{error}");
        assert!(error.contains("custom.csv"), "{error}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
