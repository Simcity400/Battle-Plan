use crate::*;
use std::fs;
use std::path::Path;

const SEASONS_SOURCE: &str = "map/seasons.txt";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SeasonId {
    Winter,
    Spring,
    Summer,
    Autumn,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TreeSeasonId {
    Winter,
    Winter2,
    Spring,
    Spring2,
    Summer,
    Summer2,
    Autumn,
    Autumn2,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SeasonDate {
    pub month: u8,
    pub day: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeasonDateRange {
    pub start: SeasonDate,
    pub end: SeasonDate,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeasonColorGrade {
    pub hsv: [f32; 3],
    pub color_balance: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeasonBlock {
    pub id: SeasonId,
    pub date_range: SeasonDateRange,
    pub north: SeasonColorGrade,
    pub center: SeasonColorGrade,
    pub south: SeasonColorGrade,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeasonsTable {
    pub winter: SeasonBlock,
    pub spring: SeasonBlock,
    pub summer: SeasonBlock,
    pub autumn: SeasonBlock,
    pub tree_winter: SeasonDateRange,
    pub tree_winter2: SeasonDateRange,
    pub tree_spring: SeasonDateRange,
    pub tree_spring2: SeasonDateRange,
    pub tree_summer: SeasonDateRange,
    pub tree_summer2: SeasonDateRange,
    pub tree_autumn: SeasonDateRange,
    pub tree_autumn2: SeasonDateRange,
}

impl SeasonId {
    fn source_key(self) -> &'static str {
        match self {
            Self::Winter => "winter",
            Self::Spring => "spring",
            Self::Summer => "summer",
            Self::Autumn => "autumn",
        }
    }
}

impl TreeSeasonId {
    fn source_key(self) -> &'static str {
        match self {
            Self::Winter => "tree_winter",
            Self::Winter2 => "tree_winter2",
            Self::Spring => "tree_spring",
            Self::Spring2 => "tree_spring2",
            Self::Summer => "tree_summer",
            Self::Summer2 => "tree_summer2",
            Self::Autumn => "tree_autumn",
            Self::Autumn2 => "tree_autumn2",
        }
    }
}

impl SeasonDate {
    pub fn new(month: u8, day: u8) -> Result<Self> {
        if (1..=12).contains(&month) && (1..=31).contains(&day) {
            Ok(Self { month, day })
        } else {
            Err(format!("map/seasons.txt has invalid wildcard date 00.{month:02}.{day:02}").into())
        }
    }
}

impl SeasonDateRange {
    fn contains(self, date: SeasonDate) -> bool {
        if self.start <= self.end {
            self.start <= date && date <= self.end
        } else {
            date >= self.start || date <= self.end
        }
    }
}

impl SeasonsTable {
    pub fn season(&self, id: SeasonId) -> &SeasonBlock {
        match id {
            SeasonId::Winter => &self.winter,
            SeasonId::Spring => &self.spring,
            SeasonId::Summer => &self.summer,
            SeasonId::Autumn => &self.autumn,
        }
    }

    pub fn tree_range(&self, id: TreeSeasonId) -> SeasonDateRange {
        match id {
            TreeSeasonId::Winter => self.tree_winter,
            TreeSeasonId::Winter2 => self.tree_winter2,
            TreeSeasonId::Spring => self.tree_spring,
            TreeSeasonId::Spring2 => self.tree_spring2,
            TreeSeasonId::Summer => self.tree_summer,
            TreeSeasonId::Summer2 => self.tree_summer2,
            TreeSeasonId::Autumn => self.tree_autumn,
            TreeSeasonId::Autumn2 => self.tree_autumn2,
        }
    }

    pub fn season_for_date(&self, month: u8, day: u8) -> Option<SeasonId> {
        let date = SeasonDate::new(month, day).ok()?;
        [
            SeasonId::Winter,
            SeasonId::Spring,
            SeasonId::Summer,
            SeasonId::Autumn,
        ]
        .into_iter()
        .find(|id| self.season(*id).date_range.contains(date))
    }
}

/// The eight tree-season blocks in the same file order as `map/seasons.txt`
/// and the column order sampled by `gfx/FX/tree.shader`.
const TREE_SEASON_FILE_ORDER: [TreeSeasonId; 8] = [
    TreeSeasonId::Winter,
    TreeSeasonId::Winter2,
    TreeSeasonId::Spring,
    TreeSeasonId::Spring2,
    TreeSeasonId::Summer,
    TreeSeasonId::Summer2,
    TreeSeasonId::Autumn,
    TreeSeasonId::Autumn2,
];

const TREE_SEASON_YEAR_DAYS: u32 = 365;
const TREE_SEASON_MONTH_DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
const TREE_SEASON_CB1_LERP_OFFSET: usize = 64;
const TREE_SEASON_CB1_COLUMN_OFFSET: usize = 68;
const TREE_SEASON_CB1_OVERRIDE_END: usize = 72;

/// Best-hypothesis tree-season CB1 law for `gfx/FX/tree.shader`.
///
/// Hypothesis: `vSeasonColumn` is the raw 0..7 tree-season block index in the
/// file order above, not the shader's centered U coordinate. The shader itself
/// computes `vSeasonColumn / 8 + 1 / 16`. `vSeasonLerp` is 0 inside a block. In
/// a gap, the previous column stays active and `vSeasonLerp` linearly ramps
/// from 0 at block N end toward 1 at block N+1 start, wrapping across year-end.
///
/// This remains a hypothesis pending broader dated capture evidence. Current
/// constraint set from the local 2026-07-04 capture census:
/// valid 80-byte `tree_pixel_shader_277b7b5e8964214f` tree captures all had
/// `(vSeasonLerp=0, vSeasonColumn=0)` with varying `vTreeFade`; their PS t4 was
/// `ColorMap` and PS t5 `ColorMapSecond` was null. Older 112-byte captures
/// decoded as `(2048, 1024, 8)` at these offsets while binding unrelated t4/t5
/// resources, so they are reported as observed tuples but are not constraints
/// for this verified 80-byte `tree.shader` CB layout.
pub fn tree_season_cb_values(seasons: &SeasonsTable, month: u32, day: u32) -> Option<(f32, f32)> {
    let month = u8::try_from(month).ok()?;
    let day = u8::try_from(day).ok()?;
    let date = SeasonDate::new(month, day).ok()?;
    let date_day = tree_date_to_day_of_year(date)?;
    let ranges = tree_season_ranges_by_file_order(seasons)?;

    for (index, range) in ranges.iter().enumerate() {
        if tree_range_contains_day(*range, date_day) {
            return Some((0.0, index as f32));
        }
    }

    for index in 0..ranges.len() {
        let current = ranges[index];
        let next = ranges[(index + 1) % ranges.len()];
        let total = tree_forward_days(current.end, next.start);
        if total <= 1 {
            continue;
        }
        let elapsed = tree_forward_days(current.end, date_day);
        if (1..total).contains(&elapsed) {
            return Some((elapsed as f32 / total as f32, index as f32));
        }
    }

    None
}

/// Apply an already-computed tree-season override to captured tree CB1 bytes.
///
/// `None` is the flag-off/no-date path and leaves the buffer byte-identical.
/// `Some((lerp, column))` writes only `vSeasonLerp` and the raw
/// `vSeasonColumn` float at bytes 64..72; it deliberately does not touch
/// `vTreeFade` at byte 72.
pub fn apply_tree_season_cb_override(cb1_bytes: &mut [u8], values: Option<(f32, f32)>) -> bool {
    let Some((lerp, column)) = values else {
        return false;
    };
    if cb1_bytes.len() < TREE_SEASON_CB1_OVERRIDE_END {
        return false;
    }
    cb1_bytes[TREE_SEASON_CB1_LERP_OFFSET..TREE_SEASON_CB1_COLUMN_OFFSET]
        .copy_from_slice(&lerp.to_le_bytes());
    cb1_bytes[TREE_SEASON_CB1_COLUMN_OFFSET..TREE_SEASON_CB1_OVERRIDE_END]
        .copy_from_slice(&column.to_le_bytes());
    true
}

#[derive(Clone, Copy)]
struct TreeSeasonRangeDays {
    start: u32,
    end: u32,
}

fn tree_season_ranges_by_file_order(seasons: &SeasonsTable) -> Option<[TreeSeasonRangeDays; 8]> {
    let ranges = TREE_SEASON_FILE_ORDER.map(|id| {
        let range = seasons.tree_range(id);
        Some(TreeSeasonRangeDays {
            start: tree_date_to_day_of_year(range.start)?,
            end: tree_date_to_day_of_year(range.end)?,
        })
    });
    let ranges: [TreeSeasonRangeDays; 8] = [
        ranges[0]?, ranges[1]?, ranges[2]?, ranges[3]?, ranges[4]?, ranges[5]?, ranges[6]?,
        ranges[7]?,
    ];
    tree_season_ranges_are_non_overlapping(&ranges).then_some(ranges)
}

fn tree_date_to_day_of_year(date: SeasonDate) -> Option<u32> {
    let month_index = usize::from(date.month.checked_sub(1)?);
    let month_days = *TREE_SEASON_MONTH_DAYS.get(month_index)?;
    let day = u32::from(date.day);
    if day == 0 || day > month_days {
        return None;
    }
    Some(TREE_SEASON_MONTH_DAYS[..month_index].iter().sum::<u32>() + day - 1)
}

fn tree_range_contains_day(range: TreeSeasonRangeDays, day: u32) -> bool {
    if range.start <= range.end {
        range.start <= day && day <= range.end
    } else {
        day >= range.start || day <= range.end
    }
}

fn tree_forward_days(from: u32, to: u32) -> u32 {
    if to >= from {
        to - from
    } else {
        TREE_SEASON_YEAR_DAYS - from + to
    }
}

fn tree_season_ranges_are_non_overlapping(ranges: &[TreeSeasonRangeDays; 8]) -> bool {
    let mut covered = [false; TREE_SEASON_YEAR_DAYS as usize];
    for range in ranges {
        let mut day = range.start;
        loop {
            let slot = &mut covered[day as usize];
            if *slot {
                return false;
            }
            *slot = true;
            if day == range.end {
                break;
            }
            day = (day + 1) % TREE_SEASON_YEAR_DAYS;
        }
    }
    true
}

/// Test-only convenience wrapper; production callers pass a `GameFileResolver`.
pub fn seasons_table_from_game_dir(game_dir: &Path) -> Result<SeasonsTable> {
    let resolver = GameFileResolver::vanilla(game_dir.to_path_buf())?;
    seasons_table_from_resolver(&resolver)
}

pub fn seasons_table_from_resolver(resolver: &GameFileResolver) -> Result<SeasonsTable> {
    let path = resolver.resolve_required(SEASONS_SOURCE, "seasons.txt")?;
    let bytes = fs::read(&path).map_err(|err| {
        format!(
            "failed to read {SEASONS_SOURCE} resolved to {}: {err}",
            path.display()
        )
    })?;
    seasons_table_from_source(&latin1_bytes_to_string(&bytes)).map_err(|err| {
        format!(
            "failed to parse {SEASONS_SOURCE} resolved to {}: {err}",
            path.display()
        )
        .into()
    })
}

pub fn seasons_table_from_source(source: &str) -> Result<SeasonsTable> {
    let text = strip_comments(source);
    let winter = parse_season_block(&text, SeasonId::Winter)?;
    let spring = parse_season_block(&text, SeasonId::Spring)?;
    let summer = parse_season_block(&text, SeasonId::Summer)?;
    let autumn = parse_season_block(&text, SeasonId::Autumn)?;
    let tree_winter = parse_tree_range(&text, TreeSeasonId::Winter)?;
    let tree_winter2 = parse_tree_range(&text, TreeSeasonId::Winter2)?;
    let tree_spring = parse_tree_range(&text, TreeSeasonId::Spring)?;
    let tree_spring2 = parse_tree_range(&text, TreeSeasonId::Spring2)?;
    let tree_summer = parse_tree_range(&text, TreeSeasonId::Summer)?;
    let tree_summer2 = parse_tree_range(&text, TreeSeasonId::Summer2)?;
    let tree_autumn = parse_tree_range(&text, TreeSeasonId::Autumn)?;
    let tree_autumn2 = parse_tree_range(&text, TreeSeasonId::Autumn2)?;
    Ok(SeasonsTable {
        winter,
        spring,
        summer,
        autumn,
        tree_winter,
        tree_winter2,
        tree_spring,
        tree_spring2,
        tree_summer,
        tree_summer2,
        tree_autumn,
        tree_autumn2,
    })
}

fn parse_season_block(source: &str, id: SeasonId) -> Result<SeasonBlock> {
    let key = id.source_key();
    let block = required_block(source, key)?;
    Ok(SeasonBlock {
        id,
        date_range: parse_date_range(&block, key)?,
        north: parse_color_grade(&block, key, "north")?,
        center: parse_color_grade(&block, key, "center")?,
        south: parse_color_grade(&block, key, "south")?,
    })
}

fn parse_tree_range(source: &str, id: TreeSeasonId) -> Result<SeasonDateRange> {
    let key = id.source_key();
    parse_date_range(&required_block(source, key)?, key)
}

fn required_block(source: &str, key: &str) -> Result<String> {
    extract_block_after(source, key, 0)
        .map(|(inner, _)| inner)
        .ok_or_else(|| format!("map/seasons.txt is missing required block {key}").into())
}

fn parse_date_range(block: &str, block_name: &str) -> Result<SeasonDateRange> {
    Ok(SeasonDateRange {
        start: parse_date_value(block, block_name, "start_date")?,
        end: parse_date_value(block, block_name, "end_date")?,
    })
}

fn parse_date_value(block: &str, block_name: &str, key: &str) -> Result<SeasonDate> {
    let raw = parse_token_value(block, key)
        .ok_or_else(|| format!("map/seasons.txt {block_name} is missing {key}"))?;
    parse_wildcard_date(&raw)
        .map_err(|err| format!("map/seasons.txt {block_name} {key}={raw:?}: {err}").into())
}

fn parse_wildcard_date(raw: &str) -> Result<SeasonDate> {
    let parts = raw.split('.').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "00" {
        return Err("expected wildcard-year date 00.MM.DD".into());
    }
    let month = parts[1]
        .parse::<u8>()
        .map_err(|err| format!("month is not a number: {err}"))?;
    let day = parts[2]
        .parse::<u8>()
        .map_err(|err| format!("day is not a number: {err}"))?;
    SeasonDate::new(month, day)
}

fn parse_color_grade(block: &str, block_name: &str, band: &str) -> Result<SeasonColorGrade> {
    Ok(SeasonColorGrade {
        hsv: parse_vec3_assignment(block, block_name, &format!("hsv_{band}"))?,
        color_balance: parse_vec3_assignment(block, block_name, &format!("colorbalance_{band}"))?,
    })
}

fn parse_vec3_assignment(block: &str, block_name: &str, key: &str) -> Result<[f32; 3]> {
    let idx = find_assignment(block, key, 0)
        .ok_or_else(|| format!("map/seasons.txt {block_name} is missing {key}"))?;
    let value = &block[idx..];
    let brace = value
        .find('{')
        .ok_or_else(|| format!("map/seasons.txt {block_name} {key} is missing a value block"))?;
    let inner = extract_brace_at(value, brace)
        .ok_or_else(|| format!("map/seasons.txt {block_name} {key} has an unclosed value block"))?;
    let nums = inner
        .split_whitespace()
        .map(|part| part.parse::<f32>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|err| {
            format!("map/seasons.txt {block_name} {key} has a non-float value: {err}")
        })?;
    if nums.len() != 3 || nums.iter().any(|value| !value.is_finite()) {
        return Err(format!(
            "map/seasons.txt {block_name} {key} expected exactly three finite floats"
        )
        .into());
    }
    Ok([nums[0], nums[1], nums[2]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::path::{Path, PathBuf};

    const VANILLA_SEASONS_TXT: &str = r#"#		winter = {
#			start_date=00.12.01
#			end_date=00.02.31
#			
#			hsv_north=  		 { 0.2 0.1 0.6 }
#			colorbalance_north=  { 0.8 0.8 1.1	}
#			
#			hsv_center=			 { 0.0 0.85 1.0 }
#			colorbalance_center= { 1.1 1.0 1.0  }
#			
#			hsv_south= 			 { 0.0 0.85 1.0 }	
#			colorbalance_south=  { 1.1 1.0 1.0  }		
#		}
#		
#		
#		spring = {
#			start_date=00.04.01
#			end_date=00.05.1	
#			
#			hsv_north=  		{ 0.2 0.1 0.6 }
#			colorbalance_north= { 1.1 1.0 1.2   }
#			
#			hsv_center= 		 { 0.0 0.85 1.0 }
#			colorbalance_center= { 1.1 1.1 1.0  }
#			
#			hsv_south= 			 { 0.0 0.85 1.0 }	
#			colorbalance_south=  { 1.1 1.0 1.0  }	
#		}
#		
#		
#		summer = {
#			start_date=00.06.01
#			end_date=00.09.10
#			
#			hsv_north= 			 { 0.2 0.1 0.6  }
#			colorbalance_north=  { 1.3 1.15 0.9  }
#			
#			hsv_center= 		 { 0.0 0.85 1.0 }
#			colorbalance_center= { 1.0 1.0 1.0  }
#			
#			hsv_south= 			 { 0.0 0.85 1.0 }	
#			colorbalance_south=  { 1.0 1.0 1.0  }		
#		}
#		
#		
#		autumn = {
#			start_date=00.10.10
#			end_date=00.10.31
#		
#			hsv_north=  		 { 0.2 0.1 0.6 }
#			colorbalance_north=  { 1.3 1.1 0.8   }
#			
#			hsv_center= 		 { 0.0 0.85 1.0  }
#			colorbalance_center= { 1.1 1.0 1.0   }
#			
#			hsv_south=  		 { 0.0 0.85 1.0  }	
#			colorbalance_south=  { 1.1 1.0 1.0   }		
#		
#		}



winter = {
	start_date=00.12.01
	end_date=00.02.10
	
	hsv_north=  		 { 0 0.1 1 }
	colorbalance_north=  { 0.9 0.9 1 }	
	
	hsv_center=			 { 0.0 1.0 1.0 }
	colorbalance_center= { 1.0 1.0 1.0  }
	
	hsv_south= 			 { 0.0 1.0 1.0 }	
	colorbalance_south=  { 1.0 1.0 1.0  }	

	
}


spring = {
	start_date=00.03.10
	end_date=00.04.22	
	
	hsv_north=  		 { 0 0.1 1 }
	colorbalance_north=  { 0.9 0.9 1 }	
	
	hsv_center=			 { 0.0 1.0 1.0 }
	colorbalance_center= { 1.0 1.0 1.0  }
	
	hsv_south= 			 { 0.0 1.0 1.0 }	
	colorbalance_south=  { 1.0 1.0 1.0  }			
}


summer = {
	start_date=00.05.20
	end_date=00.09.10

	hsv_north=  		 { 0 0.1 1 }
	colorbalance_north=  { 0.9 0.9 1 }	
	
	hsv_center=			 { 0.0 1.0 1.0 }
	colorbalance_center= { 1.0 1.0 1.0  }
	
	hsv_south= 			 { 0.0 1.0 1.0 }	
	colorbalance_south=  { 1.0 1.0 1.0  }		
}


autumn = {
	start_date=00.10.10
	end_date=00.10.31

	hsv_north=  		 { 0 0.1 1 }
	colorbalance_north=  { 0.9 0.9 1 }	
	
	hsv_center=			 { 0.0 1.0 1.0 }
	colorbalance_center= { 1.0 1.0 1.0  }
	
	hsv_south= 			 { 0.0 1.0 1.0 }	
	colorbalance_south=  { 1.0 1.0 1.0  }				

}

tree_winter = {
	start_date=00.11.15
	end_date=00.12.01
}
tree_winter2 = {
	start_date=00.12.20
	end_date=00.01.20
}
tree_spring = {
	start_date=00.02.20
	end_date=00.03.01	
}
tree_spring2 = {
	start_date=00.03.20
	end_date=00.04.20	
}
tree_summer = {
	start_date=00.05.20
	end_date=00.06.01	
}
tree_summer2 = {
	start_date=00.06.20
	end_date=00.09.10
}
tree_autumn = {
	start_date=00.10.01
	end_date=00.10.10
}
tree_autumn2 = {
	start_date=00.10.25
	end_date=00.11.01
}
"#;

    fn path_from_rel(root: &Path, rel: &str) -> PathBuf {
        rel.split('/')
            .filter(|part| !part.is_empty())
            .fold(root.to_path_buf(), |path, part| path.join(part))
    }

    fn write_text(root: &Path, rel: &str, text: &str) {
        let path = path_from_rel(root, rel);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, text).expect("write fixture file");
    }

    #[test]
    fn vanilla_file_parses_and_ignores_commented_example_block() {
        assert_eq!(VANILLA_SEASONS_TXT.lines().count(), 156);
        let table = seasons_table_from_source(VANILLA_SEASONS_TXT).expect("parse vanilla seasons");
        assert_eq!(table.winter.north.hsv, [0.0, 0.1, 1.0]);
        assert_ne!(table.winter.north.hsv, [0.2, 0.1, 0.6]);
        assert_eq!(table.winter.north.color_balance, [0.9, 0.9, 1.0]);
        assert_eq!(table.spring.center.hsv, [0.0, 1.0, 1.0]);
        assert_eq!(table.autumn.south.color_balance, [1.0, 1.0, 1.0]);
        assert_eq!(
            table.tree_range(TreeSeasonId::Winter2),
            SeasonDateRange {
                start: SeasonDate { month: 12, day: 20 },
                end: SeasonDate { month: 1, day: 20 },
            }
        );
    }

    #[test]
    fn seasons_from_resolver_uses_mod_override() {
        let root = temp_test_dir("seasons_resolver_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, SEASONS_SOURCE, VANILLA_SEASONS_TXT);
        let modded = VANILLA_SEASONS_TXT.replace("start_date=00.12.01", "start_date=00.11.01");
        write_text(&mod_root, SEASONS_SOURCE, &modded);

        let resolver =
            GameFileResolver::with_mod_paths(vanilla, vec![mod_root.clone()]).expect("resolver");
        let table = seasons_table_from_resolver(&resolver).expect("parse modded seasons");

        assert_eq!(
            table.winter.date_range.start,
            SeasonDate { month: 11, day: 1 }
        );
        assert_eq!(resolver.stats().overridden_file_count, 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn seasons_from_resolver_honors_replace_path_exclusion() {
        let root = temp_test_dir("seasons_resolver_replace_path");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, SEASONS_SOURCE, VANILLA_SEASONS_TXT);
        write_text(
            &mod_root,
            "descriptor.mod",
            "name=\"ReplaceMap\"\nreplace_path=\"map\"\n",
        );

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let err = seasons_table_from_resolver(&resolver).expect_err("replace_path removes vanilla");

        assert!(err.to_string().contains("missing seasons.txt"), "{err}");
        assert_eq!(resolver.stats().replace_path_excluded_file_count, 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn seasons_vanilla_resolver_matches_direct_source_parse() {
        let root = temp_test_dir("seasons_resolver_vanilla_equivalence");
        let vanilla = root.join("vanilla");
        write_text(&vanilla, SEASONS_SOURCE, VANILLA_SEASONS_TXT);

        let resolver = GameFileResolver::vanilla(vanilla.clone()).expect("resolver");
        let via_resolver = seasons_table_from_resolver(&resolver).expect("parse via resolver");
        let direct = seasons_table_from_source(
            &fs::read_to_string(path_from_rel(&vanilla, SEASONS_SOURCE)).unwrap(),
        )
        .expect("parse direct");

        assert_eq!(via_resolver, direct);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn seasons_mod_parse_error_names_mod_file_without_vanilla_fallback() {
        let root = temp_test_dir("seasons_resolver_bad_mod");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, SEASONS_SOURCE, VANILLA_SEASONS_TXT);
        write_text(&mod_root, SEASONS_SOURCE, "winter = {}\n");

        let resolver =
            GameFileResolver::with_mod_paths(vanilla, vec![mod_root.clone()]).expect("resolver");
        let err = seasons_table_from_resolver(&resolver).expect_err("bad mod file fails");
        let message = err.to_string();

        assert!(
            message.contains(&mod_root.display().to_string()),
            "{message}"
        );
        assert!(message.contains(SEASONS_SOURCE), "{message}");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn winter_wraps_and_gaps_are_exposed_as_none() {
        let table = seasons_table_from_source(VANILLA_SEASONS_TXT).expect("parse vanilla seasons");
        assert_eq!(table.season_for_date(1, 15), Some(SeasonId::Winter));
        assert_eq!(table.season_for_date(12, 15), Some(SeasonId::Winter));
        assert_eq!(table.season_for_date(11, 20), None);
        assert_eq!(table.season_for_date(4, 30), None);
    }

    #[test]
    fn malformed_input_fails_closed() {
        let bad_date = VANILLA_SEASONS_TXT.replace("end_date=00.02.10", "end_date=bad");
        let err = seasons_table_from_source(&bad_date).expect_err("bad date must fail closed");
        assert!(
            err.to_string().contains("expected wildcard-year date"),
            "{err}"
        );

        let err = seasons_table_from_source("winter = {}").expect_err("missing blocks fail closed");
        assert!(err.to_string().contains("missing start_date"), "{err}");
    }

    #[test]
    fn tree_season_cb_law_maps_inside_each_block_to_file_order_columns() {
        let table = seasons_table_from_source(VANILLA_SEASONS_TXT).expect("parse vanilla seasons");
        for (month, day, expected_column) in [
            (11, 20, 0.0),
            (1, 1, 1.0),
            (2, 25, 2.0),
            (4, 1, 3.0),
            (5, 25, 4.0),
            (7, 1, 5.0),
            (10, 5, 6.0),
            (10, 30, 7.0),
        ] {
            let (lerp, column) =
                tree_season_cb_values(&table, month, day).expect("tree season cb values");
            assert_eq!(lerp, 0.0, "{month}.{day}");
            assert_eq!(column, expected_column, "{month}.{day}");
        }
    }

    #[test]
    fn tree_season_cb_law_ramps_across_winter_gap() {
        let table = seasons_table_from_source(VANILLA_SEASONS_TXT).expect("parse vanilla seasons");
        let (lerp, column) =
            tree_season_cb_values(&table, 12, 10).expect("winter gap tree season cb values");

        assert_eq!(column, 0.0);
        assert!((lerp - 0.5).abs() < 0.05, "{lerp}");
    }

    #[test]
    fn tree_season_cb_law_ramps_across_year_wrap_gap() {
        let table = seasons_table_from_source(VANILLA_SEASONS_TXT).expect("parse vanilla seasons");
        let (lerp, column) =
            tree_season_cb_values(&table, 11, 8).expect("year wrap tree season cb values");

        assert_eq!(column, 7.0);
        assert!((lerp - 0.5).abs() < f32::EPSILON, "{lerp}");
    }

    #[test]
    fn tree_season_cb_law_fails_closed_on_invalid_tree_table() {
        let mut table =
            seasons_table_from_source(VANILLA_SEASONS_TXT).expect("parse vanilla seasons");
        table.tree_winter.start = SeasonDate { month: 2, day: 31 };

        assert_eq!(tree_season_cb_values(&table, 1, 1), None);
    }

    #[test]
    fn tree_season_cb_override_writes_only_lerp_and_raw_column() {
        let original = (0..80).map(|value| value as u8).collect::<Vec<_>>();
        let mut flag_off = original.clone();
        assert!(!apply_tree_season_cb_override(&mut flag_off, None));
        assert_eq!(flag_off, original);

        let mut flag_on = original.clone();
        assert!(apply_tree_season_cb_override(
            &mut flag_on,
            Some((0.25, 6.0))
        ));
        assert_eq!(
            f32::from_le_bytes(flag_on[64..68].try_into().expect("lerp bytes")),
            0.25
        );
        assert_eq!(
            f32::from_le_bytes(flag_on[68..72].try_into().expect("column bytes")),
            6.0
        );
        for index in 0..flag_on.len() {
            if (64..72).contains(&index) {
                continue;
            }
            assert_eq!(flag_on[index], original[index], "byte {index}");
        }
    }
}
