use std::io;

use super::{MAGIC, invalid};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Ints(Vec<i32>),
    Floats(Vec<f32>),
    Strings(Vec<String>),
}

impl Value {
    pub fn ints(&self) -> Option<&[i32]> {
        match self {
            Self::Ints(values) => Some(values),
            _ => None,
        }
    }

    pub fn floats(&self) -> Option<&[f32]> {
        match self {
            Self::Floats(values) => Some(values),
            _ => None,
        }
    }

    pub fn strings(&self) -> Option<&[String]> {
        match self {
            Self::Strings(values) => Some(values),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub properties: Vec<(String, Value)>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn property(&self, name: &str) -> Option<&Value> {
        self.properties
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    pub(super) fn string(&self, name: &str) -> Option<&str> {
        self.property(name)?.strings()?.first().map(String::as_str)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| invalid("a PDX asset length overflows"))?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| self.truncated(count))?;
        self.offset = end;
        Ok(slice)
    }

    fn truncated(&self, wanted: usize) -> io::Error {
        invalid(format!(
            "a PDX asset ends after {} bytes while reading {wanted} more at offset {}",
            self.bytes.len(),
            self.offset
        ))
    }

    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> io::Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn count(&mut self, element_bytes: usize) -> io::Result<usize> {
        let count = self.u32()? as usize;
        let needed = count
            .checked_mul(element_bytes)
            .ok_or_else(|| invalid("a PDX asset element count overflows"))?;
        if needed > self.remaining() {
            return Err(invalid(format!(
                "a PDX asset declares {count} elements of {element_bytes} bytes at offset {}, \
                 past its {} byte end",
                self.offset,
                self.bytes.len()
            )));
        }
        Ok(count)
    }

    fn nul_terminated(&mut self) -> io::Result<String> {
        let start = self.offset;
        let end = self.bytes[start..]
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(|| {
                invalid(format!(
                    "a PDX asset node name at offset {start} has no end"
                ))
            })?;
        let name = latin1(&self.bytes[start..start + end]);
        self.offset = start + end + 1;
        Ok(name)
    }
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

pub fn parse(bytes: &[u8]) -> io::Result<Node> {
    if bytes.get(..4) != Some(&MAGIC) {
        return Err(invalid(
            "not a PDX binary asset: the file does not start with the magic \"@@b@\"",
        ));
    }
    let mut reader = Reader {
        bytes,
        offset: MAGIC.len(),
    };
    let mut stack = vec![Node::default()];
    while let Some(byte) = reader.peek() {
        match byte {
            b'[' => {
                let mut depth = 0usize;
                while reader.peek() == Some(b'[') {
                    reader.offset += 1;
                    depth += 1;
                }
                let name = reader.nul_terminated()?;
                if depth == 0 || depth > stack.len() {
                    return Err(invalid(format!(
                        "the PDX asset node \"{name}\" opens at depth {depth} under an open path \
                         of depth {}",
                        stack.len() - 1
                    )));
                }
                close_to(&mut stack, depth);
                stack.push(Node {
                    name,
                    ..Node::default()
                });
            }
            b'!' => {
                reader.offset += 1;
                let length = reader.byte()? as usize;
                let key = latin1(reader.take(length)?);
                let kind = reader.byte()?;
                let value = match kind {
                    b'i' => {
                        let count = reader.count(4)?;
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(reader.u32()? as i32);
                        }
                        Value::Ints(values)
                    }
                    b'f' => {
                        let count = reader.count(4)?;
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(f32::from_bits(reader.u32()?));
                        }
                        Value::Floats(values)
                    }
                    b's' => {
                        let count = reader.count(4)?;
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            let length = reader.count(1)?;
                            let bytes = reader.take(length)?;
                            values.push(latin1(content_reader_parser::parser::c_string(bytes)));
                        }
                        Value::Strings(values)
                    }
                    other => {
                        return Err(invalid(format!(
                            "the PDX asset property \"{key}\" has the unknown type byte {other:#04x}"
                        )));
                    }
                };
                if let Some(node) = stack.last_mut() {
                    node.properties.push((key, value));
                }
            }
            other => {
                return Err(invalid(format!(
                    "the byte {other:#04x} at offset {} starts no PDX asset token",
                    reader.offset
                )));
            }
        }
    }
    close_to(&mut stack, 1);
    stack
        .pop()
        .ok_or_else(|| invalid("the PDX asset reader lost its root node"))
}

fn close_to(stack: &mut Vec<Node>, depth: usize) {
    while stack.len() > depth {
        let Some(node) = stack.pop() else { return };
        match stack.last_mut() {
            Some(parent) => parent.children.push(node),
            None => {
                stack.push(node);
                return;
            }
        }
    }
}
