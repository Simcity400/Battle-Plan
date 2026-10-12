//! Rust-owned contracts for bounded G1 replies and the Appendix C fixtures.
//!
//! Names, state mappings, union gates and numeric widths are authored here.
//! Consumers regenerate their contracts with `export-wire`; no game data is
//! required by either the generator or the synthetic fixtures.

use crate::model::*;
use serde_json::{Value, json};
use std::collections::VecDeque;

pub const DIAGNOSTICS: u32 = 1;
pub const NATIONAL_FOCUS_SERIES: u32 = 100;
pub const NATIONAL_FOCUS_CATALOGUE: u32 = 101;
pub const FOCUS_ARTWORK_PRESENTATION: u32 = 110;
pub const FOCUS_ARTWORK_DESCRIPTION: u32 = 111;
pub const EQUIPMENT_PRODUCED_CATALOGUE: u32 = 200;
pub const EQUIPMENT_PRODUCED_SERIES: u32 = 201;
pub const EQUIPMENT_PRODUCED_RANK: u32 = 202;
pub const EQUIPMENT_PRODUCED_SELECT: u32 = 203;
pub const CHART_SERIES: u32 = 300;
pub const DIVISION_LOG_TOTALS: u32 = 8000;
pub const GAME_SCORE_CATALOGUE: u32 = 8001;
pub const CONSTRUCTS: u32 = 9000;

#[derive(Clone)]
struct Node {
    name: String,
    descriptor: Descriptor,
    children: Vec<Node>,
    dictionary_kind: Option<DictionaryKind>,
    static_enum: Option<Vec<String>>,
    fixed_list_length: Option<u64>,
    state_mapping: StateMapping,
}

fn scalar(name: &str, logical_type: LogicalType, a: u32) -> Node {
    Node {
        name: name.into(),
        descriptor: Descriptor {
            logical_type,
            type_param_a: a,
            type_param_b: u32::from(logical_type == LogicalType::Bool),
            physical_encoding: if logical_type == LogicalType::Bool {
                Encoding::BitPacked
            } else {
                Encoding::Plain
            },
            ..Descriptor::default()
        },
        children: vec![],
        dictionary_kind: None,
        static_enum: None,
        fixed_list_length: None,
        state_mapping: StateMapping::default(),
    }
}
fn unsigned(name: &str, width: u32) -> Node {
    scalar(name, LogicalType::Unsigned, width)
}
fn signed(name: &str, width: u32) -> Node {
    scalar(name, LogicalType::Signed, width)
}
fn hour(name: &str) -> Node {
    scalar(name, LogicalType::Hour, 64)
}
fn boolean(name: &str) -> Node {
    scalar(name, LogicalType::Bool, 0)
}
fn bytes(name: &str) -> Node {
    scalar(name, LogicalType::Bytes, 0)
}
fn text(name: &str) -> Node {
    scalar(name, LogicalType::Text, 0)
}
fn wide(name: &str, signed: bool) -> Node {
    scalar(name, LogicalType::WideInt, u32::from(signed))
}
fn fixed(name: &str, width: u32, signed: bool, radix: u32, scale: u32) -> Node {
    let mut n = scalar(
        name,
        LogicalType::FixedPoint,
        width | (u32::from(signed) << 8),
    );
    n.descriptor.type_param_b = radix | (scale << 8);
    n
}
fn dictionary(name: &str, kind: DictionaryKind) -> Node {
    let mut n = scalar(name, LogicalType::Enum, 32);
    n.descriptor.physical_encoding = Encoding::Dictionary;
    n.dictionary_kind = Some(kind);
    n
}
fn refusal(name: &str) -> Node {
    let mut n = dictionary(name, DictionaryKind::Refusal);
    n.descriptor.logical_type = LogicalType::Refusal;
    n
}
fn static_enum(name: &str, variants: &[&str]) -> Node {
    let mut n = scalar(name, LogicalType::Enum, 8);
    n.static_enum = Some(variants.iter().map(|s| (*s).to_string()).collect());
    n
}
fn structure(name: &str, children: Vec<Node>) -> Node {
    let mut n = scalar(name, LogicalType::Struct, 0);
    n.children = children;
    n
}
fn list(name: &str, child: Node) -> Node {
    let mut n = scalar(name, LogicalType::List, 64);
    n.children = vec![child];
    n
}
fn native(mut n: Node) -> Node {
    n.descriptor.state_layers |= 2;
    n.descriptor.state_widths |= 0x10;
    n.state_mapping.native_absence = true;
    n
}
fn computed(mut n: Node) -> Node {
    n.descriptor.state_layers |= 2;
    n.descriptor.state_widths |= 0x10;
    n.state_mapping.computed_absence = true;
    n
}
fn recorded(mut n: Node) -> Node {
    n.descriptor.state_layers |= 1;
    n.descriptor.state_widths |= 1;
    n.state_mapping.source_distinction = true;
    n
}
fn known(mut n: Node, witness: &str) -> Node {
    n.descriptor.state_layers |= 4;
    n.descriptor.state_widths |= 0x100;
    n.state_mapping.knownness_witness = Some(witness.into());
    n
}
fn refused(mut n: Node, witness: &str) -> Node {
    n.descriptor.state_layers |= 8;
    n.descriptor.state_widths |= 0x2000;
    n.state_mapping.refusal_witness = Some(witness.into());
    n
}
fn gate(mut n: Node, value: u32) -> Node {
    n.descriptor.gate_column = 0;
    n.descriptor.gate_value = value;
    n
}

