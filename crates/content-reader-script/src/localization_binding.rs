#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormattedLocalization {
    Key(Vec<u8>),
    Formatter { formatter: Vec<u8>, token: Vec<u8> },
    Token(Vec<u8>),
}

impl Default for FormattedLocalization {
    fn default() -> Self {
        Self::Key(Vec::new())
    }
}

impl FormattedLocalization {
    pub fn parse(text: &[u8]) -> Self {
        if let Some(at) = text.windows(6).position(|window| window == b"token:") {
            return Self::Token(text.get(at + 6..).unwrap_or_default().to_vec());
        }
        match text.iter().position(|byte| *byte == b'|') {
            Some(bar) => Self::Formatter {
                formatter: text[..bar].to_vec(),
                token: text[bar + 1..].to_vec(),
            },
            None => Self::Key(text.to_vec()),
        }
    }

    pub fn is_empty_key(&self) -> bool {
        matches!(self, Self::Key(key) if key.is_empty())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoundNode {
    pub key: FormattedLocalization,
    pub arguments: Vec<BoundArgument>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundArgument {
    pub name: Vec<u8>,
    pub value: BoundValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundValue {
    Localization(FormattedLocalization),
    Node(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoundLocalization {
    pub nodes: Vec<BoundNode>,
}

impl BoundLocalization {
    pub fn from_value(value: &content_reader_parser::parser::Value) -> Self {
        fn formatted(value: &content_reader_parser::parser::Value) -> FormattedLocalization {
            FormattedLocalization::parse(content_reader_parser::parser::c_string(
                value.read_string().unwrap_or(b"{"),
            ))
        }
        fn node(
            value: &content_reader_parser::parser::Value,
            result: &mut BoundLocalization,
        ) -> usize {
            let at = result.nodes.len();
            result.nodes.push(BoundNode::default());
            if let Some(fields) = value.items() {
                for field in fields {
                    if field.keyword_is("localization_key") {
                        result.nodes[at].key = formatted(&field.value);
                    } else {
                        let value = if field.value.items().is_some() {
                            BoundValue::Node(node(&field.value, result))
                        } else {
                            BoundValue::Localization(formatted(&field.value))
                        };
                        result.nodes[at].arguments.push(BoundArgument {
                            name: content_reader_parser::parser::c_string(
                                field.key.as_deref().unwrap_or_default(),
                            )
                            .to_vec(),
                            value,
                        });
                    }
                }
            } else {
                result.nodes[at].key = formatted(value);
            }
            at
        }
        let mut result = Self::default();
        node(value, &mut result);
        if value.items().is_some() && result.nodes[0].key.is_empty_key() {
            result.nodes.clear();
        }
        result
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
