#[cfg(any(test, feature = "test-support"))]
use crate::province_geometry::engine_map_checked;
#[cfg(any(test, feature = "test-support"))]
use content_reader_parser::fields::invalid;
use crate::{ResolvedLayers, province_geometry::land_areas::LandAreas};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug, Default)]
pub struct ProvinceAreas {
    pub land: Vec<i64>,
}

impl ProvinceAreas {
    pub const UNIT: &'static str = "map cells";

    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        Self::read_checked(layers, &mut || Ok(()))
    }

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
        Self::stream_checked(definitions, |piece| layers.read_pieces(&provinces, piece), check).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Map area from {definition} and {provinces}: {error}"),
            )
        })
    }

    fn stream_checked(
        definitions: content_reader_layers::content::Bytes,
        raster: impl FnOnce(&mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<()>,
        check: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        check()?;
        let mut counted = LandAreas::new(&definitions)?;
        drop(definitions);
        raster(&mut |piece: &[u8]| {
            check()?;
            counted.piece(piece)
        })?;
        check()?;
        Ok(Self { land: counted.finish()? })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_pieces(definitions: &[u8], pieces: &[&[u8]]) -> io::Result<Self> {
        Self::stream_checked(
            definitions.into(),
            |piece| pieces.iter().try_for_each(|bytes| piece(bytes)),
            &mut || Ok(()),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_assets(definitions: &[u8], provinces: &[u8]) -> io::Result<Self> {
        Self::from_assets_checked(definitions, provinces, &mut || Ok(()))
    }

    #[cfg(any(test, feature = "test-support"))]
    fn from_assets_checked(
        definitions: &[u8],
        provinces: &[u8],
        check: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        check()?;
        let rows = content_reader_map::map::definition_rows(definitions)?;
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

    #[cfg(any(test, feature = "test-support"))]
    fn from_map_checked(
        map: crate::province_geometry::EngineMap,
        check: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        check()?;
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
    fn streamed_areas_equal_the_whole_map_count_in_any_pieces() {
        let definitions = b"0;0;0;0;land\n1;1;2;3;land\n2;2;3;4;sea\n3;3;4;5;lake\n4;4;5;6;land\n5;7;7;7;land\n";
        let mut colours = Vec::new();
        for y in 0..7 {
            for x in 0..5 {
                colours.push(match (x + 2 * y) % 6 {
                    0 => [1, 2, 3],
                    1 => [2, 3, 4],
                    2 => [4, 5, 6],
                    3 => [9, 8, 7],
                    4 => [1, 2, 3],
                    _ => [7, 7, 7 + (y as u8 % 2)],
                });
            }
        }
        let wide = bitmap_rows(&colours, 5, 7);
        let mut indexed = vec![0_u8; 54 + 1024];
        indexed[..2].copy_from_slice(b"BM");
        indexed[10..14].copy_from_slice(&(54_u32 + 1024).to_le_bytes());
        indexed[14..18].copy_from_slice(&40_u32.to_le_bytes());
        indexed[18..22].copy_from_slice(&6_i32.to_le_bytes());
        indexed[22..26].copy_from_slice(&4_i32.to_le_bytes());
        indexed[26..28].copy_from_slice(&1_u16.to_le_bytes());
        indexed[28..30].copy_from_slice(&8_u16.to_le_bytes());
        for row in 0..4_u8 {
            indexed.extend([1, 2, 3, row, 5, 6, 7, 8]);
        }
        let mut narrow = indexed[..54].to_vec();
        narrow[10..14].copy_from_slice(&54_u32.to_le_bytes());
        narrow[18..22].copy_from_slice(&3_i32.to_le_bytes());
        narrow[28..30].copy_from_slice(&1_u16.to_le_bytes());
        narrow.extend([0; 8]);
        for raster in [&wide, &bitmap(&[[1, 2, 3], [4, 5, 6], [9, 9, 9]]), &indexed, &narrow] {
            let whole = ProvinceAreas::from_assets(definitions, raster).map(|areas| areas.land).map_err(|error| error.to_string());
            for size in [1, 2, 3, 7, 54, 64, raster.len()] {
                let pieces: Vec<&[u8]> = raster.chunks(size).collect();
                let streamed = ProvinceAreas::from_pieces(definitions, &pieces).map(|areas| areas.land).map_err(|error| error.to_string());
                assert_eq!(streamed, whole, "pieces of {size} bytes");
            }
        }
        assert!(ProvinceAreas::from_assets(definitions, &wide).unwrap().land.iter().any(|&cells| cells > 0));
        let short = &wide[..wide.len() - 5];
        let refused = ProvinceAreas::from_pieces(definitions, &short.chunks(9).collect::<Vec<_>>()).unwrap_err().to_string();
        assert_eq!(refused, ProvinceAreas::from_assets(definitions, short).unwrap_err().to_string());
    }

    fn bitmap_rows(colours: &[[u8; 3]], width: usize, height: usize) -> Vec<u8> {
        let stride = (width * 3).div_ceil(4) * 4;
        let mut bytes = vec![0; 54 + stride * height];
        bytes[..2].copy_from_slice(b"BM");
        bytes[2..6].copy_from_slice(&((54 + stride * height) as u32).to_le_bytes());
        bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
        bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        bytes[22..26].copy_from_slice(&(height as i32).to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
        for (index, [red, green, blue]) in colours.iter().enumerate() {
            let at = 54 + (index / width) * stride + (index % width) * 3;
            bytes[at..at + 3].copy_from_slice(&[*blue, *green, *red]);
        }
        bytes
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
        let changed = ProvinceAreas::read(&layers).unwrap_err();
        assert_eq!(changed.kind(), io::ErrorKind::InvalidData);
        let changed = changed.to_string();
        assert!(changed.contains("changed since verification"), "{changed}");
        assert!(changed.contains("custom-territory.bmp"), "{changed}");
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