fn shape(
    id: u32,
    version: u32,
    name: &str,
    reply_kind: ReplyKind,
    unit_kinds: &[UnitKind],
    bounds: BoundsRule,
    roots: Vec<Node>,
) -> Shape {
    let top_level = (0..roots.len()).map(|i| i as u32).collect();
    let mut queue: VecDeque<Node> = roots.into();
    let mut names = vec![];
    let mut columns = vec![];
    while let Some(mut node) = queue.pop_front() {
        let i = columns.len();
        node.descriptor.name_id = i as u32;
        if !node.children.is_empty() {
            node.descriptor.child_first = (i + queue.len() + 1) as u32;
            node.descriptor.child_count = node.children.len() as u32;
            for mut child in node.children {
                child.name = format!("{}.{}", node.name, child.name);
                queue.push_back(child);
            }
        }
        names.push(node.name);
        columns.push(ColumnSpec {
            descriptor: node.descriptor,
            static_enum: node.static_enum,
            fixed_list_length: node.fixed_list_length,
            dictionary_kind: node.dictionary_kind,
            state_mapping: node.state_mapping,
        });
    }
    Shape {
        id,
        version,
        name: name.into(),
        reply_kind,
        unit_kinds: unit_kinds.to_vec(),
        names,
        columns,
        top_level,
        bounds,
    }
}

