use content_reader_parser::fixed5::{fixed5_percent, fixed5_to_string};
use replay_data::{rows::ModifierBreakdownNode, rows::ModifierDefinitionRow, rows::check_tree};

pub const PERCENT_TIMES_HUNDRED: u32 = 1 << 0;
pub const POSITIVE_IS_GOOD: u32 = 1 << 1;
pub const NEUTRAL: u32 = 1 << 2;
pub const NO_PLUS: u32 = 1 << 3;
pub const BOOLEAN: u32 = 1 << 5;
pub const PERCENT: u32 = 1 << 7;

pub fn value_text(
    row: &ModifierDefinitionRow,
    value: i64,
    precision: i32,
    plus: bool,
    invert: bool,
    mut localise: impl FnMut(&[u8], bool) -> Result<String, String>,
) -> Result<String, String> {
    let flags = row.flags;
    let mut number = String::new();
    if flags & BOOLEAN == 0 {
        if value > 0 && (plus || flags & NO_PLUS == 0) {
            number.push('+');
        }
        let precision = precision.min(row.precision);
        number.push_str(&if flags & PERCENT_TIMES_HUNDRED != 0 {
            fixed5_percent(value, precision, true)
        } else if flags & PERCENT != 0 {
            fixed5_percent(value, precision, false)
        } else {
            fixed5_to_string(value, precision)
        });
    } else {
        number.push_str(&localise(if value == 0 { b"NO" } else { b"YES" }, true)?);
    }
    let colour = if flags & NEUTRAL != 0 {
        'Y'
    } else if (flags & POSITIVE_IS_GOOD != 0) != invert {
        match value.signum() {
            1 => 'G',
            -1 => 'R',
            _ => 'Y',
        }
    } else {
        match value.signum() {
            1 => 'R',
            -1 => 'G',
            _ => 'Y',
        }
    };
    let mut result = format!("\u{11}{colour}{number}\u{11}!");
    if !row.postfix_key.is_empty() {
        result.push(' ');
        result.push_str(&localise(&row.postfix_key, false)?);
    }
    Ok(result)
}

pub struct Breakdown<'a> {
    pub depth: i32,
    pub prefix: &'a str,
    pub suffix: &'a str,
}

pub fn breakdown(
    nodes: &[ModifierBreakdownNode],
    options: Breakdown<'_>,
    mut leaf: impl FnMut(&ModifierBreakdownNode) -> Result<String, String>,
) -> Result<String, String> {
    check_tree(nodes).map_err(|e| e.to_string())?;
    let mut ends = vec![0; nodes.len()];
    for at in (0..nodes.len()).rev() {
        let mut end = at + 1;
        for _ in 0..nodes[at].children {
            end = ends[end];
        }
        ends[at] = end;
    }
    let mut stack = vec![(0_usize, options.depth, 0_u32, 1_usize, String::new())];
    loop {
        let (at, depth, visited, next, text) = stack.last_mut().unwrap();
        let node = &nodes[*at];
        if *visited == 0 && (node.children == 0 || *depth == 0) && node.value != 0 {
            *text = leaf(node)?;
        }
        if *depth != 0 && *visited < node.children {
            let child = *next;
            *next = ends[child];
            *visited += 1;
            let depth = depth.wrapping_sub(1);
            stack.push((child, depth, 0, child + 1, String::new()));
            continue;
        }
        let (_, _, _, _, text) = stack.pop().unwrap();
        let top = stack.len() == 1;
        let Some(parent) = stack.last_mut() else {
            return Ok(text);
        };
        let text = text.trim_matches(['\n', '\t', '\r', ' ']);
        if !text.is_empty() {
            if top {
                parent.4.push_str(options.prefix);
            }
            parent.4.push_str(text);
            parent.4.push('\n');
            if top {
                parent.4.push_str(options.suffix);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_zero_and_nested_prefix_suffix_preserve_breakdown_order() {
        let node = |children, value, name: &[u8]| ModifierBreakdownNode {
            children,
            value,
            name: name.to_vec(),
        };
        let nodes = [
            node(2, 90, b"root"),
            node(2, 80, b"nested"),
            node(0, 0, b"zero"),
            node(0, 1, b" \x0cfirst\x0b \t"),
            node(0, 2, b"last"),
        ];
        let render = |depth| {
            breakdown(
                &nodes,
                Breakdown {
                    depth,
                    prefix: ">",
                    suffix: "!",
                },
                |node| Ok(String::from_utf8(node.name.clone()).unwrap()),
            )
            .unwrap()
        };
        assert_eq!(render(0), "root");
        assert_eq!(render(1), ">nested\n!>last\n!");
        assert_eq!(render(-1), ">\x0cfirst\x0b\n!>last\n!");
        assert!(
            breakdown(
                &nodes[..4],
                Breakdown {
                    depth: -1,
                    prefix: "",
                    suffix: ""
                },
                |_| Ok(String::new())
            )
            .is_err()
        );
    }
}
