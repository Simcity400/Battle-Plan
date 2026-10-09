//! NScript::SScriptedKey presentation cases. The shared native reader owns
//! their grammar; consumers supply the recorded event scope for selection.

use crate::parser::{Item, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub key: String,
    /// None is the native null trigger, from a scalar or `key = yes`.
    pub trigger: Option<Vec<Item>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptedKey {
    pub cases: Vec<Case>,
}

impl ScriptedKey {
    /// 1413622A0/1413623A0 append cases; key strings use raw C-string bytes,
    /// and a false/invalid bool creates a conjunction, even if it is empty.
    pub fn append(&mut self, value: &Value) -> Result<(), String> {
        if let Some(body) = value.items() {
            for item in body {
                let bytes = crate::parser::c_string(item.key.as_deref().unwrap_or_default());
                let key = std::str::from_utf8(bytes)
                    .map_err(|why| format!("scripted key case: {why}"))?
                    .to_owned();
                let trigger =
                    if item.value.scalar().and_then(crate::parser::read_bool) == Some(true) {
                        None
                    } else {
                        Some(item.value.items().unwrap_or_default().to_vec())
                    };
                self.cases.push(Case { key, trigger });
            }
        } else {
            self.cases.push(Case {
                key: value.c_str().map_err(|why| why.to_string())?.to_owned(),
                trigger: None,
            });
        }
        Ok(())
    }

    /// 1413621E0: first null/true trigger wins. A single case is evaluated,
    /// and no match gives no key; an unknown trigger cannot skip to a fallback.
    pub fn select(
        &self,
        mut evaluate: impl FnMut(&[Item]) -> Result<bool, String>,
    ) -> Result<Option<&str>, String> {
        for case in &self.cases {
            if case.trigger.as_deref().map_or(Ok(true), &mut evaluate)? {
                return Ok(Some(&case.key));
            }
        }
        Ok(None)
    }
}