fn span(name: &str) -> Node {
    structure(name, vec![hour("first_hour"), hour("last_hour")])
}
fn identity_run() -> Node {
    structure(
        "$item",
        vec![
            hour("first_hour"),
            hour("last_hour"),
            unsigned("slot", 32),
            unsigned("country_id", 32),
        ],
    )
}
fn artwork(name: &str, descriptions: bool) -> Node {
    let mut fields = vec![
        dictionary("token", DictionaryKind::Plain),
        computed(dictionary("name", DictionaryKind::Text)),
    ];
    if descriptions {
        fields.push(computed(dictionary("description", DictionaryKind::Text)));
    }
    fields.push(computed(scalar(
        "base_duration_days",
        LogicalType::Float,
        64,
    )));
    fields.push(computed(dictionary("icon", DictionaryKind::ImageKey)));
    structure(name, fields)
}
fn produced_key(name: &str) -> Node {
    structure(
        name,
        vec![
            static_enum("$kind", &["Type", "Variant", "Domain"]),
            gate(dictionary("type_token", DictionaryKind::Plain), 0),
            gate(unsigned("variant", 64), 1),
            gate(dictionary("domain_token", DictionaryKind::Plain), 2),
        ],
    )
}
fn variant_name() -> Node {
    structure(
        "$item",
        vec![
            hour("first_hour"),
            known(computed(dictionary("name", DictionaryKind::Text)), "known"),
            boolean("known"),
            computed(dictionary("design", DictionaryKind::Plain)),
        ],
    )
}
fn coded(name: &str) -> Node {
    // Full-width differences preserve signed corrections and avoid safe53 loss.
    structure(
        name,
        vec![
            unsigned("from", 64),
            list("deltas", wide("$item", true)),
            list("unknown", unsigned("$item", 64)),
        ],
    )
}
fn exact_produced() -> Node {
    structure(
        "operands",
        vec![
            computed(list("identities", identity_run())),
            list("first_hours", hour("$item")),
            list("last_hours", hour("$item")),
            list("breaks", unsigned("$item", 64)),
            list(
                "values",
                structure(
                    "$item",
                    vec![
                        unsigned("key", 64),
                        coded("values"),
                        computed(coded("equipment_values")),
                    ],
                ),
            ),
            computed(coded("total")),
            computed(list("membership", span("$item"))),
            list("known", span("$item")),
            list("reads", hour("$item")),
            list("observations", hour("$item")),
        ],
    )
}
fn chart_columns() -> Vec<Node> {
    vec![
        structure(
            "series",
            vec![
                dictionary("country", DictionaryKind::Tag),
                produced_key("key"),
                static_enum("measure", &["Total", "Week", "Month"]),
            ],
        ),
        unsigned("first_run", 64),
        list(
            "runs",
            structure(
                "$item",
                vec![
                    signed("first", 32),
                    signed("last", 32),
                    scalar("value", LogicalType::Float, 64),
                    boolean("break_before"),
                    known(computed(refusal("break_reason")), "coverage"),
                    known(wide("exact", true), "coverage"),
                    boolean("coverage"),
                ],
            ),
        ),
        list("segment_starts", unsigned("$item", 32)),
        bytes("tree"),
        bytes("directory"),
    ]
}
fn missing(name: &str) -> Node {
    structure(
        name,
        vec![
            static_enum("kind", &["Recording", "Content"]),
            refusal("reason"),
        ],
    )
}
fn score_identity() -> Node {
    let mut color = native(list("color", unsigned("$item", 8)));
    color.fixed_list_length = Some(3);
    structure(
        "value",
        vec![
            computed(unsigned("canonical_slot", 32)),
            native(unsigned("native_country", 32)),
            refused(
                computed(dictionary("name", DictionaryKind::Text)),
                "name_missing",
            ),
            computed(missing("name_missing")),
            color,
            computed(missing("color_missing")),
            native(structure(
                "faction",
                vec![
                    unsigned("id", 64),
                    refused(
                        computed(dictionary("name", DictionaryKind::Text)),
                        "name_missing",
                    ),
                    computed(missing("name_missing")),
                    unsigned("ordinal", 32),
                ],
            )),
            computed(missing("faction_missing")),
        ],
    )
}

