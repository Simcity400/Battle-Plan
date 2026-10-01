//! The combat windows' authored hierarchy. Positions remain in game UI pixels.
use installed_content::{
    gui,
    parser::{Item, Value, read_bool},
};
use std::io;

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub kind: String,
    pub items: Vec<Item>,
    pub children: Vec<Node>,
}

/// The last value under keyword `key`: the GUI readers dispatch by token id
/// (parser::Item::keyword), so a bare key folds and a quoted one is none.
pub fn value<'a>(items: &'a [Item], key: &str) -> Option<&'a Value> {
    items
        .iter()
        .rev()
        .find(|item| item.keyword_is(key))
        .map(|item| &item.value)
}
pub fn string<'a>(items: &'a [Item], key: &str) -> Option<&'a str> {
    value(items, key).and_then(Value::scalar_str)
}
/// A GUI boolean field as read_bool_field (1424ABCA0 -> 1424B06E0) reads it,
/// each assignment in order: a failed read keeps the earlier value, or `None`.
/// A block value is `Err`: the game reads its contents as the widget's own fields.
pub fn boolean(items: &[Item], key: &str) -> Result<Option<bool>, String> {
    let mut read = None;
    // parse_field dispatches by token id: a quoted key is no keyword.
    for item in items.iter().filter(|item| item.keyword_is(key)) {
        match &item.value {
            Value::Scalar(text) => {
                if let Some(value) = read_bool(text) {
                    read = Some(value);
                }
            }
            Value::Block(_) | Value::Tagged { .. } => {
                return Err(format!(
                    "{key} is a block where read_bool_field (1424ABCA0) takes one value, so the game reads its contents as the widget's own fields"
                ));
            }
        }
    }
    Ok(read)
}
pub fn number(items: &[Item], key: &str, default: f32) -> f32 {
    string(items, key)
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
fn length(value: gui::Length, parent: f32) -> f32 {
    match value {
        gui::Length::Pixels(n) => n as f32,
        gui::Length::Percent(n) => parent * n as f32 / 100.0,
    }
}
impl Node {
    pub fn new(kind: &str, items: &[Item]) -> Self {
        Self {
            name: string(items, "name").unwrap_or_default().into(),
            kind: kind.to_ascii_lowercase(),
            items: items.to_vec(),
            children: items
                .iter()
                .filter_map(|item| {
                    let children = item.value.items()?;
                    string(children, "name")?;
                    Some(Self::new(
                        std::str::from_utf8(item.key.as_deref()?).ok()?,
                        children,
                    ))
                })
                .collect(),
        }
    }
    pub fn child(&self, path: &str) -> Option<&Node> {
        let mut current = self;
        for part in path.split('/') {
            current = current.children.iter().find(|n| n.name == part)?;
        }
        Some(current)
    }
    pub fn size(&self, parent: [f32; 2]) -> io::Result<[f32; 2]> {
        Ok(match gui::size(&self.items)? {
            Some((x, y)) => [length(x, parent[0]), length(y, parent[1])],
            None => parent,
        })
    }
    pub fn origin(&self, parent: [f32; 4]) -> io::Result<[f32; 2]> {
        let (x, y) =
            gui::position(&self.items)?.unwrap_or((gui::Length::Pixels(0), gui::Length::Pixels(0)));
        let orientation = string(&self.items, "orientation")
            .unwrap_or("upper_left")
            .to_ascii_lowercase();
        let mut at = [
            parent[0] + length(x, parent[2]),
            parent[1] + length(y, parent[3]),
        ];
        if orientation.contains("right") {
            at[0] += parent[2];
        } else if orientation.contains("center") {
            at[0] += parent[2] * 0.5;
        }
        if orientation.contains("lower")
            || orientation.contains("bottom")
            || orientation.contains("down")
        {
            at[1] += parent[3];
        } else if orientation == "center" {
            at[1] += parent[3] * 0.5;
        }
        if string(&self.items, "origo").is_some_and(|s| s.eq_ignore_ascii_case("center")) {
            let size = self.size([parent[2], parent[3]])?;
            at[0] -= size[0] * 0.5;
            at[1] -= size[1] * 0.5;
        }
        Ok(at)
    }
    pub fn rect(&self, parent: [f32; 4]) -> io::Result<[f32; 4]> {
        let at = self.origin(parent)?;
        let size = self.size([parent[2], parent[3]])?;
        Ok([at[0], at[1], size[0], size[1]])
    }
    pub fn content_rect(&self, mut rect: [f32; 4]) -> [f32; 4] {
        if let Some(items) = value(&self.items, "margin").and_then(Value::items) {
            let left = number(items, "left", 0.0);
            let top = number(items, "top", 0.0);
            rect[0] += left;
            rect[1] += top;
            rect[2] -= left + number(items, "right", 0.0);
            rect[3] -= top + number(items, "bottom", 0.0);
        }
        rect
    }
    pub fn path_rect(&self, root: [f32; 4], path: &str) -> io::Result<[f32; 4]> {
        let mut rect = root;
        let mut node = self;
        for part in path.split('/') {
            rect = node.content_rect(rect);
            node = node
                .children
                .iter()
                .find(|n| n.name == part)
                .ok_or_else(|| super::invalid(format!("{} has no child {part}", node.name)))?;
            rect = node.rect(rect)?;
        }
        Ok(rect)
    }
}

pub fn contains(rect: [f32; 4], point: [f32; 2]) -> bool {
    point[0] >= rect[0]
        && point[1] >= rect[1]
        && point[0] < rect[0] + rect[2]
        && point[1] < rect[1] + rect[3]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1424B06E0 compares the text exactly: `YES` and `No` keep the earlier
    /// value; keys still fold; a block value is refused by name.
    #[test]
    fn boolean_values_are_exact_yes_or_no_read_in_order() {
        let document = installed_content::parser::parse(
            b"CenterPosition = yes centerPosition = YES horizontal = no clipping = YES               fixedsize = No tilingCenter = no TILINGCENTER = yes \"horizontal\" = yes               orientation = { x = 1 }",
        )
        .unwrap();
        let items = &document.items;
        assert_eq!(boolean(items, "centerPosition"), Ok(Some(true)));
        assert_eq!(boolean(items, "horizontal"), Ok(Some(false)));
        assert_eq!(boolean(items, "clipping"), Ok(None));
        assert_eq!(boolean(items, "fixedsize"), Ok(None));
        assert_eq!(boolean(items, "tilingCenter"), Ok(Some(true)));
        assert_eq!(boolean(items, "alwaysTransparent"), Ok(None));
        assert!(
            boolean(items, "orientation")
                .unwrap_err()
                .starts_with("orientation is a block")
        );
    }

    #[test]
    fn boolean_values_stop_at_the_first_nul() {
        let mut document = installed_content::parser::parse(b"centerPosition = no").unwrap();
        for (text, expected) in [(b"yes\0suffix".as_slice(), true), (b"no\0suffix", false)] {
            document.items[0].value = Value::Scalar(text.to_vec());
            assert_eq!(
                boolean(&document.items, "centerPosition"),
                Ok(Some(expected))
            );
        }
    }

    #[test]
    fn centered_window_and_scrolling_content_use_separate_origins() {
        let document = installed_content::parser::parse(br#"
            name = root position = { x = 0 y = 0 } size = { width = 708 height = 532 }
            orientation = center origo = center
            containerWindowType = { name = list position = { x = 10 y = 20 }
                size = { width = 200 height = 100 } margin = { top = 10 left = 4 }
                gridBoxType = { name = rows position = { x = 3 y = 0 } size = { width = 100% height = 100% } }
            }
        "#).unwrap();
        let root = Node::new("containerWindowType", &document.items);
        let rect = root.rect([0.0, 0.0, 2048.0, 1152.0]).unwrap();
        assert_eq!(rect, [670.0, 310.0, 708.0, 532.0]);
        assert_eq!(
            root.path_rect(rect, "list/rows").unwrap(),
            [687.0, 340.0, 196.0, 90.0]
        );
    }
}
