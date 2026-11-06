//! The native ordered province-neighbour lists.
//!
//! Shared by the movement map and `installed_content_save::naval_history::RailwayAdjacency`,
//! whose saved railway levels follow the same order.

use std::{collections::HashMap, io};

use crate::{ResolvedLayers, fields::invalid, terrain::DefaultMap};

/// The authored adjacency file, if the content has one. Its path starts
/// empty and is set to `map/` plus the `adjacencies` value. A missing key
/// or file means no authored adjacencies, while a present file that cannot
/// be read still fails.
pub fn read_authored(
    layers: &ResolvedLayers,
    default_map: &DefaultMap,
) -> io::Result<Option<Vec<u8>>> {
    let Some(path) = authored_path(default_map) else {
        return Ok(None);
    };
    match layers.read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                // A listing the recording withholds proves no absence.
                && !layers.paths().map_or(true, |mut paths| {
                    paths.any(|known| known.eq_ignore_ascii_case(path.as_bytes()))
                }) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// The `map/` path of the authored adjacency file; `None` without the key.
pub(crate) fn authored_path(default_map: &DefaultMap) -> Option<String> {
    default_map.file("adjacencies").ok()
}

fn append(lists: &mut [Vec<u32>], source: u32, target: u32) {
    let list = &mut lists[source as usize];
    if !list.contains(&target) {
        list.push(target);
    }
}

fn append_pair(lists: &mut [Vec<u32>], source: u32, target: u32) {
    if target != 0 && source != target && !lists[source as usize].contains(&target) {
        // Raster pairs append the reciprocal first.
        append(lists, target, source);
        append(lists, source, target);
    }
}

/// Scans pixels in file order: right, next row, left, previous row,
/// wrapping horizontally, then the next-row diagonal of a four-province
/// crossing. `ids` holds file rows (row 0 is the bottom) of record IDs
/// below `count`; the result is indexed by ID.
pub fn raster(width: usize, height: usize, ids: &[u32], count: usize) -> io::Result<Vec<Vec<u32>>> {
    if width == 0
        || height == 0
        || width.checked_mul(height) != Some(ids.len())
        || ids.iter().any(|&id| id as usize >= count)
    {
        return Err(invalid(
            "adjacency raster dimensions or province identities are invalid",
        ));
    }
    let mut lists = vec![Vec::new(); count];
    let at = |x: usize, y: usize| ids[y * width + x];
    for y in 0..height {
        for x in 0..width {
            let source = at(x, y);
            if source == 0 {
                continue;
            }
            let right_x = if x + 1 == width { 0 } else { x + 1 };
            let left_x = if x == 0 { width - 1 } else { x - 1 };
            let right = at(right_x, y);
            let next = if y + 1 < height { at(x, y + 1) } else { 0 };
            let left = at(left_x, y);
            let previous = if y > 0 { at(x, y - 1) } else { 0 };
            for target in [right, next, left, previous] {
                append_pair(&mut lists, source, target);
            }
            let diagonal = if y + 1 < height {
                at(right_x, y + 1)
            } else {
                0
            };
            let four = [source, right, next, diagonal];
            if four.iter().all(|&id| id != 0)
                && (0..4).all(|a| (a + 1..4).all(|b| four[a] != four[b]))
            {
                append_pair(&mut lists, source, diagonal);
            }
        }
    }
    Ok(lists)
}

/// Applies authored rows ([`crate::adjacencies::parse`]) in file order.
/// New pairs append source then target; a repeated pair keeps its position
/// and takes the row's crossing kind and raw signed Through in both directions.
pub fn append_authored(
    lists: &mut [Vec<u32>],
    kinds: &mut HashMap<(u32, u32), (u8, i32)>,
    authored: &[u8],
) -> io::Result<()> {
    let count = lists.len();
    for row in crate::adjacencies::parse(authored)? {
        let (source, target) = (row.from, row.to);
        if source as usize >= count || target == 0 || target as usize >= count {
            return Err(invalid(format!(
                "authored adjacency {source};{target} names an unknown province"
            )));
        }
        append(lists, source, target);
        append(lists, target, source);
        kinds.insert((source, target), (row.kind, row.through));
        kinds.insert((target, source), (row.kind, row.through));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Native file rows, bottom first.
    fn lists(rows: &[&[u32]], count: usize, authored: &[u8]) -> Vec<Vec<u32>> {
        let ids: Vec<u32> = rows.iter().flat_map(|row| row.iter().copied()).collect();
        let mut lists = raster(rows[0].len(), rows.len(), &ids, count).unwrap();
        append_authored(&mut lists, &mut HashMap::new(), authored).unwrap();
        lists
    }

    #[test]
    fn raster_scan_keeps_reciprocal_insertion_and_authored_append_order() {
        let lists = lists(
            &[&[1, 2], &[3, 4]],
            6,
            b"header\n1;5;sea;-1;0;0;0;0;;new\n2;2;impassable;-1;-1;-1;-1;-1;;self\n-1\n1;9;sea\n",
        );
        assert_eq!(lists[1], [2, 3, 4, 5]);
        assert_eq!(lists[2], [1, 4, 3, 2]);
        assert_eq!(lists[3], [1, 2, 4]);
        assert_eq!(lists[4], [1, 2, 3]);
        assert_eq!(lists[5], [1]);
    }

    #[test]
    fn a_default_map_without_adjacencies_names_no_authored_file() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let map = |extra: &str| {
            let text = format!(
                "definitions = d.csv provinces = p.bmp terrain = t.bmp rivers = r.bmp
                 heightmap = h.bmp tree_definition = tr.bmp adjacency_rules = rules.txt {extra}"
            );
            crate::terrain::parse_default_map(text.as_bytes()).unwrap()
        };
        assert_eq!(authored_path(&map("")), None);
        assert_eq!(
            authored_path(&map("adjacencies = old.csv adjacencies = sub/new.csv")),
            Some("map/sub/new.csv".to_owned())
        );
    }

    #[test]
    fn repeated_rows_update_kinds_and_a_missing_target_keeps_the_previous_one() {
        let ids = [1, 2, 1, 3];
        let mut lists = raster(4, 1, &ids, 4).unwrap();
        assert_eq!(lists[2], [1]);
        let mut kinds = HashMap::new();
        append_authored(
            &mut lists,
            &mut kinds,
            b"h\n1;3;sea;-1;-1;-1;-1;-1;;\n1;3;river;-1;-1;-1;-1;-1;;\n2\n",
        )
        .unwrap();
        assert_eq!(kinds[&(3, 1)], (2, -1));
        assert_eq!(lists[1], [2, 3]);
        // Row "2" has no readable target, so it reuses 3.
        assert_eq!(lists[2], [1, 3]);
        assert_eq!(kinds[&(3, 2)], (0, -1));
        assert!(append_authored(&mut lists, &mut kinds, b"h\n1;4;sea;-1;-1;-1;-1;-1;;\n").is_err());
    }
}