/// G1's closed product domain plus real C.12 reference shapes and a primitive
/// conformance shape. Reply row/child coordinates are always 64-bit.
#[allow(clippy::vec_init_then_push)] // One push per shape keeps the registry diffable.
pub fn registry() -> Vec<Shape> {
    let mut out = vec![];
    out.push(shape(
        DIAGNOSTICS,
        1,
        "shared.diagnostics",
        ReplyKind::Diagnostics,
        &[UnitKind::Page, UnitKind::Complete],
        BoundsRule::Forbidden,
        vec![
            dictionary("code", DictionaryKind::DiagnosticCode),
            computed(static_enum("severity", &["info", "warning", "error"])),
            static_enum(
                "source_kind",
                &[
                    "warnings",
                    "content_warnings",
                    "score_warnings",
                    "row_reason",
                    "error",
                ],
            ),
            unsigned("occurrence", 64),
            computed(unsigned("affected_count", 64)),
            computed(unsigned("observations", 64)),
            computed(structure(
                "entity",
                vec![
                    static_enum(
                        "kind",
                        &["country", "battle", "receipt", "division", "other"],
                    ),
                    dictionary("code", DictionaryKind::Plain),
                    wide("native_id", false),
                ],
            )),
            computed(hour("hour")),
            computed(scalar("native_date", LogicalType::NativeDate, 64)),
            text("message"),
            computed(bytes("detail")),
        ],
    ));
    out.push(shape(
        NATIONAL_FOCUS_SERIES,
        1,
        "national-focus.series",
        ReplyKind::Series,
        &[UnitKind::Window, UnitKind::Page],
        BoundsRule::Required,
        vec![structure(
            "history",
            vec![
                list(
                    "segments",
                    structure(
                        "$item",
                        vec![
                            hour("first_hour"),
                            hour("last_hour"),
                            structure(
                                "order",
                                vec![
                                    list(
                                        "completed",
                                        structure(
                                            "$item",
                                            vec![
                                                unsigned("ordinal", 64),
                                                dictionary("token", DictionaryKind::Plain),
                                                computed(hour("completed_by_hour")),
                                            ],
                                        ),
                                    ),
                                    recorded(native(dictionary("current", DictionaryKind::Plain))),
                                    recorded(native(dictionary(
                                        "continuous",
                                        DictionaryKind::Plain,
                                    ))),
                                    recorded(boolean("paused")),
                                ],
                            ),
                        ],
                    ),
                ),
                list(
                    "progress",
                    structure(
                        "$item",
                        vec![
                            hour("first_hour"),
                            hour("last_hour"),
                            recorded(fixed("progress_raw", 64, true, 10, 5)),
                        ],
                    ),
                ),
            ],
        )],
    ));
    out.push(shape(
        NATIONAL_FOCUS_CATALOGUE,
        1,
        "national-focus.catalogue",
        ReplyKind::Catalogue,
        &[UnitKind::Page],
        BoundsRule::Forbidden,
        vec![artwork("artwork", true)],
    ));
    out.push(shape(
        FOCUS_ARTWORK_PRESENTATION,
        1,
        "focus-artwork.presentation",
        ReplyKind::Presentation,
        &[UnitKind::Page],
        BoundsRule::Forbidden,
        vec![
            unsigned("branch_id", 64),
            hour("hour"),
            dictionary("tag", DictionaryKind::Tag),
            unsigned("request_ordinal", 64),
            artwork("artwork", true),
            computed(refusal("reason")),
        ],
    ));
    out.push(shape(
        FOCUS_ARTWORK_DESCRIPTION,
        1,
        "focus-artwork.description",
        ReplyKind::Description,
        &[UnitKind::Page],
        BoundsRule::Forbidden,
        vec![
            unsigned("branch_id", 64),
            hour("hour"),
            dictionary("tag", DictionaryKind::Tag),
            dictionary("token", DictionaryKind::Plain),
            refused(text("description"), "reason"),
            computed(refusal("reason")),
        ],
    ));
    out.push(shape(
        EQUIPMENT_PRODUCED_CATALOGUE,
        2,
        "equipment-produced.catalogue",
        ReplyKind::Catalogue,
        &[UnitKind::Page],
        BoundsRule::Forbidden,
        vec![
            unsigned("branch_id", 64),
            static_enum("measure", &["Total", "Week", "Month"]),
            static_enum("grouping", &["Type", "Variant", "Domain"]),
            list(
                "types",
                structure(
                    "$item",
                    vec![
                        dictionary("key", DictionaryKind::Plain),
                        computed(dictionary("name", DictionaryKind::Text)),
                        computed(static_enum("domain", &["Land", "Air", "Naval"])),
                    ],
                ),
            ),
            list(
                "keys",
                structure(
                    "$item",
                    vec![
                        produced_key("key"),
                        computed(dictionary("name", DictionaryKind::Text)),
                        computed(dictionary("type_key", DictionaryKind::Plain)),
                        computed(static_enum("domain", &["Land", "Air", "Naval"])),
                        list("names", variant_name()),
                    ],
                ),
            ),
        ],
    ));
    let mut produced_series = chart_columns();
    produced_series.insert(0, unsigned("branch_id", 64));
    produced_series.push(exact_produced());
    out.push(shape(
        EQUIPMENT_PRODUCED_SERIES,
        2,
        "equipment-produced.series",
        ReplyKind::Series,
        &[UnitKind::Window, UnitKind::Page],
        BoundsRule::Required,
        produced_series,
    ));
    out.push(shape(
        EQUIPMENT_PRODUCED_RANK,
        2,
        "equipment-produced.rank",
        ReplyKind::Rank,
        &[UnitKind::Window, UnitKind::Page],
        BoundsRule::Required,
        vec![
            dictionary("tag", DictionaryKind::Tag),
            computed(list("identities", identity_run())),
            list("membership", span("$item")),
            unsigned("rank", 64),
        ],
    ));
    out.push(shape(
        EQUIPMENT_PRODUCED_SELECT,
        2,
        "equipment-produced.select",
        ReplyKind::Select,
        &[UnitKind::TopK, UnitKind::Page],
        BoundsRule::Required,
        vec![
            dictionary("country", DictionaryKind::Tag),
            refused(
                known(wide("score", true), "exact"),
                "unresolved_contribution",
            ),
            unsigned("rank", 64),
            boolean("exact"),
            computed(refusal("unresolved_contribution")),
            exact_produced(),
        ],
    ));
    out.push(shape(
        CHART_SERIES,
        1,
        "chart.series",
        ReplyKind::Series,
        &[UnitKind::Window, UnitKind::Page, UnitKind::Viewport],
        BoundsRule::Required,
        chart_columns(),
    ));
    out.push(shape(
        DIVISION_LOG_TOTALS,
        1,
        "division-log-totals.windowTotal",
        ReplyKind::WindowTotal,
        &[UnitKind::Complete],
        BoundsRule::Required,
        vec![
            unsigned("created", 64),
            unsigned("lost", 64),
            unsigned("transferred", 64),
        ],
    ));
    out.push(shape(
        GAME_SCORE_CATALOGUE,
        1,
        "game_score.catalogue",
        ReplyKind::Catalogue,
        &[UnitKind::Page],
        BoundsRule::Forbidden,
        vec![
            unsigned("branch_id", 64),
            scalar("scale", LogicalType::Safe53, 0),
            list("reasons", missing("$item")),
            list(
                "labels",
                structure(
                    "$item",
                    vec![
                        dictionary("key", DictionaryKind::Text),
                        dictionary("value", DictionaryKind::Text),
                    ],
                ),
            ),
            list(
                "hints",
                structure(
                    "$item",
                    vec![
                        dictionary("key", DictionaryKind::Text),
                        dictionary("value", DictionaryKind::Text),
                    ],
                ),
            ),
            dictionary("tag", DictionaryKind::Tag),
            list(
                "identity",
                structure(
                    "$item",
                    vec![hour("first_hour"), hour("last_hour"), score_identity()],
                ),
            ),
        ],
    ));
    let mut bitpacked = unsigned("packed", 8);
    bitpacked.descriptor.physical_encoding = Encoding::BitPacked;
    bitpacked.descriptor.type_param_b = 3;
    let mut rle = signed("rle", 64);
    rle.descriptor.physical_encoding = Encoding::RunLength;
    let mut wide_dict = dictionary("wide_dictionary", DictionaryKind::Plain);
    wide_dict.descriptor.type_param_a = 64;
    let mut wide_refusal = refused(unsigned("refused", 64), "fixture_reason");
    wide_refusal.descriptor.state_widths = 0x4000;
    out.push(shape(
        CONSTRUCTS,
        1,
        "wire.constructs",
        ReplyKind::Catalogue,
        &[UnitKind::Page],
        BoundsRule::Optional,
        vec![
            boolean("bool"),
            unsigned("u8", 8),
            unsigned("u16", 16),
            unsigned("u32", 32),
            unsigned("u64", 64),
            signed("i8", 8),
            signed("i16", 16),
            signed("i32", 32),
            signed("i64", 64),
            scalar("safe53", LogicalType::Safe53, 0),
            wide("u128", false),
            wide("i128", true),
            fixed("fixed10", 64, true, 10, 5),
            fixed("fixed2", 128, true, 2, 15),
            scalar("f32", LogicalType::Float, 32),
            scalar("f64", LogicalType::Float, 64),
            hour("hour"),
            scalar("native_date", LogicalType::NativeDate, 64),
            bytes("bytes"),
            text("text"),
            wide_dict,
            refusal("refusal"),
            bitpacked,
            rle,
            known(recorded(unsigned("recorded_known", 64)), "fixture_known"),
            native(unsigned("native_option", 64)),
            computed(unsigned("computed_option", 64)),
            wide_refusal,
            structure(
                "union",
                vec![
                    static_enum("$kind", &["number", "text"]),
                    gate(unsigned("number", 64), 0),
                    gate(dictionary("text", DictionaryKind::Text), 1),
                ],
            ),
            list(
                "nested",
                list("$item", list("$item", unsigned("$item", 64))),
            ),
        ],
    ));
    out
}

