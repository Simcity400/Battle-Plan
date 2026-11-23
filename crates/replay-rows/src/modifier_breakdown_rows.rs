use super::*;
use crate::value::value_struct;
pub use game_model::modifier_breakdown::Node as ModifierBreakdownNode;
value_struct!(ModifierBreakdownNode { children: u32, value: i64, name: Vec<u8> });
opaque_value!(Vec<ModifierBreakdownNode>);
opaque_value!(Option<Vec<ModifierBreakdownNode>>);
opaque_value!(Option<std::sync::Arc<Vec<ModifierBreakdownNode>>>);
opaque_value!(Vec<Vec<u8>>);

pub fn printed_tree(nodes: &[ModifierBreakdownNode]) -> io::Result<Vec<ModifierBreakdownNode>> {
    check_tree(nodes)?;
    let mut output = Vec::new();
    let mut cursor = 0;
    copy_printed(nodes, &mut cursor, true, &mut output)?;
    let root = &nodes[0];
    if root.children != 0 && output[0].children == 0 {
        output[0].children = 1;
        engine_allocation::reserve(&mut output, 1)?;
        output.push(ModifierBreakdownNode::default());
    }
    Ok(output)
}

fn copy_printed(
    nodes: &[ModifierBreakdownNode],
    cursor: &mut usize,
    root: bool,
    output: &mut Vec<ModifierBreakdownNode>,
) -> io::Result<bool> {
    let node = &nodes[*cursor];
    *cursor += 1;
    let start = output.len();
    engine_allocation::reserve(output, 1)?;
    output.push(ModifierBreakdownNode::default());
    let mut kept = 0_u32;
    for _ in 0..node.children {
        if copy_printed(nodes, cursor, false, output)? {
            kept += 1;
        }
    }
    if !root && (node.children != 0 || node.value == 0) && kept == 0 {
        output.truncate(start);
        return Ok(false);
    }
    let name = if node.children == 0 && node.value != 0 {
        engine_allocation::copy_vec(&node.name)?
    } else {
        Vec::new()
    };
    output[start] = ModifierBreakdownNode {
        children: kept,
        value: node.value,
        name,
    };
    Ok(true)
}

pub fn check_tree(nodes: &[ModifierBreakdownNode]) -> io::Result<()> {
    let mut pending = 1_usize;
    for node in nodes {
        if pending == 0 {
            return Err(invalid("modifier breakdown has extra roots"));
        }
        pending = (pending - 1)
            .checked_add(node.children as usize)
            .ok_or_else(|| invalid("modifier breakdown child count overflow"))?;
    }
    if pending != 0 {
        return Err(invalid("modifier breakdown is incomplete"));
    }
    Ok(())
}
