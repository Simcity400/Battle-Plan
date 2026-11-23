use std::io;

use content_reader_layers::ResolvedLayers;
use content_reader_parser::{parser::c_string, tokens::Token};
use content_reader_script::native::{self, Field, Shape};

pub const GROUPS_DIRECTORY: &str = "common/equipment_groups/";

#[derive(Clone, Copy, Debug)]
enum GroupNode {
    Top,
    Group,
}

fn group_shape(node: GroupNode, field: &Field<'_>, declared: &mut Vec<Token>) -> Shape<GroupNode> {
    match node {
        GroupNode::Top => {
            let token = Token::c_str(&field.key.text);
            if declared.contains(&token) {
                Shape::Skip
            } else {
                declared.push(token);
                Shape::Object(GroupNode::Group)
            }
        }
        GroupNode::Group => match field.keyword().as_deref() {
            Some(b"icon" | b"description") => Shape::One,
            Some(b"equipment_type") => Shape::List,
            _ => Shape::Skip,
        },
    }
}

#[derive(Clone, Debug)]
pub struct Group {
    token: Token,
    pub name: Vec<u8>,
    pub description: Vec<u8>,
    pub members: Vec<Vec<u8>>,
}

impl Group {
    pub fn key(&self) -> &[u8] {
        if self.description.is_empty() {
            &self.name
        } else {
            &self.description
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Groups {
    groups: Vec<Group>,
    pub misreads: Vec<String>,
    pub lost: Option<String>,
}

impl Groups {
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        layers.recorded_product("equipment groups", 1, &[], Self::read).map(|groups| groups.as_ref().clone())
    }

    fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut groups = Self::default();
        for path in content_reader_layers::script_files::paths(layers, GROUPS_DIRECTORY, Some(".txt"))? {
            groups.read_file(&path.read(layers)?, &path.text());
            if groups.lost.is_some() {
                break;
            }
        }
        Ok(groups)
    }

    fn read_file(&mut self, bytes: &[u8], path: &str) {
        let mut declared: Vec<Token> = self
            .groups
            .iter()
            .map(|group| group.token.clone())
            .collect();
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(GroupNode::Top),
            None,
            &mut |node, field| group_shape(node, field, &mut declared),
        );
        self.misreads.extend(
            read.misreads
                .iter()
                .map(|misread| format!("{path} {misread}")),
        );
        if let Some(unsettled) = &read.unsettled {
            self.lost = Some(format!("{path} {unsettled}"));
            return;
        }
        for item in &read.items {
            let name = c_string(item.key.as_deref().unwrap_or_default());
            let token = Token::register(name);
            if self.groups.iter().any(|group| group.token == token) {
                self.misreads.push(format!(
                    "{path}: equipment group {} is declared again, which the group reader reports and skips, keeping the first",
                    String::from_utf8_lossy(name)
                ));
                continue;
            }
            let fields = item.value.items().unwrap_or_default();
            let members = fields
                .iter()
                .filter(|field| field.keyword_is("equipment_type"))
                .flat_map(|field| field.value.items().unwrap_or_default())
                .map(|member| member.value.scalar().unwrap_or_default().to_vec())
                .collect();
            let description = fields
                .iter()
                .rev()
                .find(|field| field.keyword_is("description"))
                .map(|field| field.value.read_string().unwrap_or(b"{").to_vec())
                .unwrap_or_default();
            self.groups.push(Group {
                token,
                name: name.to_vec(),
                description,
                members,
            });
        }
    }

    pub fn expand(&self, names: &[impl AsRef<[u8]>]) -> Result<Vec<Vec<u8>>, String> {
        let mut out = Vec::new();
        self.expand_into(names, &mut out)?;
        Ok(out)
    }

    pub fn expand_inclusive(&self, names: &[impl AsRef<[u8]>]) -> Result<Vec<Vec<u8>>, String> {
        let names: Vec<Vec<u8>> = names.iter().map(|name| name.as_ref().to_vec()).collect();
        let mut out = Vec::new();
        self.collect(&names, &mut out, &mut Vec::new(), true)?;
        Ok(out)
    }

    pub fn expand_into(
        &self,
        names: &[impl AsRef<[u8]>],
        out: &mut Vec<Vec<u8>>,
    ) -> Result<(), String> {
        let names: Vec<Vec<u8>> = names.iter().map(|name| name.as_ref().to_vec()).collect();
        self.collect(&names, out, &mut Vec::new(), false)
    }

    fn collect(
        &self,
        members: &[Vec<u8>],
        out: &mut Vec<Vec<u8>>,
        within: &mut Vec<usize>,
        named: bool,
    ) -> Result<(), String> {
        let keep = |member: &Vec<u8>, out: &mut Vec<Vec<u8>>| {
            if !out.iter().any(|seen| seen.eq_ignore_ascii_case(member)) {
                out.push(member.clone());
            }
        };
        for member in members {
            match self.position(member)? {
                Some(at) if within.contains(&at) => {
                    let names: Vec<_> = within
                        .iter()
                        .chain(std::iter::once(&at))
                        .map(|&group| String::from_utf8_lossy(&self.groups[group].name))
                        .collect();
                    return Err(format!(
                        "equipment groups {} contain each other; group expansion recurses into them with no guard, so the game would not return",
                        names.join(" -> ")
                    ));
                }
                Some(at) => {
                    if named {
                        keep(member, out);
                    }
                    within.push(at);
                    self.collect(&self.groups[at].members, out, within, named)?;
                    within.pop();
                }
                None => keep(member, out),
            }
        }
        Ok(())
    }

    pub fn group(&self, member: &[u8]) -> Result<Option<&Group>, String> {
        Ok(self.position(member)?.map(|at| &self.groups[at]))
    }

    fn position(&self, member: &[u8]) -> Result<Option<usize>, String> {
        let token = Token::register(c_string(member));
        if let Some(at) = self.groups.iter().position(|group| group.token == token) {
            return Ok(Some(at));
        }
        match self.groups.iter().find(|group| group.token.is_rejected()) {
            None => Ok(None),
            Some(group) => Err(format!(
                "equipment group {} has a name that is empty or starts with a digit or `-`, token 0; member {} names it when the lexer holds no token for that text, which depends on every name any reader registered",
                String::from_utf8_lossy(&group.name),
                String::from_utf8_lossy(member)
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inclusive_expansion_keeps_group_names() {
        content_reader_script::test_game::enter();
        let mut groups = Groups::default();
        groups.read_file(
            b"outer = { equipment_type = { inner plain_type } }
              inner = { equipment_type = { PLAIN_TYPE other_type } }
              loop_a = { equipment_type = { loop_b } }
              loop_b = { equipment_type = { loop_a } }",
            "common/equipment_groups/g.txt",
        );
        assert!(groups.lost.is_none(), "{:?}", groups.lost);
        assert_eq!(
            groups.expand_inclusive(&["Outer", "loose"]).unwrap(),
            [
                b"Outer".to_vec(),
                b"inner".to_vec(),
                b"PLAIN_TYPE".to_vec(),
                b"other_type".to_vec(),
                b"loose".to_vec(),
            ]
        );
        assert_eq!(
            groups.expand(&["Outer"]).unwrap(),
            [b"PLAIN_TYPE".to_vec(), b"other_type".to_vec()]
        );
        assert!(groups.expand_inclusive(&["loop_a"]).is_err());
    }
}