fn shape_json(shape: &Shape, columns: Vec<Value>) -> Value {
    json!({
        "id":shape.id,"version":shape.version,"name":shape.name,
        "replyKind":shape.reply_kind as u32,
        "unitKinds":shape.unit_kinds.iter().map(|k| *k as u32).collect::<Vec<_>>(),
        "names":shape.names,"topLevel":shape.top_level,
        "bounds":match shape.bounds {BoundsRule::Forbidden=>"forbidden",BoundsRule::Optional=>"optional",BoundsRule::Required=>"required"},
        "columns":columns
    })
}

/// Full contracts retain producer roles and reference shapes for fixture export.
pub fn contract_json(shape: &Shape) -> Value {
    shape_json(shape, shape.columns.iter().map(|c| {
            let d=&c.descriptor;
            json!({"descriptor":{
                "nameId":d.name_id,"logicalType":d.logical_type as u16,"encoding":d.physical_encoding as u8,
                "stateLayers":d.state_layers,"paramA":d.type_param_a,"paramB":d.type_param_b,
                "dictionaryRef":d.dictionary_ref,"childFirst":d.child_first,"childCount":d.child_count,
                "dataBuffer":d.data_buffer,"length":d.length.to_string(),"auxBuffer":d.aux_buffer,
                "stateBuffer":d.state_buffer,"stateWidths":d.state_widths,
                "gateColumn":d.gate_column,"gateValue":d.gate_value,"refusalRef":d.refusal_ref,"reserved":d.reserved},
                "staticEnum":c.static_enum,"fixedListLength":c.fixed_list_length.map(|v|v.to_string()),
                "dictionaryKind":c.dictionary_kind.map(|v|v as u32),
                "stateMapping":{"nativeAbsence":c.state_mapping.native_absence,"computedAbsence":c.state_mapping.computed_absence,
                    "knownnessWitness":c.state_mapping.knownness_witness,"refusalWitness":c.state_mapping.refusal_witness,
                    "sourceDistinction":c.state_mapping.source_distinction}})
        }).collect())
}

