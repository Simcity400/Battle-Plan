//! Generic, mod-extensible scripted-GUI evaluator.
//!
//! Definitions are discovered from the effective UI14/UI15 VFS. The evaluator
//! consumes replay_core's typed d51 variable/flag view first. Predicates and
//! effects outside that subset fail closed and may be satisfied only by exact
//! d84 engine-resolved inputs carried in the recording.

use crate::assets::AssetResolver;
use crate::paradox_text::{parse_file, Entry, Value};
use crate::{GameUiError, Result};
use replay_core::{Leaf, ScriptedGuiRuntimeInputs, ScriptedGuiView};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Truth {
    True,
    False,
    Unavailable,
}

impl Truth {
    fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unavailable => Self::Unavailable,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableScope {
    Country,
    World,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableRef {
    pub scope: VariableScope,
    pub name: String,
}

impl VariableRef {
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim().trim_start_matches('?').trim_end_matches('?');
        let lower = raw.to_ascii_lowercase();
        for prefix in ["global.", "world."] {
            if lower.starts_with(prefix) {
                return Self {
                    scope: VariableScope::World,
                    name: raw[prefix.len()..].to_string(),
                };
            }
        }
        for prefix in ["root.", "this.", "country."] {
            if lower.starts_with(prefix) {
                return Self {
                    scope: VariableScope::Country,
                    name: raw[prefix.len()..].to_string(),
                };
            }
        }
        Self {
            scope: VariableScope::Country,
            name: raw.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    Equal,
    NotEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operand {
    Number { value: f64 },
    Variable { variable: VariableRef },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Predicate {
    Constant {
        value: bool,
    },
    Compare {
        variable: VariableRef,
        op: CompareOp,
        operand: Operand,
    },
    HasVariable {
        variable: VariableRef,
    },
    HasFlag {
        scope: VariableScope,
        name: String,
    },
    All {
        predicates: Vec<Predicate>,
    },
    Any {
        predicates: Vec<Predicate>,
    },
    Not {
        predicate: Box<Predicate>,
    },
    Unsupported {
        construct: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EffectOp {
    Set {
        variable: VariableRef,
        operand: Operand,
    },
    Add {
        variable: VariableRef,
        operand: Operand,
    },
    Subtract {
        variable: VariableRef,
        operand: Operand,
    },
    Multiply {
        variable: VariableRef,
        operand: Operand,
    },
    Divide {
        variable: VariableRef,
        operand: Operand,
    },
    Modulo {
        variable: VariableRef,
        operand: Operand,
    },
    Clear {
        variable: VariableRef,
    },
    SetFlag {
        scope: VariableScope,
        name: String,
    },
    ClearFlag {
        scope: VariableScope,
        name: String,
    },
    Conditional {
        predicate: Predicate,
        then_ops: Vec<EffectOp>,
        else_ops: Vec<EffectOp>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectProgram {
    pub operations: Vec<EffectOp>,
    pub unsupported_constructs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PropertyExpr {
    Literal { value: String },
    Variable { variable: VariableRef },
    EngineCall { call: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptedGuiDefinition {
    pub name: String,
    pub context_type: Option<String>,
    pub window_name: Option<String>,
    pub parent_window_token: Option<String>,
    pub source_file: String,
    pub visible: Predicate,
    pub enabled: Predicate,
    pub triggers: BTreeMap<String, Predicate>,
    pub effects: BTreeMap<String, EffectProgram>,
    /// Flattened `<element>.<property>` keys.
    pub properties: BTreeMap<String, PropertyExpr>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptedGuiRegistry {
    pub schema: String,
    pub files: usize,
    pub definitions: BTreeMap<String, ScriptedGuiDefinition>,
    pub source_issues: Vec<String>,
}

impl ScriptedGuiRegistry {
    pub fn production_admitted(&self) -> bool {
        self.source_issues.is_empty()
    }
}

pub fn discover_effective_scripted_guis(
    game_root: &std::path::Path,
    userdata: &std::path::Path,
) -> Result<ScriptedGuiRegistry> {
    let resolver = AssetResolver::from_runtime_userdir(game_root, userdata)?;
    load_scripted_gui_registry(&resolver)
}

pub fn load_scripted_gui_registry(resolver: &AssetResolver) -> Result<ScriptedGuiRegistry> {
    let files = resolver.list_files_recursive("common/scripted_guis", Some("txt"))?;
    let mut definitions = BTreeMap::new();
    let mut source_issues = Vec::new();
    for file in &files {
        let document = parse_file(&file.path)?;
        source_issues.extend(document.recoveries.iter().map(|recovery| {
            format!(
                "{}:{}:{}:{}",
                file.rel_path, recovery.line, recovery.column, recovery.message
            )
        }));
        for entry in document.root.fields() {
            let Some(key) = entry.key.as_deref() else {
                source_issues.push(format!("{}:anonymous top-level construct", file.rel_path));
                continue;
            };
            if key.starts_with('@') {
                continue;
            }
            if !key.eq_ignore_ascii_case("scripted_gui") {
                source_issues.push(format!(
                    "{}:unsupported top-level construct {key}",
                    file.rel_path
                ));
                continue;
            }
            let Some(rows) = entry.value.as_object() else {
                source_issues.push(format!("{}:scripted_gui is not an object", file.rel_path));
                continue;
            };
            for row in rows {
                let Some(name) = row.key.as_deref() else {
                    source_issues.push(format!(
                        "{}:anonymous scripted_gui definition",
                        file.rel_path
                    ));
                    continue;
                };
                let Some(body) = row.value.as_object() else {
                    source_issues.push(format!("{}:{name} is not an object", file.rel_path));
                    continue;
                };
                let definition = compile_definition(name, body, &file.rel_path, &mut source_issues);
                if definitions.insert(name.to_string(), definition).is_some() {
                    source_issues.push(format!(
                        "{}:duplicate effective scripted_gui id {name}",
                        file.rel_path
                    ));
                }
            }
        }
    }
    Ok(ScriptedGuiRegistry {
        schema: "hoi4-xui22-scripted-gui-registry-v1".to_string(),
        files: files.len(),
        definitions,
        source_issues,
    })
}

fn compile_definition(
    name: &str,
    body: &[Entry],
    source_file: &str,
    source_issues: &mut Vec<String>,
) -> ScriptedGuiDefinition {
    let value = Value::Object(body.to_vec());
    let visible = value
        .first_value_ci("visible")
        .map(|value| compile_predicate(value, &format!("{name}.visible")))
        .unwrap_or(Predicate::Constant { value: true });
    let enabled = value
        .first_value_ci("enabled")
        .map(|value| compile_predicate(value, &format!("{name}.enabled")))
        .unwrap_or(Predicate::Constant { value: true });
    let mut triggers = BTreeMap::new();
    if let Some(value) = value.first_value_ci("triggers") {
        if let Some(block) = value.as_object() {
            for row in block {
                if let Some(trigger) = row.key.as_deref() {
                    triggers.insert(
                        trigger.to_string(),
                        compile_predicate(&row.value, &format!("{name}.triggers.{trigger}")),
                    );
                } else {
                    source_issues.push(format!("{source_file}:{name}.triggers:anonymous trigger"));
                }
            }
        } else {
            source_issues.push(format!("{source_file}:{name}.triggers is not an object"));
        }
    }
    let mut effects = BTreeMap::new();
    if let Some(value) = value.first_value_ci("effects") {
        if let Some(block) = value.as_object() {
            for row in block {
                if let Some(effect) = row.key.as_deref() {
                    effects.insert(
                        effect.to_string(),
                        compile_effect_program(&row.value, &format!("{name}.effects.{effect}")),
                    );
                } else {
                    source_issues.push(format!("{source_file}:{name}.effects:anonymous effect"));
                }
            }
        } else {
            source_issues.push(format!("{source_file}:{name}.effects is not an object"));
        }
    }
    let mut properties = BTreeMap::new();
    if let Some(value) = value.first_value_ci("properties") {
        if let Some(block) = value.as_object() {
            for element in block {
                let Some(element_name) = element.key.as_deref() else {
                    source_issues
                        .push(format!("{source_file}:{name}.properties:anonymous element"));
                    continue;
                };
                let Some(fields) = element.value.as_object() else {
                    source_issues.push(format!(
                        "{source_file}:{name}.properties.{element_name} is not an object"
                    ));
                    continue;
                };
                for property in fields {
                    let Some(property_name) = property.key.as_deref() else {
                        source_issues.push(format!(
                            "{source_file}:{name}.properties.{element_name}:anonymous property"
                        ));
                        continue;
                    };
                    let Some(raw) = property.value.as_scalar() else {
                        source_issues.push(format!(
                            "{source_file}:{name}.properties.{element_name}.{property_name} is not scalar"
                        ));
                        continue;
                    };
                    properties.insert(
                        format!("{element_name}.{property_name}"),
                        compile_property(raw),
                    );
                }
            }
        } else {
            source_issues.push(format!("{source_file}:{name}.properties is not an object"));
        }
    }
    ScriptedGuiDefinition {
        name: name.to_string(),
        context_type: value.first_scalar_ci("context_type").map(str::to_string),
        window_name: value.first_scalar_ci("window_name").map(str::to_string),
        parent_window_token: value
            .first_scalar_ci("parent_window_token")
            .map(str::to_string),
        source_file: source_file.to_string(),
        visible,
        enabled,
        triggers,
        effects,
        properties,
    }
}

fn compile_property(raw: &str) -> PropertyExpr {
    let trimmed = raw.trim();
    if let Some(inner) = trimmed
        .strip_prefix("[?")
        .and_then(|v| v.strip_suffix("?]"))
    {
        return PropertyExpr::Variable {
            variable: VariableRef::parse(inner),
        };
    }
    if let Some(inner) = trimmed.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
        return PropertyExpr::EngineCall {
            call: inner.to_string(),
        };
    }
    if trimmed.contains('[') {
        return PropertyExpr::EngineCall {
            call: trimmed.to_string(),
        };
    }
    PropertyExpr::Literal {
        value: raw.to_string(),
    }
}

fn compile_predicate(value: &Value, path: &str) -> Predicate {
    if let Some(raw) = value.as_scalar() {
        return match raw.to_ascii_lowercase().as_str() {
            "yes" | "true" => Predicate::Constant { value: true },
            "no" | "false" => Predicate::Constant { value: false },
            _ => Predicate::Unsupported {
                construct: format!("{path}:scalar:{raw}"),
            },
        };
    }
    let Some(entries) = value.as_object() else {
        return Predicate::Unsupported {
            construct: format!("{path}:non_object"),
        };
    };
    let mut predicates = Vec::new();
    for entry in entries {
        let Some(key) = entry.key.as_deref() else {
            predicates.push(Predicate::Unsupported {
                construct: format!("{path}:anonymous"),
            });
            continue;
        };
        let lower = key.to_ascii_lowercase();
        let predicate = match lower.as_str() {
            "always" => compile_predicate(&entry.value, &format!("{path}.always")),
            "and" => compile_predicate(&entry.value, &format!("{path}.AND")),
            "or" => Predicate::Any {
                predicates: predicate_children(&entry.value, &format!("{path}.OR")),
            },
            "not" => Predicate::Not {
                predicate: Box::new(compile_predicate(&entry.value, &format!("{path}.NOT"))),
            },
            "check_variable" | "is_variable_equal" => parse_comparison(&entry.value, path)
                .unwrap_or_else(|| Predicate::Unsupported {
                    construct: format!("{path}.{key}"),
                }),
            "has_variable" | "is_variable_set" => entry
                .value
                .as_scalar()
                .map(|raw| Predicate::HasVariable {
                    variable: VariableRef::parse(raw),
                })
                .unwrap_or_else(|| Predicate::Unsupported {
                    construct: format!("{path}.{key}"),
                }),
            "has_country_flag" => scalar_flag(&entry.value, VariableScope::Country, path, key),
            "has_global_flag" => scalar_flag(&entry.value, VariableScope::World, path, key),
            "root" | "this" => compile_predicate(&entry.value, &format!("{path}.{key}")),
            _ => Predicate::Unsupported {
                construct: format!("{path}.{key}"),
            },
        };
        predicates.push(predicate);
    }
    match predicates.len() {
        0 => Predicate::Constant { value: true },
        1 => predicates.pop().unwrap(),
        _ => Predicate::All { predicates },
    }
}

fn predicate_children(value: &Value, path: &str) -> Vec<Predicate> {
    value
        .as_object()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    let wrapper = Value::Object(vec![entry.clone()]);
                    compile_predicate(&wrapper, path)
                })
                .collect()
        })
        .unwrap_or_else(|| vec![compile_predicate(value, path)])
}

fn scalar_flag(value: &Value, scope: VariableScope, path: &str, key: &str) -> Predicate {
    value
        .as_scalar()
        .map(|name| Predicate::HasFlag {
            scope,
            name: name.to_string(),
        })
        .unwrap_or_else(|| Predicate::Unsupported {
            construct: format!("{path}.{key}"),
        })
}

fn parse_compare_op(raw: &str) -> Option<CompareOp> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "=" | "==" | "equal" | "equals" => Some(CompareOp::Equal),
        "!=" | "not_equal" => Some(CompareOp::NotEqual),
        ">" | "greater_than" => Some(CompareOp::Greater),
        ">=" | "greater_than_or_equals" | "greater_or_equal" => Some(CompareOp::GreaterEqual),
        "<" | "less_than" => Some(CompareOp::Less),
        "<=" | "less_than_or_equals" | "less_or_equal" => Some(CompareOp::LessEqual),
        _ => None,
    }
}

fn parse_operand(raw: &str) -> Option<Operand> {
    raw.parse::<f64>()
        .ok()
        .map(|value| Operand::Number { value })
        .or_else(|| {
            let token = raw.strip_prefix("var:")?;
            Some(Operand::Variable {
                variable: VariableRef::parse(token),
            })
        })
}

fn parse_comparison(value: &Value, path: &str) -> Option<Predicate> {
    let entries = value.as_object()?;
    let object = Value::Object(entries.to_vec());
    if let (Some(var), Some(rhs)) = (
        object.first_scalar_ci("var"),
        object.first_scalar_ci("value"),
    ) {
        let op = object
            .first_scalar_ci("compare")
            .and_then(parse_compare_op)
            .unwrap_or(CompareOp::Equal);
        return Some(Predicate::Compare {
            variable: VariableRef::parse(var),
            op,
            operand: parse_operand(rhs)?,
        });
    }
    for entry in entries {
        if let (Some(var), Some(rhs)) = (entry.key.as_deref(), entry.value.as_scalar()) {
            if !matches!(
                var.to_ascii_lowercase().as_str(),
                "var" | "value" | "compare"
            ) {
                return Some(Predicate::Compare {
                    variable: VariableRef::parse(var),
                    op: CompareOp::Equal,
                    operand: parse_operand(rhs)?,
                });
            }
        }
    }
    let anonymous = entries
        .iter()
        .filter(|entry| entry.key.is_none())
        .filter_map(|entry| entry.value.as_scalar())
        .collect::<Vec<_>>();
    if anonymous.len() >= 3 {
        return Some(Predicate::Compare {
            variable: VariableRef::parse(anonymous[0]),
            op: parse_compare_op(anonymous[1])?,
            operand: parse_operand(anonymous[2])?,
        });
    }
    if anonymous.len() == 1 {
        if let Some(next) = entries.iter().find(|entry| {
            entry
                .key
                .as_deref()
                .is_some_and(|key| matches!(key, ">" | "<" | "!" | "="))
        }) {
            let glyph = next.key.as_deref()?;
            let rhs = next.value.as_scalar()?;
            return Some(Predicate::Compare {
                variable: VariableRef::parse(anonymous[0]),
                op: parse_compare_op(&format!("{glyph}="))?,
                operand: parse_operand(rhs)?,
            });
        }
    }
    let _ = path;
    None
}

fn compile_effect_program(value: &Value, path: &str) -> EffectProgram {
    let mut operations = Vec::new();
    let mut unsupported_constructs = Vec::new();
    compile_effect_entries(value, path, &mut operations, &mut unsupported_constructs);
    EffectProgram {
        operations,
        unsupported_constructs,
    }
}

fn compile_effect_entries(
    value: &Value,
    path: &str,
    operations: &mut Vec<EffectOp>,
    unsupported: &mut Vec<String>,
) {
    let Some(entries) = value.as_object() else {
        unsupported.push(format!("{path}:non_object"));
        return;
    };
    for entry in entries {
        let Some(key) = entry.key.as_deref() else {
            unsupported.push(format!("{path}:anonymous"));
            continue;
        };
        let lower = key.to_ascii_lowercase();
        let op = match lower.as_str() {
            "set_variable" | "set_temp_variable" => parse_variable_effect(&entry.value, 0),
            "add_to_variable" | "add_to_temp_variable" => parse_variable_effect(&entry.value, 1),
            "subtract_from_variable" | "subtract_from_temp_variable" => {
                parse_variable_effect(&entry.value, 2)
            }
            "multiply_variable" | "multiply_temp_variable" => {
                parse_variable_effect(&entry.value, 3)
            }
            "divide_variable" | "divide_temp_variable" => parse_variable_effect(&entry.value, 4),
            "modulo_variable" => parse_variable_effect(&entry.value, 5),
            "clear_variable" | "clear_temp_variable" => parse_clear_effect(&entry.value),
            "set_country_flag" => parse_flag_effect(&entry.value, VariableScope::Country, true),
            "clr_country_flag" | "clear_country_flag" => {
                parse_flag_effect(&entry.value, VariableScope::Country, false)
            }
            "set_global_flag" => parse_flag_effect(&entry.value, VariableScope::World, true),
            "clr_global_flag" | "clear_global_flag" => {
                parse_flag_effect(&entry.value, VariableScope::World, false)
            }
            "root" | "this" => {
                compile_effect_entries(
                    &entry.value,
                    &format!("{path}.{key}"),
                    operations,
                    unsupported,
                );
                continue;
            }
            "if" => parse_conditional_effect(&entry.value, &format!("{path}.if")),
            _ => None,
        };
        match op {
            Some(op) => operations.push(op),
            None => unsupported.push(format!("{path}.{key}")),
        }
    }
}

fn variable_and_operand(value: &Value) -> Option<(VariableRef, Operand)> {
    let entries = value.as_object()?;
    let object = Value::Object(entries.to_vec());
    if let (Some(var), Some(rhs)) = (
        object.first_scalar_ci("var"),
        object.first_scalar_ci("value"),
    ) {
        return Some((VariableRef::parse(var), parse_operand(rhs)?));
    }
    for entry in entries {
        if let (Some(var), Some(rhs)) = (entry.key.as_deref(), entry.value.as_scalar()) {
            if !matches!(
                var.to_ascii_lowercase().as_str(),
                "var" | "value" | "tooltip"
            ) {
                return Some((VariableRef::parse(var), parse_operand(rhs)?));
            }
        }
    }
    None
}

fn parse_variable_effect(value: &Value, kind: u8) -> Option<EffectOp> {
    let (variable, operand) = variable_and_operand(value)?;
    Some(match kind {
        0 => EffectOp::Set { variable, operand },
        1 => EffectOp::Add { variable, operand },
        2 => EffectOp::Subtract { variable, operand },
        3 => EffectOp::Multiply { variable, operand },
        4 => EffectOp::Divide { variable, operand },
        _ => EffectOp::Modulo { variable, operand },
    })
}

fn parse_clear_effect(value: &Value) -> Option<EffectOp> {
    let raw = value.as_scalar().or_else(|| value.first_scalar_ci("var"))?;
    Some(EffectOp::Clear {
        variable: VariableRef::parse(raw),
    })
}

fn parse_flag_effect(value: &Value, scope: VariableScope, set: bool) -> Option<EffectOp> {
    let raw = value
        .as_scalar()
        .or_else(|| value.first_scalar_ci("flag"))?;
    Some(if set {
        EffectOp::SetFlag {
            scope,
            name: raw.to_string(),
        }
    } else {
        EffectOp::ClearFlag {
            scope,
            name: raw.to_string(),
        }
    })
}

fn parse_conditional_effect(value: &Value, path: &str) -> Option<EffectOp> {
    let entries = value.as_object()?;
    let object = Value::Object(entries.to_vec());
    let limit = object.first_value_ci("limit")?;
    let predicate = compile_predicate(limit, &format!("{path}.limit"));
    let mut then_ops = Vec::new();
    let mut else_ops = Vec::new();
    let mut unsupported = Vec::new();
    let then_entries = entries
        .iter()
        .filter(|entry| !entry.key_matches("limit") && !entry.key_matches("else"))
        .cloned()
        .collect::<Vec<_>>();
    compile_effect_entries(
        &Value::Object(then_entries),
        path,
        &mut then_ops,
        &mut unsupported,
    );
    if let Some(value) = object.first_value_ci("else") {
        compile_effect_entries(
            value,
            &format!("{path}.else"),
            &mut else_ops,
            &mut unsupported,
        );
    }
    if unsupported.is_empty() {
        Some(EffectOp::Conditional {
            predicate,
            then_ops,
            else_ops,
        })
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptedGuiRuntimeState {
    pub country_tag: String,
    pub country_variables: Option<BTreeMap<String, f64>>,
    pub country_flags: Option<BTreeSet<String>>,
    pub world_variables: Option<BTreeMap<String, f64>>,
    pub world_flags: Option<BTreeSet<String>>,
}

impl ScriptedGuiRuntimeState {
    pub fn from_view(view: &ScriptedGuiView, country_tag: &str) -> Self {
        Self {
            country_tag: country_tag.to_string(),
            country_variables: measured_fixed5_or_projection(
                view.by_country_raw_fixed5.get(country_tag),
                view.by_country.get(country_tag),
            ),
            country_flags: measured_set(view.country_flags.get(country_tag)),
            world_variables: measured_fixed5_or_projection(
                Some(&view.world_variables_raw_fixed5),
                Some(&view.world_variables),
            ),
            world_flags: measured_leaf_set(&view.world_flags),
        }
    }

    fn variable(&self, variable: &VariableRef) -> Option<f64> {
        match variable.scope {
            VariableScope::Country => self
                .country_variables
                .as_ref()?
                .get(&variable.name)
                .copied(),
            VariableScope::World => self.world_variables.as_ref()?.get(&variable.name).copied(),
        }
    }

    fn variables_mut(&mut self, scope: VariableScope) -> Option<&mut BTreeMap<String, f64>> {
        match scope {
            VariableScope::Country => self.country_variables.as_mut(),
            VariableScope::World => self.world_variables.as_mut(),
        }
    }

    fn flags(&self, scope: VariableScope) -> Option<&BTreeSet<String>> {
        match scope {
            VariableScope::Country => self.country_flags.as_ref(),
            VariableScope::World => self.world_flags.as_ref(),
        }
    }

    fn flags_mut(&mut self, scope: VariableScope) -> Option<&mut BTreeSet<String>> {
        match scope {
            VariableScope::Country => self.country_flags.as_mut(),
            VariableScope::World => self.world_flags.as_mut(),
        }
    }
}

fn measured_map(leaf: Option<&Leaf<BTreeMap<String, f64>>>) -> Option<BTreeMap<String, f64>> {
    match leaf {
        Some(Leaf::Measured(value)) => Some(value.clone()),
        _ => None,
    }
}

fn measured_fixed5_or_projection(
    raw: Option<&Leaf<BTreeMap<String, i64>>>,
    projected: Option<&Leaf<BTreeMap<String, f64>>>,
) -> Option<BTreeMap<String, f64>> {
    match raw {
        Some(Leaf::Measured(values)) => {
            let Some(Leaf::Measured(projected)) = projected else {
                return None;
            };
            let projection_matches = values.len() == projected.len()
                && values.iter().all(|(key, value)| {
                    projected
                        .get(key)
                        .is_some_and(|projected| *projected == *value as f64 / 100_000.0)
                });
            projection_matches.then(|| {
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), *value as f64 / 100_000.0))
                    .collect()
            })
        }
        Some(Leaf::Unobtained) => None,
        Some(Leaf::Absent) | None => measured_map(projected),
    }
}

fn measured_set(leaf: Option<&Leaf<Vec<String>>>) -> Option<BTreeSet<String>> {
    match leaf {
        Some(Leaf::Measured(value)) => Some(value.iter().cloned().collect()),
        _ => None,
    }
}

fn measured_leaf_set(leaf: &Leaf<Vec<String>>) -> Option<BTreeSet<String>> {
    match leaf {
        Leaf::Measured(value) => Some(value.iter().cloned().collect()),
        _ => None,
    }
}

impl Predicate {
    pub fn evaluate(&self, state: &ScriptedGuiRuntimeState) -> Truth {
        match self {
            Self::Constant { value } => {
                if *value {
                    Truth::True
                } else {
                    Truth::False
                }
            }
            Self::Compare {
                variable,
                op,
                operand,
            } => {
                let Some(left) = state.variable(variable) else {
                    return Truth::Unavailable;
                };
                let Some(right) = operand_value(operand, state) else {
                    return Truth::Unavailable;
                };
                let result = match op {
                    CompareOp::Equal => left == right,
                    CompareOp::NotEqual => left != right,
                    CompareOp::Greater => left > right,
                    CompareOp::GreaterEqual => left >= right,
                    CompareOp::Less => left < right,
                    CompareOp::LessEqual => left <= right,
                };
                if result {
                    Truth::True
                } else {
                    Truth::False
                }
            }
            Self::HasVariable { variable } => match variable.scope {
                VariableScope::Country if state.country_variables.is_none() => Truth::Unavailable,
                VariableScope::World if state.world_variables.is_none() => Truth::Unavailable,
                _ if state.variable(variable).is_some() => Truth::True,
                _ => Truth::False,
            },
            Self::HasFlag { scope, name } => match state.flags(*scope) {
                Some(flags) if flags.contains(name) => Truth::True,
                Some(_) => Truth::False,
                None => Truth::Unavailable,
            },
            Self::All { predicates } => {
                let mut unavailable = false;
                for predicate in predicates {
                    match predicate.evaluate(state) {
                        Truth::False => return Truth::False,
                        Truth::Unavailable => unavailable = true,
                        Truth::True => {}
                    }
                }
                if unavailable {
                    Truth::Unavailable
                } else {
                    Truth::True
                }
            }
            Self::Any { predicates } => {
                let mut unavailable = false;
                for predicate in predicates {
                    match predicate.evaluate(state) {
                        Truth::True => return Truth::True,
                        Truth::Unavailable => unavailable = true,
                        Truth::False => {}
                    }
                }
                if unavailable {
                    Truth::Unavailable
                } else {
                    Truth::False
                }
            }
            Self::Not { predicate } => predicate.evaluate(state).not(),
            Self::Unsupported { .. } => Truth::Unavailable,
        }
    }
}

fn operand_value(operand: &Operand, state: &ScriptedGuiRuntimeState) -> Option<f64> {
    match operand {
        Operand::Number { value } => Some(*value),
        Operand::Variable { variable } => state.variable(variable),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvaluatedProperty {
    String { value: String, captured: bool },
    Number { value: f64 },
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptedGuiEvaluation {
    pub visible: Truth,
    pub enabled: Truth,
    pub triggers: BTreeMap<String, Truth>,
    pub properties: BTreeMap<String, EvaluatedProperty>,
    pub effect_enabled: BTreeMap<String, Truth>,
}

pub fn evaluate_definition(
    definition: &ScriptedGuiDefinition,
    state: &ScriptedGuiRuntimeState,
    captured: Option<&ScriptedGuiRuntimeInputs>,
) -> ScriptedGuiEvaluation {
    let visible = captured_truth(
        definition.visible.evaluate(state),
        captured.map(|v| &v.visible.value),
    );
    let enabled = captured_truth(
        definition.enabled.evaluate(state),
        captured.map(|v| &v.enabled.value),
    );
    let triggers: BTreeMap<String, Truth> = definition
        .triggers
        .iter()
        .map(|(name, predicate)| {
            let truth = predicate.evaluate(state);
            let truth = if truth == Truth::Unavailable {
                captured
                    .and_then(|value| value.triggers.get(name))
                    .map_or(Truth::Unavailable, |value| {
                        captured_truth(Truth::Unavailable, Some(&value.value))
                    })
            } else {
                truth
            };
            (name.clone(), truth)
        })
        .collect();
    let properties = definition
        .properties
        .iter()
        .map(|(name, property)| {
            let value = match property {
                PropertyExpr::Literal { value } => EvaluatedProperty::String {
                    value: value.clone(),
                    captured: false,
                },
                PropertyExpr::Variable { variable } => state
                    .variable(variable)
                    .map(|value| EvaluatedProperty::Number { value })
                    .unwrap_or_else(|| captured_property(captured, name)),
                PropertyExpr::EngineCall { .. } => captured_property(captured, name),
            };
            (name.clone(), value)
        })
        .collect();
    let effect_enabled = definition
        .effects
        .keys()
        .map(|name| {
            let truth = effect_enabled_truth(definition, name, state, captured);
            (name.clone(), truth)
        })
        .collect();
    ScriptedGuiEvaluation {
        visible,
        enabled,
        triggers,
        properties,
        effect_enabled,
    }
}

fn effect_enabled_truth(
    definition: &ScriptedGuiDefinition,
    effect_name: &str,
    state: &ScriptedGuiRuntimeState,
    captured: Option<&ScriptedGuiRuntimeInputs>,
) -> Truth {
    let definition_enabled = captured_truth(
        definition.enabled.evaluate(state),
        captured.map(|value| &value.enabled.value),
    );
    let trigger_name = format!("{effect_name}_enabled");
    let Some(predicate) = definition.triggers.get(&trigger_name) else {
        return truth_and(definition_enabled, Truth::True);
    };
    let local = predicate.evaluate(state);
    let effect_gate = if local != Truth::Unavailable {
        local
    } else {
        captured
            .and_then(|value| value.effect_enabled.get(effect_name))
            .map_or(Truth::Unavailable, |value| {
                captured_truth(Truth::Unavailable, Some(&value.value))
            })
    };
    truth_and(definition_enabled, effect_gate)
}

fn captured_property(captured: Option<&ScriptedGuiRuntimeInputs>, name: &str) -> EvaluatedProperty {
    captured
        .and_then(|value| value.properties.get(name))
        .and_then(|value| match &value.value {
            Leaf::Measured(value) => Some(value),
            _ => None,
        })
        .map_or(EvaluatedProperty::Unavailable, |value| EvaluatedProperty::String {
            value: value.clone(),
            captured: true,
        })
}

fn truth_and(left: Truth, right: Truth) -> Truth {
    match (left, right) {
        (Truth::False, _) | (_, Truth::False) => Truth::False,
        (Truth::True, Truth::True) => Truth::True,
        _ => Truth::Unavailable,
    }
}

fn captured_truth(local: Truth, captured: Option<&Leaf<bool>>) -> Truth {
    if local != Truth::Unavailable {
        return local;
    }
    match captured {
        Some(Leaf::Measured(true)) => Truth::True,
        Some(Leaf::Measured(false)) => Truth::False,
        _ => Truth::Unavailable,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectApplyError {
    UnknownEffect,
    Disabled,
    EngineEvaluationRequired(Vec<String>),
    MissingRecordedState(String),
    DivisionByZero,
    NonFiniteResult,
}

pub fn apply_effect_preview(
    definition: &ScriptedGuiDefinition,
    effect_name: &str,
    state: &ScriptedGuiRuntimeState,
    captured: Option<&ScriptedGuiRuntimeInputs>,
) -> std::result::Result<ScriptedGuiRuntimeState, EffectApplyError> {
    let program = definition
        .effects
        .get(effect_name)
        .ok_or(EffectApplyError::UnknownEffect)?;
    match effect_enabled_truth(definition, effect_name, state, captured) {
        Truth::False => return Err(EffectApplyError::Disabled),
        Truth::Unavailable => {
            return Err(EffectApplyError::EngineEvaluationRequired(vec![format!(
                "{effect_name}_enabled"
            )]));
        }
        Truth::True => {}
    }
    if !program.unsupported_constructs.is_empty() {
        return Err(EffectApplyError::EngineEvaluationRequired(
            program.unsupported_constructs.clone(),
        ));
    }
    let mut next = state.clone();
    apply_ops(&program.operations, &mut next)?;
    Ok(next)
}

fn apply_ops(
    operations: &[EffectOp],
    state: &mut ScriptedGuiRuntimeState,
) -> std::result::Result<(), EffectApplyError> {
    for operation in operations {
        match operation {
            EffectOp::Set { variable, operand } => {
                let value = operand_value(operand, state)
                    .ok_or_else(|| EffectApplyError::MissingRecordedState(variable.name.clone()))?;
                assign_variable(state, variable, value)?;
            }
            EffectOp::Add { variable, operand }
            | EffectOp::Subtract { variable, operand }
            | EffectOp::Multiply { variable, operand }
            | EffectOp::Divide { variable, operand }
            | EffectOp::Modulo { variable, operand } => {
                let left = state
                    .variable(variable)
                    .ok_or_else(|| EffectApplyError::MissingRecordedState(variable.name.clone()))?;
                let right = operand_value(operand, state)
                    .ok_or_else(|| EffectApplyError::MissingRecordedState(variable.name.clone()))?;
                let value = match operation {
                    EffectOp::Add { .. } => left + right,
                    EffectOp::Subtract { .. } => left - right,
                    EffectOp::Multiply { .. } => left * right,
                    EffectOp::Divide { .. } => {
                        if right == 0.0 {
                            return Err(EffectApplyError::DivisionByZero);
                        }
                        left / right
                    }
                    EffectOp::Modulo { .. } => {
                        if right == 0.0 {
                            return Err(EffectApplyError::DivisionByZero);
                        }
                        left % right
                    }
                    _ => unreachable!(),
                };
                assign_variable(state, variable, value)?;
            }
            EffectOp::Clear { variable } => {
                state
                    .variables_mut(variable.scope)
                    .ok_or_else(|| EffectApplyError::MissingRecordedState(variable.name.clone()))?
                    .remove(&variable.name);
            }
            EffectOp::SetFlag { scope, name } => {
                state
                    .flags_mut(*scope)
                    .ok_or_else(|| EffectApplyError::MissingRecordedState(name.clone()))?
                    .insert(name.clone());
            }
            EffectOp::ClearFlag { scope, name } => {
                state
                    .flags_mut(*scope)
                    .ok_or_else(|| EffectApplyError::MissingRecordedState(name.clone()))?
                    .remove(name);
            }
            EffectOp::Conditional {
                predicate,
                then_ops,
                else_ops,
            } => match predicate.evaluate(state) {
                Truth::True => apply_ops(then_ops, state)?,
                Truth::False => apply_ops(else_ops, state)?,
                Truth::Unavailable => {
                    return Err(EffectApplyError::EngineEvaluationRequired(vec![
                        "conditional predicate".to_string(),
                    ]));
                }
            },
        }
    }
    Ok(())
}

fn assign_variable(
    state: &mut ScriptedGuiRuntimeState,
    variable: &VariableRef,
    value: f64,
) -> std::result::Result<(), EffectApplyError> {
    if !value.is_finite() {
        return Err(EffectApplyError::NonFiniteResult);
    }
    state
        .variables_mut(variable.scope)
        .ok_or_else(|| EffectApplyError::MissingRecordedState(variable.name.clone()))?
        .insert(variable.name.clone(), value);
    Ok(())
}

pub fn captured_inputs_for<'a>(
    view: &'a ScriptedGuiView,
    country_tag: &str,
    gui_name: &str,
) -> Option<&'a ScriptedGuiRuntimeInputs> {
    match view.runtime_by_country.get(country_tag) {
        Some(Leaf::Measured(rows)) => rows.get(gui_name),
        _ => None,
    }
}

/// Production entry point: bind one effective definition to one recorded
/// country context without exposing raw recording bytes to `game_ui`.
pub fn evaluate_registered_gui(
    registry: &ScriptedGuiRegistry,
    view: &ScriptedGuiView,
    country_tag: &str,
    gui_name: &str,
) -> Result<ScriptedGuiEvaluation> {
    let definition = definition_or_error(registry, gui_name)?;
    let state = ScriptedGuiRuntimeState::from_view(view, country_tag);
    Ok(evaluate_definition(
        definition,
        &state,
        captured_inputs_for(view, country_tag, gui_name),
    ))
}

pub fn definition_or_error<'a>(
    registry: &'a ScriptedGuiRegistry,
    name: &str,
) -> Result<&'a ScriptedGuiDefinition> {
    registry
        .definitions
        .get(name)
        .ok_or_else(|| GameUiError::missing(format!("scripted-GUI definition is absent: {name}")))
}
