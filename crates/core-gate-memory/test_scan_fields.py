import unittest
from collections import defaultdict
from pathlib import Path
from tempfile import TemporaryDirectory

from scan_fields import (
    PARSER,
    audit_whole_rows,
    code,
    compare,
    nodes,
    receiver_tables,
    scan,
    value_tree,
    whole_row_sites,
)


class FieldProofTests(unittest.TestCase):
    def schemas(self):
        return {
            "Alpha": {"row": "AlphaRow", "fields": ["value", "gate"], "types": {"value": "i64", "gate": "u32"}},
            "Beta": {"row": "BetaRow", "fields": ["value"], "types": {"value": "Recorded<i64>"}},
        }

    def sites(self, source, scope=None):
        data = source.encode()
        tree = value_tree(data, PARSER.parse(data))
        return whole_row_sites(tree, data, "synthetic.rs", "synthetic", set(self.schemas()), self.schemas(), scope or {}, {})

    def test_table_arguments_follow_helpers_outside_the_selected_function(self):
        source = """
fn decode(state: &State, table: TableId) { columns::row(state, table, 0, 1); }
fn forward(state: &State, table: TableId) { decode(state, table); }
fn serve(state: &State) {
    forward(state, TableId::Alpha);
    forward(state, TableId::Beta);
}
"""
        sites, unresolved = self.sites(source, {"functions": ["serve"]})
        self.assertEqual({site["table"] for site in sites}, {"Alpha", "Beta"})
        self.assertTrue(all(site["decode_sources"] == ["synthetic.rs:2"] for site in sites))
        self.assertEqual(unresolved, [])

    def test_a_new_site_on_an_already_audited_table_still_fails(self):
        sites, unresolved = self.sites("fn serve(s: &State) { columns::row(s, TableId::Alpha, 0, 1); }")
        config = {"whole_reads": [{"family": "synthetic", "table": "Alpha", "sites": []}]}
        missing, excluded = audit_whole_rows(sites, unresolved, config)
        self.assertEqual(missing, sites)
        self.assertEqual(excluded, [])
        config["whole_reads"][0]["sites"] = [sites[0]["source"]]
        self.assertEqual(audit_whole_rows(sites, unresolved, config), ([], []))

    def test_an_uninstantiated_decode_cannot_silently_disappear(self):
        sites, unresolved = self.sites("fn decode(s: &State, table: TableId) { columns::row(s, table, 0, 1); }")
        self.assertEqual(sites, [])
        self.assertEqual(len(unresolved), 1)
        self.assertEqual(audit_whole_rows(sites, unresolved, {})[0], unresolved)

    def test_table_types_follow_arbitrarily_named_helpers(self):
        source = """
fn serve(s: &State) {
    forward::<tables::Beta, tables::Alpha>(s);
}
fn forward<'a, Unused, Selected: TableSchema>(s: &'a State) { partial::<Selected>(s); }
fn partial<T: TableSchema>(s: &State) {
    match s.view(T::ID) {
        Complete(rows) => rows.rows::<T>(),
        Truncated(rows) => rows.rows::<T>(),
    }
}
"""
        sites, unresolved = self.sites(source, {"functions": ["serve"]})
        self.assertEqual([site["table"] for site in sites], ["Alpha"])
        self.assertEqual(sites[0]["decode_sources"], ["synthetic.rs:8", "synthetic.rs:9"])
        self.assertEqual(unresolved, [])
        config = {"whole_reads": [{"family": "synthetic", "table": "Alpha", "sites": []}]}
        self.assertEqual(audit_whole_rows(sites, unresolved, config), (sites, []))
        config["whole_reads"][0]["sites"] = [sites[0]["source"]]
        self.assertEqual(audit_whole_rows(sites, unresolved, config), ([], []))

    def test_table_types_forward_into_table_id_decodes(self):
        source = """
fn decode<T: TableSchema>(s: &State) { columns::row(s, T::ID, 0, 1); }
fn serve(s: &State) { decode::<tables::Alpha>(s); }
"""
        sites, unresolved = self.sites(source)
        self.assertEqual([site["table"] for site in sites], ["Alpha"])
        self.assertEqual(sites[0]["decode_sources"], ["synthetic.rs:2"])
        self.assertEqual(unresolved, [])

    def test_an_uninstantiated_type_decode_cannot_silently_disappear(self):
        sites, unresolved = self.sites("fn partial<T: TableSchema>(rows: &Rows) { rows.rows::<T>(); }")
        self.assertEqual(sites, [])
        self.assertEqual(len(unresolved), 1)
        self.assertEqual(audit_whole_rows(sites, unresolved, {})[0], unresolved)

    def test_value_reads_cannot_be_satisfied_by_bits(self):
        manifest = {"families": [{"name": "synthetic", "tables": [{"name": "Alpha", "fields": ["value", "gate"], "recorded_only": ["gate"]}]}]}
        fact = dict(family="synthetic", table="Alpha", field="gate", source="synthetic.rs:1", kind="member")
        self.assertEqual(compare(manifest, [fact], self.schemas())[0]["error"], "value read declared as recorded bits only")
        fact["kind"] = "required recorded bit"
        self.assertEqual(compare(manifest, [fact], self.schemas()), [])

    def test_query_values_require_query_labels(self):
        table = {"name": "Alpha", "fields": ["value"]}
        manifest = {"families": [{"name": "synthetic", "tables": [table]}]}
        fact = dict(family="synthetic", table="Alpha", field="value", source="synthetic.rs:1", kind="member", query_read=True)
        self.assertEqual(len(compare(manifest, [fact], self.schemas())), 1)
        table["query_sources"] = {"value": "synthetic.rs:1"}
        self.assertEqual(compare(manifest, [fact], self.schemas()), [])

    def test_query_calls_follow_imports_reexports_and_associated_readers(self):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            for crate in ["product", "shared"]:
                path = root / "crates" / crate
                (path / "src").mkdir(parents=True)
                (path / "Cargo.toml").write_text("")
            (root / "crates/product/src/lib.rs").write_text("""
use shared::{Reader as Prepared, guard};
pub fn query() { wrapper(); guard(); }
fn wrapper() { Prepared::read(); }
""")
            (root / "crates/shared/src/lib.rs").write_text("""
mod operands;
pub use operands::{Reader, validate as guard};
""")
            (root / "crates/shared/src/operands.rs").write_text("""
struct Reader;
impl Reader {
    pub fn read(row: &AlphaRow) { helper(row); }
}
fn helper(row: &AlphaRow) { row.gate; }
pub fn validate(row: &BetaRow) { row.value; }
pub fn unused(row: &BetaRow) { row.value; }
""")
            config = {"families": {"synthetic": [dict(path="crates/product/src/lib.rs", tables=["Beta"], queries=["query"])]}}
            facts, _, _, scopes, _, edges = scan(root, config, self.schemas())
            self.assertEqual({(f["table"], f["field"]) for f in facts}, {("Alpha", "gate"), ("Beta", "value")})
            self.assertTrue(all(f["query_read"] for f in facts))
            self.assertFalse(any(f.get("function") == "unused" for f in facts))
            self.assertTrue(any(e["expression"] == "Prepared::read" for e in edges))
            table = dict(name="Alpha", fields=["gate"], recorded_only=["gate"])
            manifest = {"families": [dict(name="synthetic", tables=[table, dict(name="Beta", fields=["value"], query_sources={"value": "synthetic.rs:1"})])]}
            self.assertEqual(compare(manifest, facts, self.schemas())[0]["error"], "value read declared as recorded bits only")
            table.pop("recorded_only")
            self.assertEqual(compare(manifest, facts, self.schemas())[0]["error"], "query value lacks a query_sources label")
            table["query_sources"] = {"gate": "synthetic.rs:1"}
            self.assertEqual(compare(manifest, facts, self.schemas()), [])

    def test_cross_file_query_table_arguments_reach_generic_decoders(self):
        with TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "crates/shared"
            (path / "src").mkdir(parents=True)
            (path / "Cargo.toml").write_text("")
            (path / "src/lib.rs").write_text("mod helper; fn query() { helper::project::<tables::Alpha>(); }")
            (path / "src/helper.rs").write_text("pub fn project<T: TableSchema>() { nested::<T>(); } fn nested<U: TableSchema>() { s.rows::<U>(); }")
            config = {"families": {"synthetic": [dict(path="crates/shared/src/lib.rs", queries=["query"])]}}
            _, whole, _, _, unresolved, _ = scan(root, config, self.schemas())
            self.assertTrue(whole)
            self.assertEqual({s["table"] for s in whole}, {"Alpha"})
            self.assertEqual(unresolved, [])
            self.assertEqual(audit_whole_rows(whole, unresolved, {})[0], whole)

    def test_macros_keep_member_offsets_and_find_whole_decodes(self):
        source = 'fn serve(s: &State) { format!("{:?}", columns::row(s, TableId::Alpha, 0, 1)); }'
        sites, unresolved = self.sites(source)
        self.assertEqual([site["table"] for site in sites], ["Alpha"])
        self.assertEqual(unresolved, [])
        data = b'fn serve(row: &AlphaRow) { format!("{}", row.value); }'
        fields = [n for n in nodes(value_tree(data, PARSER.parse(data)).root_node) if n.type == "field_expression"]
        self.assertEqual([code(n, data) for n in fields], ["row.value"])

    def test_an_untyped_closure_parameter_shadows_an_outer_typed_parameter(self):
        data = b'fn serve(row: &AlphaRow) { rows.map(|row| row.value); row.gate; }'
        fields = [n for n in nodes(PARSER.parse(data).root_node) if n.type == "field_expression" and code(n, data).startswith("row.")]
        row_tables = defaultdict(set, {"AlphaRow": {"Alpha"}})
        self.assertIsNone(receiver_tables(fields[0], "row", data, row_tables))
        self.assertEqual(receiver_tables(fields[1], "row", data, row_tables), {"Alpha"})


if __name__ == "__main__":
    unittest.main()