pub fn contracts_json() -> Value {
    Value::Array(registry().iter().map(contract_json).collect())
}

fn runtime_shapes() -> Vec<Shape> {
    registry()
        .into_iter()
        .filter(|s| {
            !matches!(
                s.id,
                DIVISION_LOG_TOTALS | GAME_SCORE_CATALOGUE | CONSTRUCTS
            )
        })
        .collect()
}

fn runtime_contract_json(shape: &Shape) -> Value {
    shape_json(
        shape,
        shape
            .columns
            .iter()
            .map(|c| {
                let d = &c.descriptor;
                let mut descriptor =
                    json!({"nameId":d.name_id,"logicalType":d.logical_type as u16});
                // These shared defaults are applied by the runtime's shape comparison.
                for (name, value, default) in [
                    ("encoding", u64::from(d.physical_encoding as u8), 0),
                    ("stateLayers", u64::from(d.state_layers), 0),
                    ("paramA", u64::from(d.type_param_a), 0),
                    ("paramB", u64::from(d.type_param_b), 0),
                    (
                        "childFirst",
                        u64::from(d.child_first),
                        u64::from(NONE_INDEX),
                    ),
                    ("childCount", u64::from(d.child_count), 0),
                    ("gateColumn", u64::from(d.gate_column), u64::from(u16::MAX)),
                    ("gateValue", u64::from(d.gate_value), 0),
                ] {
                    if value != default {
                        descriptor[name] = json!(value);
                    }
                }
                let mut column = json!({"descriptor":descriptor});
                if let Some(variants) = &c.static_enum {
                    column["staticEnum"] = json!(variants);
                }
                if let Some(length) = c.fixed_list_length {
                    column["fixedListLength"] = json!(length.to_string());
                }
                if let Some(kind) = c.dictionary_kind {
                    column["dictionaryKind"] = json!(kind as u32);
                }
                column
            })
            .collect(),
    )
}

/// Production consumers need shape checks, enums, bounds and dictionary kinds.
pub fn runtime_contracts_json() -> Value {
    Value::Array(runtime_shapes().iter().map(runtime_contract_json).collect())
}

/// The runtime is handwritten; all shape-specific contracts and reader wrappers
/// below are generated by this function from the Rust registry.
pub fn typescript() -> String {
    let mut source = String::from(
        "// Generated by wire/export-wire. Do not edit.\nimport { decodeReply, type ShapeContract, type ExpectedReply, type DecodedReply } from '../wire/runtime.ts';\n\n",
    );
    let shapes = runtime_shapes();
    let contracts = Value::Array(shapes.iter().map(runtime_contract_json).collect());
    let mut contracts = serde_json::to_string(&contracts).expect("static schema JSON");
    for length in shapes
        .iter()
        .flat_map(|s| &s.columns)
        .filter_map(|c| c.fixed_list_length)
    {
        contracts = contracts.replace(
            &format!("\"fixedListLength\":\"{length}\""),
            &format!("\"fixedListLength\":{length}n"),
        );
    }
    source.push_str("export const WIRE_SHAPES: readonly ShapeContract[] = ");
    source.push_str(&contracts);
    source.push_str(";\n\n");
    source.push_str("export const WIRE_SHAPES_BY_ID: ReadonlyMap<number, ShapeContract> = new Map(WIRE_SHAPES.map((s) => [s.id, s]));\n");
    source.push_str("export function wireShape(id: number): ShapeContract { const s = WIRE_SHAPES_BY_ID.get(id); if (!s) throw new Error('unknown-shape'); return s; }\n");
    source.push_str("export function readWire(buffer: ArrayBuffer, id: number, expected: ExpectedReply = {}): DecodedReply { return decodeReply(buffer, wireShape(id), expected, WIRE_SHAPES_BY_ID); }\n");
    for shape in shapes {
        let name: String = shape
            .name
            .split(['-', '.', '_'])
            .filter(|s| !s.is_empty())
            .map(|s| {
                let mut chars = s.chars();
                chars
                    .next()
                    .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                    .unwrap_or_default()
            })
            .collect();
        source.push_str(&format!("export function read{name}(buffer: ArrayBuffer, expected: ExpectedReply = {{}}): DecodedReply {{ return readWire(buffer, {}, expected); }}\n", shape.id));
    }
    source
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_runtime_contract_excludes_producer_and_fixture_metadata() {
        let source = typescript();
        for field in [
            "dictionaryRef",
            "dataBuffer",
            "length",
            "auxBuffer",
            "stateBuffer",
            "stateWidths",
            "refusalRef",
            "reserved",
            "stateMapping",
        ] {
            assert!(
                !source.contains(&format!("\"{field}\"")),
                "unused field: {field}"
            );
        }
        for name in [
            "division-log-totals.windowTotal",
            "game_score.catalogue",
            "wire.constructs",
        ] {
            assert!(!source.contains(name), "fixture-only shape: {name}");
        }
        assert!(!source.contains("raw.map"));
        assert!(!source.contains("columns.map"));
        let full = contracts_json();
        assert!(
            full.as_array()
                .unwrap()
                .iter()
                .any(|s| s["id"] == CONSTRUCTS)
        );
        assert!(full[0]["columns"][0].get("stateMapping").is_some());
        assert!(full[0]["columns"][0]["descriptor"].get("length").is_some());
    }

    #[test]
    fn sparse_contract_retains_every_runtime_shape_check_and_semantic_rule() {
        let defaults = json!({
            "encoding":0,"stateLayers":0,"paramA":0,"paramB":0,
            "childFirst":NONE_INDEX,"childCount":0,"gateColumn":u16::MAX,"gateValue":0,
        });
        let full = registry();
        let runtime = runtime_contracts_json();
        let runtime = runtime.as_array().unwrap();
        for shape in &full {
            let compact = runtime.iter().find(|s| s["id"] == shape.id);
            if matches!(
                shape.id,
                DIVISION_LOG_TOTALS | GAME_SCORE_CATALOGUE | CONSTRUCTS
            ) {
                assert!(compact.is_none());
                continue;
            }
            let compact = compact.expect("registered production shape");
            let complete = contract_json(shape);
            for key in [
                "id",
                "version",
                "name",
                "replyKind",
                "unitKinds",
                "names",
                "topLevel",
                "bounds",
            ] {
                assert_eq!(compact[key], complete[key], "{}.{key}", shape.name);
            }
            let columns = compact["columns"].as_array().unwrap();
            assert_eq!(columns.len(), shape.columns.len());
            for (i, column) in columns.iter().enumerate() {
                let descriptor = &column["descriptor"];
                let original = &complete["columns"][i];
                for key in [
                    "nameId",
                    "logicalType",
                    "encoding",
                    "stateLayers",
                    "paramA",
                    "paramB",
                    "childFirst",
                    "childCount",
                    "gateColumn",
                    "gateValue",
                ] {
                    assert_eq!(
                        descriptor.get(key).unwrap_or(&defaults[key]),
                        &original["descriptor"][key],
                        "{}[{i}].{key}",
                        shape.name
                    );
                }
                for key in ["staticEnum", "fixedListLength", "dictionaryKind"] {
                    assert_eq!(column.get(key).unwrap_or(&Value::Null), &original[key]);
                    assert!(!column.get(key).is_some_and(Value::is_null));
                }
                for (key, default) in defaults.as_object().unwrap() {
                    assert_ne!(
                        descriptor.get(key),
                        Some(default),
                        "redundant default: {key}"
                    );
                }
            }
        }
    }
}
