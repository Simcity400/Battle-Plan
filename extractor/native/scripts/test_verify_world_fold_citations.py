#!/usr/bin/env python3
"""Focused deterministic tests for verify_world_fold_citations.py."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
import re
import sys
import tempfile
import unittest
import unittest.mock
from pathlib import Path, PurePosixPath


def _load(name: str):
    script = Path(__file__).with_name(f"{name}.py")
    spec = importlib.util.spec_from_file_location(name, script)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


VERIFY = _load("verify_world_fold_citations")
RESTAMP = _load("restamp_world_fold_citations")


class CitationFixture:
    def __init__(self, root: Path, world: str) -> None:
        self.root = root
        self.world = world
        self.world_path = root / VERIFY.WORLD_FOLD
        self.wire_dir = root / VERIFY.WIRE_SRC
        self.world_path.parent.mkdir(parents=True)
        self.wire_dir.mkdir(parents=True)
        self.world_path.write_text(world, encoding="utf-8", newline="\n")
        self.closures: dict[str, list[str]] = {}

    def set_world(self, world: str) -> None:
        """Rewrite world_fold.rs WITHOUT touching any marker or the inventory.

        That is the interesting state: the source moved and nothing that cites
        it has been told, which is exactly when a gate either notices or does
        not.
        """
        self.world = world
        self.world_path.write_text(world, encoding="utf-8", newline="\n")

    def closure_of(self, functions: tuple[str, ...]) -> set[str]:
        index = VERIFY.index_items(self.world)
        return VERIFY.callee_closure(index, VERIFY.authority_roots(index, functions))

    def add_module(
        self,
        domain: int,
        functions: tuple[str, ...],
        *,
        digest_override: str | None = None,
        include_marker: bool = True,
    ) -> None:
        spans = VERIFY.function_spans(self.world)
        envelope = VERIFY.Span(
            min(spans[name].start for name in functions),
            max(spans[name].end for name in functions),
        )
        digest = digest_override or VERIFY.normalized_span_hash(
            self.world, envelope
        )
        index = VERIFY.index_items(self.world)
        members = self.closure_of(functions)
        closure = VERIFY.closure_digest(index, members)
        marker = ""
        if include_marker:
            marker = (
                "// @world-fold-authority "
                f"domain={domain} functions={','.join(functions)} "
                f"lines={envelope.start}-{envelope.end} sha256={digest} "
                f"closure={closure}\n"
            )
        citation = f"`world_fold.rs:{envelope.start}`-`:{envelope.end}`"
        (self.wire_dir / f"d{domain}.rs").write_text(
            f"//! fixture derived from {citation}\n{marker}",
            encoding="utf-8",
            newline="\n",
        )
        self.closures[str(domain)] = sorted(members)
        self.write_inventory()

    def write_inventory(self, domains: dict[str, list[str]] | None = None) -> None:
        path = self.root / VERIFY.CLOSURE_INVENTORY
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            json.dumps({"domains": domains if domains is not None else self.closures}),
            encoding="utf-8",
            newline="\n",
        )

    def marker_of(self, domain: int) -> re.Match[str]:
        module = self.wire_dir / f"d{domain}.rs"
        return VERIFY.WORLD_MARKER_RE.search(module.read_text(encoding="utf-8"))

    def add_lane_authority(
        self,
        domain: int,
        relative: str,
        function: str,
        source: str,
    ) -> tuple[int, str, str]:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source, encoding="utf-8", newline="\n")
        spans = VERIFY.function_spans(source)
        span = spans[function]
        digest = VERIFY.normalized_span_hash(source, span)
        module = self.wire_dir / f"d{domain}.rs"
        with module.open("a", encoding="utf-8", newline="\n") as handle:
            handle.write(
                "// @lane-payload-authority "
                f"domain={domain} path={relative} function={function} "
                f"lines={span.start}-{span.end} sha256={digest}\n"
            )
        return (domain, relative, function)


WORLD = """\
impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_one(),
            39 => self.fold_shared(),
            40 => self.fold_shared(),
            51 => self.fold_lane(),
            67 => self.fold_multi(),
            _ => (),
        }
    }

    fn fold_one(&mut self) {
        let diagnostic = "a brace in a string: }";
        /* and a nested comment { /* } */ } */
        self.value += 1;
    }

    fn fold_shared(&mut self) {
        self.value += 2;
    }

    fn fold_lane(&mut self) {
        self.value += 3;
    }

    fn fold_multi(&mut self) {
        self.fold_multi_helper();
    }

    fn fold_multi_helper(&mut self) {
        let raw = r#"raw braces { }"#;
    }
}
"""



INLINE_RENDER = """\
impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_render(),
            2 => self.fold_other(),
            _ => (),
        }
    }

    fn fold_render(&mut self) {
        let mut document = Map::new();
        document.insert("dug_in".into(), json!(self.dig_in.min(2)));
        self.documents.push(document);
    }

    fn fold_other(&mut self) {
        self.value += 7;
    }

    fn unrelated_helper(&mut self) {
        self.value += 11;
    }
}
"""

EXTRACTED_RENDER = """\
impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_render(),
            2 => self.fold_other(),
            _ => (),
        }
    }

    fn fold_render(&mut self) {
        let document = Self::render_document(self.dig_in);
        self.documents.push(document);
    }

    fn fold_other(&mut self) {
        self.value += 7;
    }

    fn unrelated_helper(&mut self) {
        self.value += 11;
    }

    fn render_document(dig_in: u32) -> Map {
        let mut document = Map::new();
        document.insert("dug_in".into(), json!(dig_in.min(2)));
        document
    }
}
"""

MOVED_OUT_OF_FILE = EXTRACTED_RENDER.replace(
    "        let document = Self::render_document(self.dig_in);",
    "        let document = rendering::render_document(self.dig_in);",
).replace(
    """
    fn render_document(dig_in: u32) -> Map {
        let mut document = Map::new();
        document.insert("dug_in".into(), json!(dig_in.min(2)));
        document
    }
""",
    "",
)


class AuthorityClosureTests(unittest.TestCase):
    """The gate must survive a helper being lifted out of a cited fold."""

    def fixture(
        self, world: str
    ) -> tuple[tempfile.TemporaryDirectory[str], CitationFixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, CitationFixture(Path(temporary.name), world)

    def restamp(self, fixture: CitationFixture, **kwargs) -> None:
        RESTAMP.restamp(
            fixture.root,
            check=kwargs.pop("check", False),
            allow_closure_shrink=kwargs.pop("allow_closure_shrink", False),
            report=lambda _: None,
        )

    def test_an_extracted_helper_stays_covered_when_it_is_mutated(self) -> None:
        temporary, fixture = self.fixture(INLINE_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )

            fixture.set_world(EXTRACTED_RENDER)
            self.restamp(fixture)
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )
            self.assertIn(
                "WorldFold::render_document",
                json.loads(
                    (fixture.root / VERIFY.CLOSURE_INVENTORY).read_text(
                        encoding="utf-8"
                    )
                )["domains"]["1"],
            )

            mutated = EXTRACTED_RENDER.replace("dig_in.min(2)", "dig_in.min(1)")
            self.assertNotEqual(mutated, EXTRACTED_RENDER)
            fixture.set_world(mutated)

            marker = fixture.marker_of(1)
            envelope = VERIFY.Span(
                int(marker.group("start")), int(marker.group("end"))
            )
            self.assertEqual(
                marker.group("sha256"),
                VERIFY.normalized_span_hash(mutated, envelope),
            )

            with self.assertRaisesRegex(
                VERIFY.VerificationError, "authority closure digest changed"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_mutating_a_function_outside_the_closure_still_passes(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )
            marker = fixture.marker_of(1)
            for original, replacement in (
                ("self.value += 7;", "self.value += 8;"),
                ("self.value += 11;", "self.value += 12;"),
                (INLINE_RENDER[:0], ""),
            ):
                world = EXTRACTED_RENDER.replace(original, replacement)
                fixture.set_world(world)
                index = VERIFY.index_items(world)
                self.assertEqual(
                    marker.group("closure"),
                    VERIFY.closure_digest(
                        index, fixture.closure_of(("fold_render",))
                    ),
                    f"editing `{original}` changed d1's CLOSURE digest",
                )
                self.assertEqual(
                    VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                    (1, 0, 0, 0),
                    f"editing `{original}` must not fail d1's citation",
                )

            shifted = "// an insertion above everything\n" + EXTRACTED_RENDER
            fixture.set_world(shifted)
            self.assertEqual(
                marker.group("closure"),
                VERIFY.closure_digest(
                    VERIFY.index_items(shifted),
                    fixture.closure_of(("fold_render",)),
                ),
                "a line shift must not change a position-independent digest",
            )

    def test_a_helper_that_leaves_the_file_is_named_not_swallowed(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            fixture.set_world(MOVED_OUT_OF_FILE)

            with self.assertRaises(VERIFY.VerificationError) as raised:
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())
            message = str(raised.exception)
            self.assertIn("closure membership changed", message)
            self.assertIn("WorldFold::render_document", message)

            with self.assertRaises(SystemExit) as exited:
                self.restamp(fixture)
            self.assertIn("SHRANK", str(exited.exception))
            self.assertIn("WorldFold::render_document", str(exited.exception))

            self.restamp(fixture, allow_closure_shrink=True)
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )

    def test_a_missing_inventory_fails_closed(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            (fixture.root / VERIFY.CLOSURE_INVENTORY).unlink()
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "closure inventory .* is missing"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_an_inventory_that_omits_a_domain_fails_closed(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            fixture.write_inventory({})
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "no closure membership recorded"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_marker_without_a_closure_digest_is_not_a_marker(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            module = fixture.wire_dir / "d1.rs"
            source = module.read_text(encoding="utf-8")
            module.write_text(
                re.sub(r" closure=[0-9a-f]{64}", "", source),
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "expected one marker, found 0"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())


AMBIGUOUS_RECEIVER = """\
struct Row;
struct Other;

impl Row {
    fn to_json(&self) -> u32 {
        1
    }
}

impl Other {
    fn to_json(&self) -> u32 {
        2
    }
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_amb(),
            _ => (),
        }
    }

    fn fold_amb(&mut self) {
        let row = Row;
        self.out = row.to_json();
    }
}
"""


class ReceiverResolutionTests(unittest.TestCase):
    """A call whose receiver type is not written must never be guessed."""

    def fixture(
        self, world: str
    ) -> tuple[tempfile.TemporaryDirectory[str], CitationFixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, CitationFixture(Path(temporary.name), world)

    def test_an_ambiguous_receiver_refuses_rather_than_undercovers(self) -> None:
        temporary, fixture = self.fixture(AMBIGUOUS_RECEIVER)
        with temporary:
            with self.assertRaises(VERIFY.VerificationError) as raised:
                fixture.closure_of(("fold_amb",))
            message = str(raised.exception)
            self.assertIn("Row::to_json", message)
            self.assertIn("Other::to_json", message)
            self.assertIn("@fold-callee", message)

    def test_a_declared_receiver_covers_that_one_and_no_other(self) -> None:
        temporary, fixture = self.fixture(AMBIGUOUS_RECEIVER)
        with temporary:
            fixture.set_world(
                AMBIGUOUS_RECEIVER.replace(
                    "        self.out = row.to_json();",
                    "        // @fold-callee Row::to_json\n"
                    "        self.out = row.to_json();",
                )
            )
            members = fixture.closure_of(("fold_amb",))
            self.assertIn("Row::to_json", members)
            self.assertNotIn("Other::to_json", members)

    def test_an_external_receiver_declaration_covers_neither(self) -> None:
        temporary, fixture = self.fixture(AMBIGUOUS_RECEIVER)
        with temporary:
            fixture.set_world(
                AMBIGUOUS_RECEIVER.replace(
                    "        self.out = row.to_json();",
                    "        // @fold-callee-external to_json\n"
                    "        self.out = row.to_json();",
                )
            )
            self.assertEqual(
                fixture.closure_of(("fold_amb",)), {"WorldFold::fold_amb", "Row"}
            )

    def test_a_declaration_naming_no_local_function_is_rejected(self) -> None:
        temporary, fixture = self.fixture(AMBIGUOUS_RECEIVER)
        with temporary:
            fixture.set_world(
                AMBIGUOUS_RECEIVER.replace(
                    "        self.out = row.to_json();",
                    "        // @fold-callee Absent::to_json\n"
                    "        self.out = row.to_json();",
                )
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "names no function defined in this file"
            ):
                fixture.closure_of(("fold_amb",))

    def test_a_single_candidate_needs_no_declaration(self) -> None:
        temporary, fixture = self.fixture(
            AMBIGUOUS_RECEIVER.replace(
                """
impl Other {
    fn to_json(&self) -> u32 {
        2
    }
}
""",
                "",
            )
        )
        with temporary:
            self.assertEqual(
                fixture.closure_of(("fold_amb",)),
                {"WorldFold::fold_amb", "Row::to_json", "Row"},
            )

    def test_a_function_reached_as_a_value_is_covered(self) -> None:
        temporary, fixture = self.fixture(
            """\
struct Row;

impl Row {
    fn join_key(&self) -> u32 {
        self.id + 1
    }
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_value_ref(),
            _ => (),
        }
    }

    fn fold_value_ref(&mut self) {
        self.out = self.rows.iter().filter_map(Row::join_key).collect();
    }
}
"""
        )
        with temporary:
            self.assertEqual(
                fixture.closure_of(("fold_value_ref",)),
                {"WorldFold::fold_value_ref", "Row::join_key", "Row"},
            )

    def test_a_field_access_is_not_read_as_a_method_call(self) -> None:
        temporary, fixture = self.fixture(
            """\
struct A;
struct B;

impl A {
    fn value(&self) -> u32 {
        1
    }
}

impl B {
    fn value(&self) -> u32 {
        2
    }
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_field(),
            _ => (),
        }
    }

    fn fold_field(&mut self) {
        self.out = self.row.value;
    }
}
"""
        )
        with temporary:
            self.assertEqual(
                fixture.closure_of(("fold_field",)), {"WorldFold::fold_field"}
            )

    def test_a_call_through_a_free_function_is_followed(self) -> None:
        temporary, fixture = self.fixture(
            """\
fn shape_row(value: u32) -> u32 {
    deeper(value)
}

fn deeper(value: u32) -> u32 {
    value + 1
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_free(),
            _ => (),
        }
    }

    fn fold_free(&mut self) {
        self.out = shape_row(self.raw);
    }
}
"""
        )
        with temporary:
            self.assertEqual(
                fixture.closure_of(("fold_free",)),
                {"WorldFold::fold_free", "shape_row", "deeper"},
            )


class ScannerRefusalTests(unittest.TestCase):
    """Where the walk cannot see, it must say so rather than cover less."""

    def fixture(
        self, world: str
    ) -> tuple[tempfile.TemporaryDirectory[str], CitationFixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, CitationFixture(Path(temporary.name), world)

    def test_a_locally_defined_macro_stops_the_gate(self) -> None:
        temporary, fixture = self.fixture(
            "macro_rules! shape {\n    () => {};\n}\n" + INLINE_RENDER
        )
        with temporary:
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "defines macro"
            ):
                VERIFY.index_items(fixture.world)

    def test_an_unparsed_item_scope_stops_the_gate(self) -> None:
        world = "impl Weird for {\n    fn helper() {}\n}\n" + INLINE_RENDER
        temporary, fixture = self.fixture(world)
        with temporary:
            broken = VERIFY.SCOPE_RE
            try:
                VERIFY.SCOPE_RE = re.compile(r"(?!x)x")
                with self.assertRaisesRegex(
                    VERIFY.VerificationError, "unrecognized item-scope header"
                ):
                    VERIFY.index_items(world)
            finally:
                VERIFY.SCOPE_RE = broken

    def test_an_attribute_is_inside_the_function_it_selects(self) -> None:
        world = """\
impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_gated(),
            _ => (),
        }
    }

    #[cfg(not(test))]
    #[inline(always)]
    fn fold_gated(&mut self) {
        self.value += 1;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            members = fixture.closure_of(("fold_gated",))
            before = VERIFY.closure_digest(index, members)
            flipped = world.replace("#[cfg(not(test))]", "#[cfg(test)]")
            self.assertNotEqual(
                before,
                VERIFY.closure_digest(VERIFY.index_items(flipped), members),
                "changing which body compiles must change the closure digest",
            )

    def test_a_qualified_path_can_name_a_constant_not_only_a_function(
        self,
    ) -> None:
        world = """\
const FLAG: u32 = 1;
const FLAG_OTHER: u32 = 2;

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.value = crate::FLAG;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            self.assertIn("FLAG", fixture.closure_of(("fold_x",)))

    def test_a_const_generic_default_does_not_truncate_a_struct(self) -> None:
        world = """\
struct Row<const N: usize = { 1 }> {
    width: u8,
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        let _ = Row { width: 0 };
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            span = index.items["Row"].span
            covered = "\n".join(world.splitlines()[span.start - 1 : span.end])
            self.assertIn("width: u8", covered, covered)
            members = fixture.closure_of(("fold_x",))
            before = VERIFY.closure_digest(index, members)
            widened = world.replace("width: u8,", "width: u16,")
            self.assertNotEqual(
                before, VERIFY.closure_digest(VERIFY.index_items(widened), members)
            )

    def test_a_shift_in_a_const_generic_default_does_not_delete_the_struct(
        self,
    ) -> None:
        world = """\
struct Row<const N: usize = { 1 << 2 }> {
    width: u8,
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) -> Vec<Vec<u32>> {
        let _ = Row { width: 0 };
        Vec::new()
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            self.assertIn("Row", index.items)
            span = index.items["Row"].span
            covered = "\n".join(world.splitlines()[span.start - 1 : span.end])
            self.assertIn("width: u8", covered, covered)
            self.assertEqual(index.items["WorldFold::fold_x"].span.end, 16)

    def test_an_attribute_survives_a_blank_line_above_its_item(self) -> None:
        world = """\
impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    #[cfg(not(test))]

    // a comment between the selector and the item
    fn fold_x(&mut self) {
        self.value += 1;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            span = index.items["WorldFold::fold_x"].span
            covered = "\n".join(world.splitlines()[span.start - 1 : span.end])
            self.assertIn("#[cfg(not(test))]", covered, covered)

    def test_a_second_item_on_one_line_stops_the_gate(self) -> None:
        world = INLINE_RENDER.replace(
            "    fn fold_other(&mut self) {\n        self.value += 7;\n    }",
            "    fn fold_other(&mut self) { self.value += 7; } fn extra() {}",
        )
        temporary, fixture = self.fixture(world)
        with temporary:
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "declare more than one item"
            ):
                VERIFY.index_items(world)

    def test_a_declaration_naming_a_constant_is_rejected(self) -> None:
        world = AMBIGUOUS_RECEIVER.replace(
            "struct Row;",
            "struct Row;\n\nimpl Marker {\n}\n\nstruct Marker;",
        ).replace(
            "        self.out = row.to_json();",
            "        // @fold-callee Marker::to_json\n"
            "        self.out = row.to_json();",
        )
        temporary, fixture = self.fixture(
            world.replace("struct Marker;", "struct Marker;\nconst to_json: u32 = 1;")
        )
        with temporary:
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "names no function defined in this file"
            ):
                fixture.closure_of(("fold_amb",))

    def test_a_fn_typed_constant_carries_the_function_it_points_at(self) -> None:
        world = """\
fn helper(value: u32) -> u32 {
    value + 1
}

const HANDLER: fn(u32) -> u32 = helper;

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.value = HANDLER(self.raw);
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            members = fixture.closure_of(("fold_x",))
            self.assertIn("HANDLER", members)
            self.assertIn("helper", members)

    def test_an_alias_through_a_module_path_names_the_type(self) -> None:
        world = """\
struct Row;

type Alias = some_module::Row;

impl Row {
    fn helper(value: u32) -> u32 {
        value + 1
    }
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.value = Alias::helper(self.raw);
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            self.assertIn("Row::helper", fixture.closure_of(("fold_x",)))

    def test_a_wrapped_impl_header_still_owns_its_methods(self) -> None:
        world = """\
struct Projection;
struct Row;

impl<'a> From<&'a Row>
    for Projection
where
    Row: Sized,
{
    fn from(row: &'a Row) -> Self {
        Projection
    }
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.value += 1;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            self.assertIn("Projection::from", index.items)
            self.assertNotIn("from", index.items)

    def test_whitespace_does_not_smuggle_a_macro_past_the_tripwire(self) -> None:
        temporary, fixture = self.fixture(
            "macro_rules ! shape {\n    () => {};\n}\n" + INLINE_RENDER
        )
        with temporary:
            with self.assertRaisesRegex(VERIFY.VerificationError, "defines macro"):
                VERIFY.index_items(fixture.world)

    def test_an_unsafe_impl_is_a_scope_not_an_unrecognized_header(self) -> None:
        world = """\
struct Marker;

unsafe impl Send for Marker {
    fn helper(&self) -> u32 {
        1
    }
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.value += 1;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            self.assertIn("Marker::helper", index.items)
            self.assertNotIn("helper", index.items)

    def test_a_wrapped_return_type_is_not_mistaken_for_a_scope(self) -> None:
        world = """\
impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn rows(&self) ->
        impl Iterator<Item = u32> + '_
    {
        fn nested_helper(value: u32) -> u32 {
            value + 1
        }
        self.values.iter().copied().map(nested_helper)
    }

    fn fold_x(&mut self) {
        self.value += 1;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            self.assertIn("WorldFold::fold_x", index.items)
            self.assertIn("WorldFold::rows", index.items)
            self.assertIn("WorldFold::nested_helper", index.items)
            self.assertNotIn("Iterator::nested_helper", index.items)

    def test_an_array_type_in_a_signature_does_not_drop_the_function(self) -> None:
        world = """\
impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn helper(&self, values: [i32; 5]) -> i32 {
        values[0]
    }

    fn fold_x(&mut self) {
        self.value = self.helper([1, 2, 3, 4, 5]);
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            self.assertIn("WorldFold::helper", index.items)
            self.assertIn("WorldFold::helper", fixture.closure_of(("fold_x",)))

    def test_a_braced_initializer_does_not_truncate_a_constant(self) -> None:
        world = """\
const A: i32 = 1;
const B: i32 = 2;
const PICK: i32 = if cfg!(feature = "x") {
    A
} else {
    B
};

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.value = PICK;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            span = index.items["PICK"].span
            covered = "\n".join(world.splitlines()[span.start - 1 : span.end])
            self.assertIn("    B", covered, covered)
            members = fixture.closure_of(("fold_x",))
            before = VERIFY.closure_digest(index, members)
            flipped = world.replace("} else {\n    B\n};", "} else {\n    A\n};")
            self.assertNotEqual(flipped, world)
            self.assertNotEqual(
                before, VERIFY.closure_digest(VERIFY.index_items(flipped), members)
            )

    def test_an_array_type_does_not_truncate_a_static(self) -> None:
        world = """\
static MAGIC: [u8; 4] = [
    b'D', b'E', b'L', b'2',
];

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.magic = MAGIC;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            span = index.items["MAGIC"].span
            covered = "\n".join(world.splitlines()[span.start - 1 : span.end])
            self.assertIn("b'2'", covered, covered)
            members = fixture.closure_of(("fold_x",))
            before = VERIFY.closure_digest(index, members)
            flipped = world.replace("b'2',", "b'3',")
            self.assertNotEqual(flipped, world)
            self.assertNotEqual(
                before, VERIFY.closure_digest(VERIFY.index_items(flipped), members)
            )

    def test_a_local_type_alias_resolves_to_the_underlying_method(self) -> None:
        world = """\
struct Row;

type Alias = Row;

impl Row {
    fn helper(value: u32) -> u32 {
        value + 1
    }
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.value = Alias::helper(self.raw);
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            self.assertIn("Row::helper", fixture.closure_of(("fold_x",)))

    def test_a_value_item_is_never_a_candidate_for_a_method_call(self) -> None:
        world = """\
struct Row;

const render: u32 = 1;

impl Row {
    fn render(&self) -> u32 {
        2
    }
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        let row = Row;
        self.value = row.render();
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            self.assertIn("Row::render", fixture.closure_of(("fold_x",)))

    def test_a_bodyless_trait_method_does_not_borrow_a_later_body(self) -> None:
        world = """\
trait Shape {
    fn area(&self) -> u32;
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.value += 1;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            self.assertNotIn("Shape::area", index.items)
            self.assertEqual(
                (index.items["WorldFold::fold_x"].span.start,
                 index.items["WorldFold::fold_x"].span.end),
                (13, 15),
            )

    def test_an_annotation_inside_a_string_literal_is_not_authority(self) -> None:
        temporary, fixture = self.fixture(
            AMBIGUOUS_RECEIVER.replace(
                "        self.out = row.to_json();",
                '        let _ = "// @fold-callee-external to_json";\n'
                "        self.out = row.to_json();",
            )
        )
        with temporary:
            with self.assertRaisesRegex(VERIFY.VerificationError, "@fold-callee"):
                fixture.closure_of(("fold_amb",))

    def test_a_field_read_is_not_a_call_even_with_a_brace_after_it(self) -> None:
        temporary, fixture = self.fixture(
            AMBIGUOUS_RECEIVER.replace(
                "        self.out = row.to_json();",
                "        if row.to_json {\n            self.out = row.to_json[0];\n        }",
            )
        )
        with temporary:
            self.assertEqual(
                fixture.closure_of(("fold_amb",)), {"WorldFold::fold_amb", "Row"}
            )

    def test_a_declaration_does_not_absolve_a_second_call_elsewhere(self) -> None:
        world = AMBIGUOUS_RECEIVER.replace(
            "        self.out = row.to_json();",
            "        // @fold-callee Row::to_json\n"
            "        self.out = row.to_json();\n"
            "        let other = Other;\n"
            "        self.extra = other.to_json();",
        )
        temporary, fixture = self.fixture(world)
        with temporary:
            with self.assertRaises(VERIFY.VerificationError) as raised:
                fixture.closure_of(("fold_amb",))
            self.assertIn("Other::to_json", str(raised.exception))

    def test_a_stale_receiver_declaration_is_rejected(self) -> None:
        temporary, fixture = self.fixture(
            AMBIGUOUS_RECEIVER.replace(
                "        self.out = row.to_json();",
                "        // @fold-callee Row::to_json\n        self.out = row.id;",
            )
        )
        with temporary:
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "govern no call on their own line"
            ):
                fixture.closure_of(("fold_amb",))

    def test_a_crate_qualified_call_is_not_resolved_to_a_scope_named_crate(
        self,
    ) -> None:
        temporary, fixture = self.fixture(
            AMBIGUOUS_RECEIVER.replace(
                "        self.out = row.to_json();",
                "        self.out = crate::to_json();",
            )
        )
        with temporary:
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "@fold-callee"
            ):
                fixture.closure_of(("fold_amb",))


class ValueItemCoverageTests(unittest.TestCase):
    """A fold executes more than functions."""

    def fixture(
        self, world: str
    ) -> tuple[tempfile.TemporaryDirectory[str], CitationFixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, CitationFixture(Path(temporary.name), world)

    CONSTANT_WORLD = """\
const SUPPLY_GAIN_FLAG: u32 = 1 << 10;

struct Row {
    flags: u32,
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_flags(),
            _ => (),
        }
    }

    fn fold_flags(&mut self) {
        self.gain = self.row.flags & SUPPLY_GAIN_FLAG != 0;
        let _ = Row { flags: 0 };
    }
}
"""

    def test_a_constant_the_fold_reads_is_covered(self) -> None:
        temporary, fixture = self.fixture(self.CONSTANT_WORLD)
        with temporary:
            index = VERIFY.index_items(fixture.world)
            members = fixture.closure_of(("fold_flags",))
            self.assertIn("SUPPLY_GAIN_FLAG", members)
            before = VERIFY.closure_digest(index, members)
            flipped = self.CONSTANT_WORLD.replace("1 << 10", "1 << 9")
            self.assertNotEqual(
                before,
                VERIFY.closure_digest(VERIFY.index_items(flipped), members),
                "re-spelling the constant must change the closure digest",
            )

    def test_a_struct_the_fold_names_is_covered(self) -> None:
        temporary, fixture = self.fixture(self.CONSTANT_WORLD)
        with temporary:
            index = VERIFY.index_items(fixture.world)
            members = fixture.closure_of(("fold_flags",))
            self.assertIn("Row", members)
            before = VERIFY.closure_digest(index, members)
            widened = self.CONSTANT_WORLD.replace("flags: u32,", "flags: u64,")
            self.assertNotEqual(
                before,
                VERIFY.closure_digest(VERIFY.index_items(widened), members),
            )

    def test_the_struct_a_method_belongs_to_is_covered(self) -> None:
        world = """\
struct WorldFold {
    value: u8,
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.value += 1;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            members = fixture.closure_of(("fold_x",))
            self.assertIn("WorldFold", members)
            before = VERIFY.closure_digest(index, members)
            widened = world.replace("    value: u8,", "    value: u16,")
            self.assertNotEqual(
                before, VERIFY.closure_digest(VERIFY.index_items(widened), members)
            )

    def test_a_use_binding_the_fold_executes_is_covered(self) -> None:
        world = """\
use compact_contract::calendar::{date_from_raw, hour_index_from_raw};

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.hour = hour_index_from_raw(self.raw);
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            members = fixture.closure_of(("fold_x",))
            self.assertIn("hour_index_from_raw", members)
            before = VERIFY.closure_digest(index, members)
            repointed = world.replace("use compact_contract::", "use other_crate::")
            self.assertNotEqual(
                before, VERIFY.closure_digest(VERIFY.index_items(repointed), members)
            )

    def test_a_constant_built_from_constants_carries_them(self) -> None:
        world = """\
const A: u32 = 1 << 0;
const B: u32 = 1 << 1;
const KNOWN_FLAGS: u32 = A | B;

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.flags &= KNOWN_FLAGS;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            members = fixture.closure_of(("fold_x",))
            self.assertIn("A", members)
            self.assertIn("B", members)
            before = VERIFY.closure_digest(index, members)
            flipped = world.replace("const B: u32 = 1 << 1;", "const B: u32 = 1 << 2;")
            self.assertNotEqual(
                before, VERIFY.closure_digest(VERIFY.index_items(flipped), members)
            )

    def test_a_type_is_a_leaf_so_one_state_struct_does_not_drag_in_all(
        self,
    ) -> None:
        world = """\
struct Unrelated {
    depth: u8,
}

struct WorldFold {
    other: Unrelated,
}

impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_x(),
            _ => (),
        }
    }

    fn fold_x(&mut self) {
        self.value += 1;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            members = fixture.closure_of(("fold_x",))
            self.assertIn("WorldFold", members)
            self.assertNotIn("Unrelated", members)

    def test_a_constant_inside_a_function_body_is_not_indexed_separately(
        self,
    ) -> None:
        world = """\
impl WorldFold {
    fn fold_domain_data(&mut self, domain: u16) {
        match domain {
            1 => self.fold_a(),
            _ => (),
        }
    }

    fn fold_a(&mut self) {
        const HEADER_LEN: usize = 8;
        self.value += HEADER_LEN;
    }

    fn fold_b(&mut self) {
        const HEADER_LEN: usize = 12;
        self.value += HEADER_LEN;
    }
}
"""
        temporary, fixture = self.fixture(world)
        with temporary:
            index = VERIFY.index_items(world)
            self.assertEqual(
                [i for i in index.items if "HEADER_LEN" in i], [], index.items.keys()
            )
            self.assertEqual(
                fixture.closure_of(("fold_a",)), {"WorldFold::fold_a"}
            )



class InventoryRecordTests(unittest.TestCase):
    """The membership record must not accumulate claims nothing checks."""

    def fixture(
        self, world: str
    ) -> tuple[tempfile.TemporaryDirectory[str], CitationFixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, CitationFixture(Path(temporary.name), world)

    def test_an_inventory_domain_with_no_module_is_refused(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            recorded = dict(fixture.closures)
            recorded["77"] = ["WorldFold::fold_render"]
            fixture.write_inventory(recorded)
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "have no dNN module"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_re_stamp_drops_an_orphaned_inventory_domain(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            recorded = dict(fixture.closures)
            recorded["77"] = ["WorldFold::fold_render"]
            fixture.write_inventory(recorded)
            RESTAMP.restamp(
                fixture.root,
                check=False,
                allow_closure_shrink=True,
                report=lambda _: None,
            )
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )

    def test_a_wrong_shaped_inventory_says_so_instead_of_raising(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            path = fixture.root / VERIFY.CLOSURE_INVENTORY
            for payload, expected in (
                ("[]", "expected a JSON object at the root"),
                ('{"domains": []}', 'expected an object under "domains"'),
                ('{"domains": {"1": null}}', "do not record a list of member names"),
                ('{"domains": {"1": [3]}}', "do not record a list of member names"),
            ):
                path.write_text(payload, encoding="utf-8", newline="\n")
                with self.assertRaisesRegex(VERIFY.VerificationError, expected):
                    VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_corrupt_inventory_stops_the_re_stamper_with_a_message(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            (fixture.root / VERIFY.CLOSURE_INVENTORY).write_text(
                "{not json", encoding="utf-8", newline="\n"
            )
            with self.assertRaises(SystemExit) as exited:
                RESTAMP.restamp(
                    fixture.root,
                    check=False,
                    allow_closure_shrink=False,
                    report=lambda _: None,
                )
            self.assertIn("cannot read", str(exited.exception))


class RestampFidelityTests(unittest.TestCase):
    """What the re-stamper writes must be exactly what the gate reads."""

    def fixture(
        self, world: str
    ) -> tuple[tempfile.TemporaryDirectory[str], CitationFixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, CitationFixture(Path(temporary.name), world)

    def test_restamping_a_drifted_tree_makes_the_gate_pass(self) -> None:
        temporary, fixture = self.fixture(INLINE_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            fixture.set_world("// an insertion above everything\n" + INLINE_RENDER)
            with self.assertRaises(VERIFY.VerificationError):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())
            RESTAMP.restamp(
                fixture.root,
                check=False,
                allow_closure_shrink=False,
                report=lambda _: None,
            )
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )

    def test_restamping_is_idempotent_and_check_reports_a_clean_tree(self) -> None:
        temporary, fixture = self.fixture(INLINE_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            fixture.set_world("// an insertion above everything\n" + INLINE_RENDER)
            RESTAMP.restamp(
                fixture.root,
                check=False,
                allow_closure_shrink=False,
                report=lambda _: None,
            )
            changed, total, inventory_changed = RESTAMP.restamp(
                fixture.root,
                check=True,
                allow_closure_shrink=False,
                report=lambda _: None,
            )
            self.assertEqual((changed, total, inventory_changed), (0, 1, False))

    def test_the_human_citation_moves_with_the_marker(self) -> None:
        temporary, fixture = self.fixture(INLINE_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            fixture.set_world("// an insertion above everything\n" + INLINE_RENDER)
            RESTAMP.restamp(
                fixture.root,
                check=False,
                allow_closure_shrink=False,
                report=lambda _: None,
            )
            source = (fixture.wire_dir / "d1.rs").read_text(encoding="utf-8")
            marker = VERIFY.WORLD_MARKER_RE.search(source)
            citation = VERIFY.gated_citation(marker)
            self.assertEqual(source.count(citation), 1, source)

    def test_check_fails_on_inventory_only_drift(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary, unittest.mock.patch.object(
            VERIFY, "REQUIRED_LEAF_AUTHORITIES", {}
        ):
            fixture.add_module(1, ("fold_render",))
            fixture.write_inventory({"1": ["WorldFold::fold_render"]})
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                self.assertEqual(
                    RESTAMP.main(["--repo-root", str(fixture.root), "--check"]), 1
                )
            self.assertIn("would re-stamp 0 of 1 modules", printed.getvalue())
            self.assertIn("closure inventory", printed.getvalue())
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(RESTAMP.main(["--repo-root", str(fixture.root)]), 0)
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                self.assertEqual(
                    RESTAMP.main(["--repo-root", str(fixture.root), "--check"]), 0
                )
            self.assertNotIn("closure inventory", printed.getvalue())
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )

    def test_a_damaged_citation_is_reported_not_passed_over(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            module = fixture.wire_dir / "d1.rs"
            module.write_text(
                re.sub(
                    r"`world_fold\.rs:\d+`-`:\d+`",
                    "`world_fold.rs:1`-`:2`",
                    module.read_text(encoding="utf-8"),
                ),
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaises(SystemExit) as exited:
                RESTAMP.restamp(
                    fixture.root,
                    check=True,
                    allow_closure_shrink=False,
                    report=lambda _: None,
                )
            self.assertIn("human citation", str(exited.exception))

    def test_an_lf_module_stays_lf(self) -> None:
        temporary, fixture = self.fixture(INLINE_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            fixture.set_world("// an insertion above everything\n" + INLINE_RENDER)
            RESTAMP.restamp(
                fixture.root,
                check=False,
                allow_closure_shrink=False,
                report=lambda _: None,
            )
            raw = (fixture.wire_dir / "d1.rs").read_bytes()
            self.assertEqual(raw.count(b"\r"), 0, raw)

    def test_a_mixed_ending_module_keeps_every_ending_it_had(self) -> None:
        temporary, fixture = self.fixture(INLINE_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            module = fixture.wire_dir / "d1.rs"
            mixed = module.read_text(encoding="utf-8").replace("\n", "\r\n", 1)
            module.write_bytes(mixed.encode("utf-8"))
            fixture.set_world("// an insertion above everything\n" + INLINE_RENDER)
            RESTAMP.restamp(
                fixture.root,
                check=False,
                allow_closure_shrink=False,
                report=lambda _: None,
            )
            raw = module.read_bytes()
            self.assertEqual(raw.count(b"\r\n"), 1, raw)
            self.assertGreater(raw.count(b"\n") - raw.count(b"\r\n"), 0, raw)

    def test_a_crlf_module_is_re_stamped_as_crlf(self) -> None:
        temporary, fixture = self.fixture(INLINE_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            module = fixture.wire_dir / "d1.rs"
            module.write_bytes(
                module.read_text(encoding="utf-8").replace("\n", "\r\n").encode()
            )
            fixture.set_world("// an insertion above everything\n" + INLINE_RENDER)
            RESTAMP.restamp(
                fixture.root,
                check=False,
                allow_closure_shrink=False,
                report=lambda _: None,
            )
            raw = module.read_bytes()
            self.assertGreater(raw.count(b"\r\n"), 0)
            self.assertEqual(raw.count(b"\n"), raw.count(b"\r\n"))

    def test_a_refused_shrink_leaves_the_tree_exactly_as_it_was(self) -> None:
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            module = fixture.wire_dir / "d1.rs"
            inventory = fixture.root / VERIFY.CLOSURE_INVENTORY
            before = (module.read_bytes(), inventory.read_bytes())
            fixture.set_world(MOVED_OUT_OF_FILE)
            with self.assertRaises(SystemExit):
                RESTAMP.restamp(
                    fixture.root,
                    check=False,
                    allow_closure_shrink=False,
                    report=lambda _: None,
                )
            self.assertEqual(
                (module.read_bytes(), inventory.read_bytes()), before
            )

    def test_a_pre_existing_copy_of_the_new_citation_stops_the_re_stamp(self) -> None:
        temporary, fixture = self.fixture(INLINE_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            module = fixture.wire_dir / "d1.rs"
            fixture.set_world("// an insertion above everything\n" + INLINE_RENDER)
            span = VERIFY.function_spans(fixture.world)["fold_render"]
            with module.open("a", encoding="utf-8", newline="\n") as handle:
                handle.write(
                    "// prose already citing "
                    f"`world_fold.rs:{span.start}`-`:{span.end}`\n"
                )
            before = module.read_bytes()
            with self.assertRaises(SystemExit) as exited:
                RESTAMP.restamp(
                    fixture.root,
                    check=False,
                    allow_closure_shrink=False,
                    report=lambda _: None,
                )
            self.assertIn("copies of", str(exited.exception))
            self.assertEqual(module.read_bytes(), before)

    def test_an_authority_that_is_not_the_dispatch_target_is_refused(self) -> None:
        world = INLINE_RENDER.replace(
            "    fn fold_other(&mut self) {",
            "    fn fold_wrapper(&mut self) {\n"
            "        self.fold_render();\n"
            "    }\n"
            "\n"
            "    fn fold_other(&mut self) {",
        )
        temporary, fixture = self.fixture(world)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            module = fixture.wire_dir / "d1.rs"
            module.write_text(
                module.read_text(encoding="utf-8").replace(
                    "functions=fold_render", "functions=fold_wrapper"
                ),
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaises(SystemExit) as exited:
                RESTAMP.restamp(
                    fixture.root,
                    check=False,
                    allow_closure_shrink=False,
                    report=lambda _: None,
                )
            self.assertIn("dispatch target", str(exited.exception))

    def test_a_marker_domain_that_contradicts_the_filename_is_refused(self) -> None:
        temporary, fixture = self.fixture(INLINE_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            module = fixture.wire_dir / "d1.rs"
            module.write_text(
                module.read_text(encoding="utf-8").replace(
                    "domain=1 functions=", "domain=2 functions="
                ),
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaises(SystemExit) as exited:
                RESTAMP.restamp(
                    fixture.root,
                    check=False,
                    allow_closure_shrink=False,
                    report=lambda _: None,
                )
            self.assertIn("does not match the filename domain", str(exited.exception))

    def test_the_re_stamper_never_repoints_an_authority(self) -> None:
        temporary, fixture = self.fixture(INLINE_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            fixture.set_world("// an insertion above everything\n" + INLINE_RENDER)
            RESTAMP.restamp(
                fixture.root,
                check=False,
                allow_closure_shrink=False,
                report=lambda _: None,
            )
            self.assertEqual(
                fixture.marker_of(1).group("functions"), "fold_render"
            )


class VerifyWorldFoldCitationsTests(unittest.TestCase):
    def fixture(self) -> tuple[tempfile.TemporaryDirectory[str], CitationFixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, CitationFixture(Path(temporary.name), WORLD)

    def test_accepts_exact_dispatch_span_and_digest(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self.assertEqual(
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                ),
                (1, 0, 0, 0),
            )

    def test_rejects_line_drift_even_when_function_is_unchanged(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            fixture.world_path.write_text(
                "// inserted before every citation\n" + WORLD,
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "declared lines"
            ):
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                )

    def test_reports_every_drifted_module_not_only_the_first(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            fixture.add_module(39, ("fold_shared",))
            fixture.add_module(40, ("fold_shared",))
            fixture.world_path.write_text(
                "// inserted before every citation\n" + WORLD,
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaises(VERIFY.VerificationError) as raised:
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())
            message = str(raised.exception)
            self.assertIn("3 citation authority failure(s)", message)
            for name in ("d1.rs", "d39.rs", "d40.rs"):
                self.assertIn(name, message)

    def test_lane_authority_is_still_collected_from_a_drifted_module(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(51, ("fold_lane",))
            key = fixture.add_lane_authority(
                51,
                "extractor/native/lanes/example/original.rs",
                "decode_payload",
                "pub fn decode_payload(payload: &[u8]) -> bool {\n"
                "    !payload.is_empty()\n"
                "}\n",
            )
            fixture.world_path.write_text(
                "// inserted before every citation\n" + WORLD,
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaises(VERIFY.VerificationError) as raised:
                VERIFY.verify_repo(fixture.root, required_lane_authorities=(key,))
            message = str(raised.exception)
            self.assertIn("d51.rs: declared lines", message)
            self.assertNotIn("lane authority marker set drifted", message)

    def test_rejects_same_line_count_semantic_drift_by_digest(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            fixture.world_path.write_text(
                WORLD.replace("self.value += 1;", "self.value += 9;"),
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "authority digest changed"
            ):
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                )

    def test_shared_dispatch_function_is_valid_for_both_domains(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(39, ("fold_shared",))
            fixture.add_module(40, ("fold_shared",))
            self.assertEqual(
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                ),
                (2, 0, 0, 0),
            )

    def test_multi_function_authority_pins_the_whole_envelope(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(
                67, ("fold_multi", "fold_multi_helper")
            )
            self.assertEqual(
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                ),
                (1, 0, 0, 0),
            )

    def test_missing_marker_fails_closed(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",), include_marker=False)
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "expected one marker, found 0"
            ):
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=()
                )

    def test_lane_authority_digest_reads_the_original_function(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(51, ("fold_lane",))
            key = fixture.add_lane_authority(
                51,
                "extractor/native/lanes/example/original.rs",
                "decode_payload",
                "pub fn decode_payload(payload: &[u8]) -> bool {\n"
                "    !payload.is_empty()\n"
                "}\n",
            )
            self.assertEqual(
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=(key,)
                ),
                (1, 1, 0, 0),
            )
            authority = fixture.root / key[1]
            authority.write_text(
                "pub fn decode_payload(payload: &[u8]) -> bool {\n"
                "    payload.is_empty()\n"
                "}\n",
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "lane authority digest changed"
            ):
                VERIFY.verify_repo(
                    fixture.root, required_lane_authorities=(key,)
                )


    def _append_comment(self, fixture: CitationFixture, domain: int, text: str) -> None:
        module = fixture.wire_dir / f"d{domain}.rs"
        with module.open("a", encoding="utf-8", newline="\n") as handle:
            handle.write(text)

    def _write_allowance(
        self,
        fixture: CitationFixture,
        modules: dict,
        documents: dict | None = None,
    ) -> None:
        path = fixture.root / VERIFY.PROSE_ALLOWANCE
        path.parent.mkdir(parents=True, exist_ok=True)
        payload: dict = {"modules": modules}
        if documents is not None:
            payload["documents"] = documents
        path.write_text(json.dumps(payload), encoding="utf-8", newline="\n")

    def _write_document(
        self,
        fixture: CitationFixture,
        name: str,
        text: str,
        root_index: int = 0,
    ) -> str:
        path = fixture.root / VERIFY.DOCUMENT_ROOTS[root_index] / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="\n")
        return path.relative_to(fixture.root).as_posix()

    def test_rejects_a_new_free_text_line_number(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._append_comment(fixture, 1, "// see `world_fold.rs:123`\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:123` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_grandfathered_line_number_is_allowed_exactly_once(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._append_comment(fixture, 1, "// see `world_fold.rs:123`\n")
            self._write_allowance(fixture, {"d1.rs": [123]})
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 1, 0),
            )
            self._append_comment(fixture, 1, "// and again `world_fold.rs:123`\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:123` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_the_allowance_may_shrink_but_swapping_fails(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._write_allowance(fixture, {"d1.rs": [123, 456]})
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )
            self._append_comment(fixture, 1, "// see `world_fold.rs:789`\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:789` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_the_gated_citation_is_never_counted_as_free_text(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )

    def test_a_missing_allowance_file_grandfathers_nothing(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._append_comment(fixture, 1, "// see `world_fold.rs:123`\n")
            self.assertFalse((fixture.root / VERIFY.PROSE_ALLOWANCE).exists())
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:123` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_second_copy_of_the_gated_citation_is_not_a_second_exemption(
        self,
    ) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            module = fixture.wire_dir / "d1.rs"
            source = module.read_text(encoding="utf-8")
            citation = re.search(
                r"`world_fold\.rs:\d+`-`:\d+`", source
            ).group(0)
            self._append_comment(fixture, 1, f"// a second copy: {citation}\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError,
                "expected exactly one human authority citation",
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

            marker = VERIFY.WORLD_MARKER_RE.search(
                module.read_text(encoding="utf-8")
            )
            found = VERIFY.ungated_prose_references(
                module.read_text(encoding="utf-8"), marker
            )
            self.assertEqual(len(found), 1, found)

    def test_rejects_a_new_document_line_number(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._write_document(
                fixture, "DESIGN.md", "the fold refuses it at `world_fold.rs:4242`.\n"
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError,
                r"DESIGN\.md: `world_fold\.rs:4242` is a free-text",
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_grandfathered_document_line_number_is_allowed_exactly_once(
        self,
    ) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            relative = self._write_document(
                fixture, "DESIGN.md", "see `world_fold.rs:4242`.\n"
            )
            self._write_allowance(fixture, {}, {relative: [4242]})
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 1),
            )
            path = fixture.root / relative
            path.write_text(
                "see `world_fold.rs:4242` and again `world_fold.rs:4242`.\n",
                encoding="utf-8",
                newline="\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:4242` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_document_budget_does_not_carry_across_documents(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            allowed = self._write_document(
                fixture, "DESIGN.md", "see `world_fold.rs:4242`.\n"
            )
            self._write_allowance(fixture, {}, {allowed: [4242]})
            self._write_document(
                fixture, "nested/OTHER.md", "also `world_fold.rs:4242`.\n"
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError,
                r"nested/OTHER\.md: `world_fold\.rs:4242` is a free-text",
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_missing_allowance_grandfathers_no_document_reference(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._write_document(
                fixture, "DESIGN.md", "see `world_fold.rs:4242`.\n"
            )
            self.assertFalse((fixture.root / VERIFY.PROSE_ALLOWANCE).exists())
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:4242` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_every_document_root_is_scanned(self) -> None:
        for index in range(len(VERIFY.DOCUMENT_ROOTS)):
            temporary, fixture = self.fixture()
            with temporary:
                fixture.add_module(1, ("fold_one",))
                self._write_document(
                    fixture,
                    "DESIGN.md",
                    "see `world_fold.rs:4242`.\n",
                    root_index=index,
                )
                with self.assertRaisesRegex(
                    VERIFY.VerificationError,
                    r"`world_fold\.rs:4242` is a free-text",
                    msg=f"document root {VERIFY.DOCUMENT_ROOTS[index]} is not scanned",
                ):
                    VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_grandfathered_document_outside_every_root_fails(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            stray = fixture.root / "somewhere_else" / "DESIGN.md"
            stray.parent.mkdir(parents=True, exist_ok=True)
            stray.write_text(
                "see `world_fold.rs:4242`.\n", encoding="utf-8", newline="\n"
            )
            relative = stray.relative_to(fixture.root).as_posix()
            self._write_allowance(fixture, {}, {relative: [4242]})
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "grandfathered but were not scanned"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_a_deleted_grandfathered_document_is_not_an_error(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._write_allowance(
                fixture, {}, {"extractor/documents/GONE.md": [4242]}
            )
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )

    def test_a_named_item_citation_in_a_document_needs_no_amnesty(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            self._write_document(
                fixture,
                "DESIGN.md",
                "the refusal lives in `world_fold.rs::fold_one`.\n",
            )
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )

    def test_a_corrupt_allowance_file_fails_rather_than_opens_the_gate(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            path = fixture.root / VERIFY.PROSE_ALLOWANCE
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("{not json", encoding="utf-8", newline="\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "cannot read the prose-reference allowance"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())


class LeafFixture:
    """A synthetic repository with one shared grammar crate and two consumers.

    The shape the leaf authority exists for: a walk that lives in a crate, and
    sites in other crates that execute it and hold their own semantics over it.
    Everything here sits under ``extractor/native`` because that is where the
    gate scans for a stray marker, and a fixture outside it would be testing a
    path the gate never walks.
    """

    CRATE = "grammar_leaf"
    ROOT = "extractor/native/grammar_leaf"
    CONSUMERS = (
        "extractor/native/reader_one/src/lib.rs",
        "extractor/native/reader_two/src/lib.rs",
    )
    WALK = "pub const ROW_LEN: usize = 16;\npub fn walk() -> usize { ROW_LEN }\n"

    def __init__(
        self,
        root: Path,
        *,
        newline: str = "\n",
        consumers: tuple[str, ...] | None = None,
    ) -> None:
        self.root = root
        self.newline = newline
        self.const_only: dict[str, tuple[str, ...]] = {}
        if consumers is not None:
            self.CONSUMERS = consumers
        self.write(f"{self.ROOT}/Cargo.toml", '[package]\nname = "grammar_leaf"\n')
        self.write(f"{self.ROOT}/src/lib.rs", self.WALK)
        marker = (
            "// @leaf-source-authority crate=grammar_leaf "
            "root=extractor/native/grammar_leaf files=0 sha256=0\n"
        )
        for consumer in self.CONSUMERS:
            package_dir = PurePosixPath(consumer).parent
            while package_dir.name in ("src", "tests", "benches") or (
                package_dir.parent.name in ("src",)
            ):
                package_dir = package_dir.parent
            package = package_dir.name
            to_leaf = os.path.relpath(self.ROOT, package_dir).replace("\\", "/")
            self.write(
                f"{package_dir}/Cargo.toml",
                f'[package]\nname = "{package}"\n\n'
                "[dependencies]\n"
                f'grammar_leaf = {{ path = "{to_leaf}" }}\n',
            )
            if (self.root / consumer).is_file():
                existing = self.read(consumer)
                self.write(consumer, existing + marker)
                continue
            self.write(
                consumer,
                "// a consumer of the shared walk\n"
                + marker
                + "pub fn read() -> usize { grammar_leaf::walk() }\n",
            )

    @property
    def registry(self) -> dict[str, "VERIFY.LeafAuthority"]:
        return {
            self.CRATE: VERIFY.LeafAuthority(
                root=self.ROOT,
                consumers=self.CONSUMERS,
                const_only=dict(self.const_only),
            )
        }

    def write(self, relative: str, text: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline=self.newline)

    def read(self, relative: str) -> str:
        return (self.root / relative).read_text(encoding="utf-8")

    def restamp(self, *, allow_leaf_shrink: bool = False) -> tuple[int, int, bool]:
        return RESTAMP.restamp_leaves(
            self.root,
            check=False,
            allow_leaf_shrink=allow_leaf_shrink,
            report=lambda _: None,
            registry=self.registry,
        )

    def verify(self) -> tuple[int, int]:
        return VERIFY.verify_leaf_authorities(self.root, self.registry)

    def stamped(self) -> "LeafFixture":
        self.restamp()
        return self


class LeafSourceAuthorityTests(unittest.TestCase):
    """A shared grammar crate is covered at every site that executes it.

    The hole these close: a closure follows calls inside ONE file, so the day
    domain 13's walk moved into ``d13_grammar`` its authority closure shrank
    from 80 names to 73 and every digest in the tree stopped covering the
    grammar the fold executes.  Compilation catches API drift across the crate
    boundary; nothing caught a changed section length, flag mask or field
    offset inside the leaf.
    """

    def fixture(
        self, **kwargs
    ) -> tuple[tempfile.TemporaryDirectory[str], LeafFixture]:
        temporary = tempfile.TemporaryDirectory()
        return temporary, LeafFixture(Path(temporary.name), **kwargs)

    def test_editing_the_leafs_walk_turns_every_consumer_red(self) -> None:
        """THE PROPERTY. Production change that fails this: any edit to the
        shared grammar's source without a re-attestation at the consumers."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            self.assertEqual(fixture.verify(), (1, 2))
            fixture.write(
                f"{fixture.ROOT}/src/lib.rs",
                fixture.WALK.replace("16", "20"),
            )
            with self.assertRaises(VERIFY.VerificationError) as raised:
                fixture.verify()
            message = str(raised.exception)
            self.assertIn("2 leaf source authority failure(s)", message)
            for consumer in fixture.CONSUMERS:
                self.assertIn(f"{consumer}: grammar_leaf leaf authority digest", message)
            fixture.restamp()
            self.assertEqual(fixture.verify(), (1, 2))

    def test_an_edit_outside_every_leaf_stays_green(self) -> None:
        """A gate that fires on unrelated edits is one people re-stamp without
        reading, which is the same as no gate."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/reader_three/src/lib.rs",
                "pub fn unrelated() -> usize { 1 }\n",
            )
            fixture.write(
                fixture.CONSUMERS[0],
                fixture.read(fixture.CONSUMERS[0]) + "// a later comment\n",
            )
            self.assertEqual(fixture.verify(), (1, 2))

    def test_a_missing_marker_at_a_registered_consumer_is_refused(self) -> None:
        """Deleting the marker must not be the way to silence the gate."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                fixture.CONSUMERS[1],
                "pub fn read() -> usize { grammar_leaf::walk() }\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError,
                "expected exactly one grammar_leaf leaf authority marker, found 0",
            ):
                fixture.verify()

    def test_a_marker_at_an_unregistered_site_is_refused(self) -> None:
        """A marker nothing is required to keep current still READS as an
        attestation; a stale one somewhere unlisted is worse than none."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            marker = [
                line
                for line in fixture.read(fixture.CONSUMERS[0]).splitlines()
                if line.startswith("// @leaf-source-authority")
            ][0]
            fixture.write(
                "extractor/native/reader_three/src/lib.rs",
                f"{marker}\npub fn read() -> usize {{ 1 }}\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "a site the registry does not list"
            ):
                fixture.verify()

    def test_a_marker_for_an_unregistered_crate_is_refused(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/reader_three/src/lib.rs",
                "// @leaf-source-authority crate=other_leaf "
                "root=extractor/native/other_leaf files=1 "
                f"sha256={'0' * 64}\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "unregistered crate other_leaf"
            ):
                fixture.verify()

    def test_two_markers_for_one_crate_in_one_file_are_refused(self) -> None:
        """Two attestations in a file are one somebody can update and one that
        rots, and the gate would be satisfied by the fresh one."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            source = fixture.read(fixture.CONSUMERS[0])
            marker = [
                line
                for line in source.splitlines()
                if line.startswith("// @leaf-source-authority")
            ][0]
            fixture.write(fixture.CONSUMERS[0], f"{source}{marker}\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError,
                "expected exactly one grammar_leaf leaf authority marker, found 2",
            ):
                fixture.verify()

    def test_a_broken_marker_line_is_not_read_as_an_absent_one(self) -> None:
        """A truncated or hand-edited marker must fail as a broken
        attestation. Read as absent, it would report a site as un-attested at
        best and say nothing at all where the crate is attested elsewhere."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            source = fixture.read(fixture.CONSUMERS[0])
            fixture.write(
                fixture.CONSUMERS[0],
                source.replace(" sha256=", " sha256 "),
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "do not parse as one"
            ):
                fixture.verify()

    def test_an_indented_marker_is_read_and_its_indent_kept(self) -> None:
        """A marker belongs beside the code it attests, and that code is often
        inside an `impl` block -- `fold_tech` is. Refusing an indented marker
        would push it somewhere else in the file; re-writing it at column 0
        would leave a comment sticking out of an indented body."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            source = fixture.read(fixture.CONSUMERS[0])
            fixture.write(
                fixture.CONSUMERS[0],
                source.replace(
                    "// @leaf-source-authority", "    // @leaf-source-authority"
                ),
            )
            self.assertEqual(fixture.verify(), (1, 2))
            fixture.write(f"{fixture.ROOT}/src/lib.rs", fixture.WALK.replace("16", "20"))
            fixture.restamp()
            self.assertEqual(fixture.verify(), (1, 2))
            self.assertIn(
                "    // @leaf-source-authority", fixture.read(fixture.CONSUMERS[0])
            )

    def test_prose_about_the_marker_is_not_a_marker(self) -> None:
        """The design documents, this gate's own Rust test and the modules that
        carry a marker all write the token inside a sentence. Treating a
        sentence as a failed marker makes the gate impossible to document."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/reader_three/src/lib.rs",
                "//! Every consumer carries a `@leaf-source-authority` marker.\n"
                "// see @leaf-source-authority in the sibling modules\n",
            )
            self.assertEqual(fixture.verify(), (1, 2))

    def test_a_file_added_to_the_leaf_is_attested(self) -> None:
        """A second module in the crate is more grammar, not less. The digest
        must move and the membership record must gain it."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(f"{fixture.ROOT}/src/encode.rs", "pub fn write() {}\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "attested file membership changed"
            ):
                fixture.verify()
            fixture.restamp()
            self.assertEqual(fixture.verify(), (1, 2))
            recorded = VERIFY.load_leaf_inventory(fixture.root)["grammar_leaf"]
            self.assertIn(
                "extractor/native/grammar_leaf/src/encode.rs", recorded["files"]
            )

    def test_a_file_leaving_the_leaf_stops_the_re_stamper(self) -> None:
        """The event that created this mechanism, one level up: the walk moves
        out from under the digest. A re-stamp through it hands back a green
        tree covering less than it did, so it needs a human."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.write(f"{fixture.ROOT}/src/encode.rs", "pub fn write() {}\n")
            fixture.stamped()
            (fixture.root / fixture.ROOT / "src" / "encode.rs").unlink()
            with self.assertRaises(SystemExit) as exited:
                fixture.restamp()
            message = str(exited.exception)
            self.assertIn("SHRANK", message)
            self.assertIn("grammar_leaf/src/encode.rs", message)
            fixture.restamp(allow_leaf_shrink=True)
            self.assertEqual(fixture.verify(), (1, 2))

    def test_a_crate_root_with_no_manifest_is_refused(self) -> None:
        """A mistyped root would otherwise hash whatever `.rs` files happened
        to be under it and report a confident digest over the wrong tree."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            (fixture.root / fixture.ROOT / "Cargo.toml").unlink()
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "it is not a crate root"
            ):
                fixture.verify()

    def test_a_missing_root_is_refused(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            for path in sorted(
                (fixture.root / fixture.ROOT).rglob("*"), reverse=True
            ):
                path.unlink() if path.is_file() else path.rmdir()
            (fixture.root / fixture.ROOT).rmdir()
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "is not a directory"
            ):
                fixture.verify()

    def test_a_checkout_under_a_skipped_directory_name_still_scans(self) -> None:
        """The skip list is `runs/` and `target/` INSIDE the tree. Matched
        against the absolute path instead, a lane worktree that happens to live
        under a directory called `runs` skips every file and reports a
        confident PASS over nothing."""
        with tempfile.TemporaryDirectory() as outer:
            root = Path(outer) / "runs" / "checkout"
            root.mkdir(parents=True)
            fixture = LeafFixture(root)
            fixture.restamp()
            self.assertEqual(fixture.verify(), (1, 2))
            fixture.write(
                "extractor/native/reader_three/src/lib.rs",
                "pub fn walk_it() -> usize { grammar_leaf::walk() }\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "is not a registered consumer"
            ):
                fixture.verify()

    def test_build_output_inside_the_crate_is_not_attested(self) -> None:
        """`target/` holds generated Rust. Attesting it would move the digest
        on every build and teach everyone to re-stamp without looking."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(f"{fixture.ROOT}/target/debug/build/generated.rs", "fn x() {}\n")
            self.assertEqual(fixture.verify(), (1, 2))

    def test_the_digest_does_not_depend_on_the_checkouts_line_endings(self) -> None:
        """This checkout is CRLF and CI trees are not. A digest that moved with
        the line endings would fail every run on one of them."""
        with tempfile.TemporaryDirectory() as lf_root, tempfile.TemporaryDirectory() as crlf_root:
            lf = LeafFixture(Path(lf_root), newline="\n")
            crlf = LeafFixture(Path(crlf_root), newline="\r\n")
            attested = VERIFY.leaf_attested_files(lf.root, lf.ROOT)
            self.assertEqual(
                VERIFY.leaf_manifest_digest(lf.root, attested),
                VERIFY.leaf_manifest_digest(
                    crlf.root, VERIFY.leaf_attested_files(crlf.root, crlf.ROOT)
                ),
            )

    def test_the_re_stamp_preserves_a_consumers_line_endings(self) -> None:
        """Rewriting a CRLF file through universal newlines rewrites the whole
        file to LF: invisible in `git diff`, which normalizes, and wrong on
        disk for every other tool."""
        temporary, fixture = self.fixture(newline="\r\n")
        with temporary:
            fixture.stamped()
            raw = (fixture.root / fixture.CONSUMERS[0]).read_bytes()
            self.assertNotIn(b"files=0 sha256=0", raw)
            self.assertEqual(fixture.verify(), (1, 2))
            self.assertNotIn(b"\n", raw.replace(b"\r\n", b""))
            self.assertIn(b"\r\n", raw)
            path = fixture.root / fixture.CONSUMERS[1]
            with path.open(encoding="utf-8", newline="") as handle:
                source = handle.read()
            mixed = source.replace("\r\n", "\n", 1)
            with path.open("w", encoding="utf-8", newline="") as handle:
                handle.write(mixed)
            fixture.write(f"{fixture.ROOT}/src/lib.rs", fixture.WALK.replace("16", "24"))
            fixture.restamp()
            after = path.read_bytes()
            self.assertEqual(after.count(b"\r\n"), mixed.count("\r\n"))
            self.assertEqual(fixture.verify(), (1, 2))

    def test_an_inventory_leaf_no_consumer_attests_is_refused(self) -> None:
        """A membership record nothing compares against anything accumulates
        coverage claims no run verifies."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            path = fixture.root / VERIFY.LEAF_INVENTORY
            document = json.loads(path.read_text(encoding="utf-8"))
            document["leaves"]["departed_leaf"] = {"root": "x", "files": []}
            path.write_text(json.dumps(document), encoding="utf-8", newline="\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "no consumer is required to attest"
            ):
                fixture.verify()

    def test_a_missing_inventory_is_not_an_empty_one(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            (fixture.root / VERIFY.LEAF_INVENTORY).unlink()
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "inventory .* is missing"
            ):
                fixture.verify()

    def test_a_wrong_shaped_inventory_says_so_instead_of_raising(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            path = fixture.root / VERIFY.LEAF_INVENTORY
            for payload, expected in (
                ("[]", "expected a JSON object at the root"),
                ('{"leaves": []}', 'expected an object under "leaves"'),
                ('{"leaves": {"grammar_leaf": null}}', "do not record a root"),
                (
                    '{"leaves": {"grammar_leaf": {"root": "x", "files": [3]}}}',
                    "do not record a root",
                ),
            ):
                path.write_text(payload, encoding="utf-8", newline="\n")
                with self.assertRaisesRegex(VERIFY.VerificationError, expected):
                    fixture.verify()

    def test_a_corrupt_inventory_stops_the_re_stamper_with_a_message(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            (fixture.root / VERIFY.LEAF_INVENTORY).write_text(
                "{not json", encoding="utf-8", newline="\n"
            )
            with self.assertRaises(SystemExit) as exited:
                fixture.restamp()
            self.assertIn("cannot read", str(exited.exception))

    def test_the_re_stamper_will_not_guess_where_a_marker_belongs(self) -> None:
        """Adoption pastes the line beside the code it attests. A tool that
        inserted one would put it where nobody reading the walk ever looks."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.write(
                fixture.CONSUMERS[0],
                "pub fn read() -> usize { grammar_leaf::walk() }\n",
            )
            with self.assertRaises(SystemExit) as exited:
                fixture.restamp()
            self.assertIn("Paste the line beside the code it attests", str(exited.exception))

    def test_the_re_stamper_will_not_re_point_a_marker_at_another_crate(self) -> None:
        """`root=` is read, never chosen: rewriting it would silently move an
        attestation from the crate the site executes to one it does not."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            source = fixture.read(fixture.CONSUMERS[0])
            fixture.write(
                fixture.CONSUMERS[0],
                source.replace(
                    "root=extractor/native/grammar_leaf",
                    "root=extractor/native/other_leaf",
                ),
            )
            with self.assertRaises(SystemExit) as exited:
                fixture.restamp()
            self.assertIn("the registry says", str(exited.exception))

    def test_main_runs_the_leaf_half_and_reports_both(self) -> None:
        """The one thing a separate check function can lose: `main` forgetting
        to call it. Both halves must report on the same run, so a world-fold
        failure cannot hide five stale leaf markers behind it."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                f"{fixture.ROOT}/src/lib.rs", fixture.WALK.replace("16", "20")
            )
            stderr = io.StringIO()
            with unittest.mock.patch.object(
                VERIFY, "REQUIRED_LEAF_AUTHORITIES", fixture.registry
            ), contextlib.redirect_stderr(stderr):
                exit_code = VERIFY.main(["--repo-root", str(fixture.root)])
            self.assertEqual(exit_code, 1)
            reported = stderr.getvalue()
            self.assertIn("world_fold.rs", reported)
            self.assertIn("leaf authority digest changed", reported)

    def test_every_attested_file_is_covered_not_only_the_walk(self) -> None:
        """Each attested file, one at a time. A digest that moved for `lib.rs`
        and not for the encoder, the drift-killer or the manifest would be
        covering the half of the leaf somebody happened to test."""
        for name, before, after in (
            ("src/lib.rs", "16", "20"),
            ("src/encode.rs", "1", "2"),
            ("src/tests.rs", "assert", "debug_assert"),
            ("Cargo.toml", 'name = "grammar_leaf"', 'name = "grammar_leaf"\nedition = "2021"'),
        ):
            with self.subTest(file=name):
                temporary, fixture = self.fixture()
                with temporary:
                    fixture.write(f"{fixture.ROOT}/src/encode.rs", "pub const W: u8 = 1;\n")
                    fixture.write(
                        f"{fixture.ROOT}/src/tests.rs",
                        "#[test]\nfn t() { assert!(true); }\n",
                    )
                    fixture.stamped()
                    self.assertEqual(fixture.verify(), (1, 2))
                    source = fixture.read(f"{fixture.ROOT}/{name}")
                    fixture.write(
                        f"{fixture.ROOT}/{name}", source.replace(before, after, 1)
                    )
                    with self.assertRaisesRegex(
                        VERIFY.VerificationError, "leaf authority digest changed"
                    ):
                        fixture.verify()

    def test_a_control_character_does_not_collapse_into_a_newline(self) -> None:
        """`str.splitlines()` treats form feed, vertical tab, NEL and the
        Unicode separators as line breaks. Normalizing with it would hash two
        different Rust string literals identically."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            attested = VERIFY.leaf_attested_files(fixture.root, fixture.ROOT)
            fixture.write(f"{fixture.ROOT}/src/lib.rs", 'pub const S: &str = "a\fb";\n')
            with_form_feed = VERIFY.leaf_manifest_digest(fixture.root, attested)
            fixture.write(f"{fixture.ROOT}/src/lib.rs", 'pub const S: &str = "a\nb";\n')
            self.assertNotEqual(
                with_form_feed,
                VERIFY.leaf_manifest_digest(fixture.root, attested),
            )

    def test_a_leaf_that_can_execute_code_outside_its_source_is_refused(self) -> None:
        """The seal. Each of these changes what the crate RUNS while every
        attested byte stays identical, so a digest over the files would be an
        attestation of something that is not what executes."""
        cases = (
            (
                "Cargo.toml",
                '[package]\nname = "grammar_leaf"\nbuild = "make.rs"\n',
                "declares a build script",
            ),
            (
                "Cargo.toml",
                '[package]\nname = "grammar_leaf"\n\n[lib]\npath = "../elsewhere.rs"\n',
                r"sets \[lib\] path",
            ),
            (
                "Cargo.toml",
                '[package]\nname = "grammar_leaf"\n\n[dependencies]\nserde = "1"\n',
                r"declares \[dependencies\]",
            ),
            (
                "Cargo.toml",
                '[package]\nname = "grammar_leaf"\n\n[features]\nstrict = []\n',
                r"declares \[features\]",
            ),
            (
                "Cargo.toml",
                '[package]\nname = "another_crate"\n',
                "attests a package nobody builds",
            ),
            (
                "src/lib.rs",
                '#[path = "../../elsewhere.rs"]\nmod walk;\n',
                "an attribute that redirects a module's path",
            ),
            (
                "src/lib.rs",
                'include!("../../generated.rs");\n',
                "include!.-family macro",
            ),
            (
                "src/lib.rs",
                'pub const D: &str = env!("SOME_BUILD_VALUE");\n',
                "build-time `env!` value",
            ),
            (
                "src/lib.rs",
                'pub const D: &str = concat!(env!("OUT_DIR"), "/x");\n',
                "OUT_DIR|build-time",
            ),
        )
        for name, replacement, expected in cases:
            with self.subTest(case=expected):
                temporary, fixture = self.fixture()
                with temporary:
                    fixture.stamped()
                    fixture.write(f"{fixture.ROOT}/{name}", replacement)
                    with self.assertRaisesRegex(VERIFY.VerificationError, expected):
                        fixture.verify()

    def test_an_implicit_build_script_is_refused(self) -> None:
        """Cargo runs `build.rs` whether or not the manifest names it."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(f"{fixture.ROOT}/build.rs", "fn main() {}\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "carries build.rs"
            ):
                fixture.verify()

    def test_a_manifest_with_no_library_is_refused(self) -> None:
        """No `[lib] path` and no `src/lib.rs` is not a library: a manifest-only
        directory would otherwise be stamped, digested and reported."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            (fixture.root / fixture.ROOT / "src" / "lib.rs").unlink()
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "no library here for a consumer to execute"
            ):
                fixture.verify()

    def test_a_macro_definition_in_the_leaf_is_refused(self) -> None:
        """The seal's other rules read source text, and a `macro_rules!` that
        expands an identifier into `include!` puts the forbidden construct
        somewhere no regex over this file can see it. The cost -- a leaf may not
        use a declarative macro to generate repetitive grammar code -- is real
        and accepted; closing it properly needs expansion, not a regex."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                f"{fixture.ROOT}/src/lib.rs",
                "macro_rules! rows { () => { 16 } }\n" + fixture.WALK,
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "a macro definition"
            ):
                fixture.verify()

    def test_a_conditional_attribute_or_expression_is_refused(self) -> None:
        """`#[cfg(..)]` is not the only conditional: `cfg_attr` changes a type's
        LAYOUT -- the thing a byte grammar is -- and `cfg!` branches at run
        time, both on inputs the digest does not cover. Each of these
        re-stamped and verified before the rule existed."""
        for name, source in (
            (
                "cfg_attr layout",
                "#[cfg_attr(windows, repr(C, packed))]\npub struct Row(pub u16);\n",
            ),
            (
                "cfg_attr path",
                '#[cfg_attr(windows, path = "../../outside.rs")]\nmod walk;\n',
            ),
            (
                "cfg! expression",
                "pub fn width() -> usize { if cfg!(windows) { 2 } else { 4 } }\n",
            ),
        ):
            with self.subTest(case=name):
                temporary, fixture = self.fixture()
                with temporary:
                    fixture.stamped()
                    fixture.write(f"{fixture.ROOT}/src/lib.rs", source + fixture.WALK)
                    with self.assertRaisesRegex(
                        VERIFY.VerificationError, "cfg_attr|cfg!|redirects a module"
                    ):
                        fixture.verify()

    def test_a_link_in_the_leaf_is_refused(self) -> None:
        """A link resolves somewhere no digest reads. Skipped where this
        process cannot create one -- Windows needs the privilege -- rather than
        passing vacuously."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            target = fixture.root / "outside.rs"
            target.write_text("pub fn elsewhere() {}\n", encoding="utf-8")
            link = fixture.root / fixture.ROOT / "src" / "linked.rs"
            try:
                link.symlink_to(target)
            except (OSError, NotImplementedError) as error:
                self.skipTest(f"cannot create a symbolic link here: {error}")
            with self.assertRaisesRegex(VERIFY.VerificationError, "contains a link"):
                fixture.verify()

    def test_metadata_that_cannot_be_read_is_fatal(self) -> None:
        """A child this walk enumerated and then could not stat must not be
        skipped quietly: `is_file()` says False for the same reason, and the
        entry would leave the attested set without a word."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            real_lstat = Path.lstat

            def refusing_lstat(self):
                if self.name == "encode.rs":
                    raise OSError(5, "device not ready")
                return real_lstat(self)

            fixture.write(f"{fixture.ROOT}/src/encode.rs", "pub fn write() {}\n")
            with unittest.mock.patch.object(Path, "lstat", refusing_lstat):
                with self.assertRaisesRegex(
                    VERIFY.VerificationError, "cannot read the metadata"
                ):
                    fixture.verify()

    def test_a_forbidden_construct_in_a_comment_is_not_a_construct(self) -> None:
        """The seal reads code, not prose: the leaf documents its own rules,
        and a gate that refused the sentence would make them undocumentable."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                f"{fixture.ROOT}/src/lib.rs",
                "//! This crate has no build script, no `include!` and no\n"
                '//! `env!("X")`, and OUT_DIR never reaches it.\n' + fixture.WALK,
            )
            fixture.restamp()
            self.assertEqual(fixture.verify(), (1, 2))

    def test_a_consumer_that_depends_on_another_copy_is_refused(self) -> None:
        """A marker says "I execute this crate"; Cargo decides which crate that
        is. Re-pointing the dependency keeps the code compiling, keeps the
        marker green, and stops the attested source being what runs."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/reader_one/Cargo.toml",
                '[package]\nname = "reader_one"\n\n'
                "[dependencies]\n"
                'grammar_leaf = { path = "../vendored_grammar_leaf" }\n',
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "resolves outside the registered root"
            ):
                fixture.verify()

    def test_a_consumer_that_depends_on_a_registry_version_is_refused(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/reader_one/Cargo.toml",
                '[package]\nname = "reader_one"\n\n'
                "[dependencies]\n"
                'grammar_leaf = "0.1"\n',
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "without a path"
            ):
                fixture.verify()

    def test_a_dependency_key_that_builds_another_package_is_refused(self) -> None:
        """Cargo's `package =` field decides what a key builds. Reading the key
        alone accepts `grammar_leaf = { package = "vendored_fork" }`, which
        compiles, keeps the marker green, and runs somebody else's code."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/reader_one/Cargo.toml",
                '[package]\nname = "reader_one"\n\n'
                "[dependencies]\n"
                'grammar_leaf = { package = "vendored_fork", path = "../vendored" }\n',
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"package=vendored_fork"
            ):
                fixture.verify()

    def test_a_cargo_alias_for_the_leaf_is_refused(self) -> None:
        """A Cargo alias is the one way to execute the leaf without ever
        writing its name: `grammar = { package = "grammar_leaf" }`, and then
        `grammar::walk()`. The source scan that finds an unattested walker
        looks for the crate's name, so the alias would hide a whole package
        from it. The dependency is therefore spelled with the crate's own name
        everywhere -- which the three packages that take d13_grammar already
        do."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/reader_one/Cargo.toml",
                '[package]\nname = "reader_one"\n\n'
                "[dependencies]\n"
                'grammar = { package = "grammar_leaf", path = "../grammar_leaf" }\n',
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "must be spelled grammar_leaf"
            ):
                fixture.verify()

    def test_a_workspace_inherited_dependency_is_refused(self) -> None:
        """`{ workspace = true }` moves the path into a manifest this site does
        not own, where a later workspace edit can re-point every consumer at
        once without touching a single attested file."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/Cargo.toml",
                "[workspace]\nmembers = []\n\n"
                "[workspace.dependencies]\n"
                'grammar_leaf = { path = "grammar_leaf" }\n',
            )
            fixture.write(
                "extractor/native/reader_one/Cargo.toml",
                '[package]\nname = "reader_one"\n\n'
                "[dependencies]\n"
                "grammar_leaf = { workspace = true }\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "inherits grammar_leaf from the workspace"
            ):
                fixture.verify()

    def test_a_package_that_takes_the_leaf_and_attests_nothing_is_refused(self) -> None:
        """The completeness half, from the manifests: a package can depend on
        the leaf and walk it in a file the marker scan passes over. Every
        package that CAN reach the grammar must own a site that attests it."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/reader_four/Cargo.toml",
                '[package]\nname = "reader_four"\n\n'
                "[dependencies]\n"
                'grammar_leaf = { path = "../grammar_leaf" }\n',
            )
            fixture.write(
                "extractor/native/reader_four/src/lib.rs",
                "pub fn nothing() -> usize { 1 }\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "owns no registered consumer"
            ):
                fixture.verify()

    def test_a_target_conditional_dependency_in_the_leaf_is_refused(self) -> None:
        """`[target.'cfg(windows)'.dependencies]` is a dependency table a
        top-level scan does not see, and behaviour from another crate on one
        platform only is the hardest kind to notice."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                f"{fixture.ROOT}/Cargo.toml",
                '[package]\nname = "grammar_leaf"\n\n'
                "[target.'cfg(windows)'.dependencies]\nserde = \"1\"\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError,
                r"declares \[target\.cfg\(windows\)\.dependencies\]",
            ):
                fixture.verify()

    def test_a_production_cfg_in_the_leaf_is_refused(self) -> None:
        """Which branch of the attested source RUNS must be a function of the
        digest. `#[cfg(test)]` is the exception: it excludes the crate's own
        proof, and `test` is set by the harness, not chosen by whoever builds
        the consumers."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                f"{fixture.ROOT}/src/lib.rs",
                "#[cfg(test)]\nmod tests {}\n"
                '#[cfg(feature = "strict")]\n'
                "pub const ROW_LEN: usize = 20;\n" + fixture.WALK,
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"carries #\[cfg\(feature="
            ):
                fixture.verify()

    def test_a_re_export_of_the_leaf_is_refused(self) -> None:
        """A registered consumer writing `pub use grammar_leaf::...` lets a
        downstream file walk the grammar without naming the crate -- the one
        route the source scan cannot see."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            source = fixture.read(fixture.CONSUMERS[0])
            fixture.write(
                fixture.CONSUMERS[0], source + "pub use grammar_leaf::walk;\n"
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "publicly re-exports grammar_leaf"
            ):
                fixture.verify()

    def test_a_new_unregistered_user_of_the_leaf_is_refused(self) -> None:
        """A marker scan finds markers; it cannot find a consumer nobody
        marked. A sixth site that walks the grammar with no marker is exactly
        the shape this whole mechanism exists to stop, and it is invisible from
        the marker end."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/reader_three/src/lib.rs",
                "pub fn walk_it() -> usize { grammar_leaf::walk() }\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "is not a registered consumer"
            ):
                fixture.verify()

    def test_a_const_only_user_is_recorded_by_identifier(self) -> None:
        """`offsets.rs` re-exports two constants and walks nothing, so it needs
        no attestation -- but the exception is per IDENTIFIER, so the day that
        file starts walking the grammar it fails instead of staying exempt."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.const_only = {
                "extractor/native/reader_three/src/lib.rs": ("ROW_LEN",)
            }
            fixture.write(
                "extractor/native/reader_three/Cargo.toml",
                '[package]\nname = "reader_three"\n\n'
                "[dependencies]\n"
                'grammar_leaf = { path = "../grammar_leaf" }\n',
            )
            fixture.write(
                "extractor/native/reader_three/src/lib.rs",
                "pub const LEN: usize = grammar_leaf::ROW_LEN;\n",
            )
            fixture.stamped()
            self.assertEqual(fixture.verify(), (1, 2))
            fixture.write(
                "extractor/native/reader_three/src/lib.rs",
                "pub const LEN: usize = grammar_leaf::ROW_LEN;\n"
                "pub fn walk_it() -> usize { grammar_leaf::walk() }\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"names \['walk'\]"
            ):
                fixture.verify()
            fixture.write(
                "extractor/native/reader_three/src/lib.rs",
                "use grammar_leaf as g;\n"
                "pub const LEN: usize = grammar_leaf::ROW_LEN;\n"
                "pub fn walk_it() -> usize { g::walk() }\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError,
                "not a recorded `grammar_leaf::<const>` reference",
            ):
                fixture.verify()

    def test_naming_the_leaf_in_a_comment_is_not_using_it(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/reader_three/src/lib.rs",
                "// grammar_leaf owns the walk; this file only says so.\n"
                'pub const NOTE: &str = "grammar_leaf::walk";\n',
            )
            self.assertEqual(fixture.verify(), (1, 2))

    def test_a_re_stamp_is_idempotent_and_check_reports_drift(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            self.assertEqual(fixture.restamp(), (0, 2, False))
            fixture.write(f"{fixture.ROOT}/src/lib.rs", fixture.WALK.replace("16", "20"))
            self.assertEqual(
                RESTAMP.restamp_leaves(
                    fixture.root,
                    check=True,
                    allow_leaf_shrink=False,
                    report=lambda _: None,
                    registry=fixture.registry,
                ),
                (2, 2, False),
            )
            with self.assertRaises(VERIFY.VerificationError):
                fixture.verify()

    def test_a_refused_leaf_shrink_leaves_the_world_markers_alone(self) -> None:
        """The two re-stamps refuse for different reasons. Writing the first
        half before asking the second leaves a tree with re-stamped world
        markers and a run nobody asked for -- half a re-stamp is the state
        that makes a lane distrust the tool."""
        temporary = tempfile.TemporaryDirectory()
        with temporary:
            root = Path(temporary.name)
            world = CitationFixture(root, INLINE_RENDER)
            world.add_module(1, ("fold_render",))
            leaf = LeafFixture(root)
            leaf.write(f"{leaf.ROOT}/src/encode.rs", "pub const W: u8 = 1;\n")
            leaf.stamped()
            world.set_world("// an insertion above everything\n" + INLINE_RENDER)
            before = (world.wire_dir / "d1.rs").read_text(encoding="utf-8")
            (root / leaf.ROOT / "src" / "encode.rs").unlink()
            with unittest.mock.patch.object(
                VERIFY, "REQUIRED_LEAF_AUTHORITIES", leaf.registry
            ), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(SystemExit):
                    RESTAMP.main(["--repo-root", str(root)])
            self.assertEqual(
                (world.wire_dir / "d1.rs").read_text(encoding="utf-8"),
                before,
                "the world-fold half wrote before the leaf half refused",
            )

    def test_a_lone_cr_normalizes_like_a_crlf(self) -> None:
        """Universal newlines is the normalization, and it maps a lone CR too.
        A tree that still carries classic-Mac endings must hash the same."""
        with tempfile.TemporaryDirectory() as lf_root, tempfile.TemporaryDirectory() as cr_root:
            lf = LeafFixture(Path(lf_root), newline="\n")
            cr = LeafFixture(Path(cr_root), newline="\r")
            self.assertEqual(
                VERIFY.leaf_manifest_digest(
                    lf.root, VERIFY.leaf_attested_files(lf.root, lf.ROOT)
                ),
                VERIFY.leaf_manifest_digest(
                    cr.root, VERIFY.leaf_attested_files(cr.root, cr.ROOT)
                ),
            )

    def test_a_user_outside_extractor_native_is_seen(self) -> None:
        """The scan is repository-wide, not `extractor/native`-wide. A package
        under `replay_interface` that path-depends on the leaf and walks it was
        invisible while the gate said PASS -- a completeness claim scoped to the
        directory today's consumers happen to sit in is not one."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "replay_interface/native/reader/Cargo.toml",
                '[package]\nname = "outside_reader"\n\n'
                "[dependencies]\n"
                'grammar_leaf = { path = "../../../extractor/native/grammar_leaf" }\n',
            )
            fixture.write(
                "replay_interface/native/reader/src/lib.rs",
                "pub fn walk_it() -> usize { grammar_leaf::walk() }\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "is not a registered consumer"
            ):
                fixture.verify()
            fixture.write(
                "replay_interface/native/reader/src/lib.rs",
                "// @leaf-source-authority crate=grammar_leaf "
                "root=extractor/native/grammar_leaf files=0 sha256=0\n"
                "pub fn walk_it() -> usize { grammar_leaf::walk() }\n",
            )
            registry = {
                fixture.CRATE: VERIFY.LeafAuthority(
                    root=fixture.ROOT,
                    consumers=fixture.CONSUMERS
                    + ("replay_interface/native/reader/src/lib.rs",),
                )
            }
            RESTAMP.restamp_leaves(
                fixture.root,
                check=False,
                allow_leaf_shrink=False,
                report=lambda _: None,
                registry=registry,
            )
            self.assertEqual(
                VERIFY.verify_leaf_authorities(fixture.root, registry), (1, 3)
            )

    def test_a_package_whose_source_this_scan_cannot_read_is_refused(self) -> None:
        """`[lib] path = "source.rust"` compiles and contains no file a `*.rs`
        walk visits, so the package, its dependency and its walker were all
        invisible. Manifests are now found by walking for MANIFESTS, and a
        leaf-dependent package may not build from a source this gate cannot
        read."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "portable_replay/kp_probe/Cargo.toml",
                '[package]\nname = "kp_probe"\n\n'
                "[lib]\n"
                'path = "source.rust"\n\n'
                "[dependencies]\n"
                'grammar_leaf = { path = "../../extractor/native/grammar_leaf" }\n',
            )
            fixture.write(
                "portable_replay/kp_probe/source.rust",
                "pub use grammar_leaf::*;\n",
            )
            with self.assertRaises(VERIFY.VerificationError) as raised:
                fixture.verify()
            message = str(raised.exception)
            self.assertIn("owns no registered consumer", message)
            self.assertIn("source scan does not read", message)

    def test_a_module_redirected_out_of_the_scan_is_refused(self) -> None:
        """Cargo's target paths are not the only redirect: a perfectly ordinary
        registered consumer can write `#[path = "../../runs/hidden.rs"] mod
        hidden;` and compile a walker out of an evidence directory, while every
        visible file in the package is clean. Measured passing before this
        rule."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/runs/hidden_reader.rs",
                "pub fn hidden() -> usize { grammar_leaf::walk() }\n",
            )
            fixture.write(
                fixture.CONSUMERS[0],
                fixture.read(fixture.CONSUMERS[0])
                + '#[path = "../../runs/hidden_reader.rs"]\nmod hidden_reader;\n',
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "with a `#\\[path\\]` module"
            ):
                fixture.verify()

    def test_an_ordinary_module_redirect_inside_the_package_is_fine(self) -> None:
        """`#[path]` is legal and used; only a redirect the scan cannot follow
        is refused. A gate that refused all of them would be refusing correct
        trees for nothing -- and this one caught two real defects: the first cut
        normalized with `os.path`, which returns backslashes on Windows and
        failed EVERY redirect, and the second guessed only two resolution bases,
        which fails the legal inline form `mod outer { #[path = "inner.rs"] mod
        inner; }` (Rust loads `outer/inner.rs`)."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/reader_one/src/shared/walkers.rs",
                "pub fn ok() -> usize { 1 }\n",
            )
            fixture.write(
                "extractor/native/reader_one/src/outer/inner.rs",
                "pub fn also_ok() -> usize { 2 }\n",
            )
            fixture.write(
                fixture.CONSUMERS[0],
                fixture.read(fixture.CONSUMERS[0])
                + '#[path = "shared/walkers.rs"]\nmod walkers;\n'
                + 'mod outer { #[path = "inner.rs"] mod inner; }\n',
            )
            self.assertEqual(fixture.verify(), (1, 2))

    def test_a_package_under_a_skipped_directory_is_still_found(self) -> None:
        """An evidence tree is append-only prose, and a marker quoted in one is
        a record. A CARGO PACKAGE under `runs/` is not prose: it compiles, it
        can take the leaf and walk it, and this repository has three such probe
        crates. Manifests are therefore skipped in fewer places than sources,
        and a leaf-dependent package that lives where the source scan does not
        look is refused."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            fixture.write(
                "extractor/native/runs/hidden_reader/Cargo.toml",
                '[package]\nname = "hidden_reader"\n\n'
                "[dependencies]\n"
                'grammar_leaf = { path = "../../grammar_leaf" }\n',
            )
            fixture.write(
                "extractor/native/runs/hidden_reader/src/lib.rs",
                "pub fn hidden() -> usize { grammar_leaf::walk() }\n",
            )
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "which this gate's source scan skips"
            ):
                fixture.verify()

    def test_a_registered_package_may_not_hide_a_second_target(self) -> None:
        """A package can own a perfectly good registered consumer AND build
        something else Cargo runs: a `build = "build.rust"` script, or a
        `[lib] path = "runs/source.rs"` inside a directory this scan skips.
        Both compiled, both executed the leaf, and both passed."""
        for name, manifest, source_path in (
            (
                "a build script from a non-.rs path",
                '[package]\nname = "reader_one"\nbuild = "build.rust"\n\n'
                "[dependencies]\n"
                'grammar_leaf = { path = "../grammar_leaf" }\n\n'
                "[build-dependencies]\n"
                'grammar_leaf = { path = "../grammar_leaf" }\n',
                "extractor/native/reader_one/build.rust",
            ),
            (
                "a library inside a skipped directory",
                '[package]\nname = "reader_one"\n\n'
                "[lib]\n"
                'path = "runs/source.rs"\n\n'
                "[dependencies]\n"
                'grammar_leaf = { path = "../grammar_leaf" }\n',
                "extractor/native/reader_one/runs/source.rs",
            ),
        ):
            with self.subTest(case=name):
                temporary, fixture = self.fixture()
                with temporary:
                    fixture.stamped()
                    fixture.write("extractor/native/reader_one/Cargo.toml", manifest)
                    fixture.write(
                        source_path,
                        "pub fn hidden() -> usize { grammar_leaf::walk() }\n",
                    )
                    with self.assertRaisesRegex(
                        VERIFY.VerificationError, "source scan does not read"
                    ):
                        fixture.verify()

    def test_two_leaves_sharing_one_consumer_both_get_re_stamped(self) -> None:
        """A port that walks two shared grammars carries two markers. Planned
        from the same text, the second write erased the first -- the same
        defect as the world/leaf pair, one level in."""
        temporary = tempfile.TemporaryDirectory()
        with temporary:
            root = Path(temporary.name)
            first = LeafFixture(root)
            second_root = "extractor/native/other_leaf"
            shared = first.CONSUMERS[0]
            for relative, text in (
                (f"{second_root}/Cargo.toml", '[package]\nname = "other_leaf"\n'),
                (f"{second_root}/src/lib.rs", "pub fn other() -> usize { 8 }\n"),
            ):
                first.write(relative, text)
            first.write(
                "extractor/native/reader_one/Cargo.toml",
                '[package]\nname = "reader_one"\n\n'
                "[dependencies]\n"
                'grammar_leaf = { path = "../grammar_leaf" }\n'
                'other_leaf = { path = "../other_leaf" }\n',
            )
            first.write(
                shared,
                first.read(shared)
                + "// @leaf-source-authority crate=other_leaf "
                f"root={second_root} files=0 sha256=0\n"
                "pub fn also() -> usize { other_leaf::other() }\n",
            )
            registry = dict(first.registry)
            registry["other_leaf"] = VERIFY.LeafAuthority(
                root=second_root, consumers=(shared,)
            )
            RESTAMP.restamp_leaves(
                root,
                check=False,
                allow_leaf_shrink=False,
                report=lambda _: None,
                registry=registry,
            )
            self.assertEqual(VERIFY.verify_leaf_authorities(root, registry), (2, 3))
            source = first.read(shared)
            self.assertNotIn("files=0 sha256=0", source)
            first.write(f"{first.ROOT}/src/lib.rs", first.WALK.replace("16", "20"))
            first.write(f"{second_root}/src/lib.rs", "pub fn other() -> usize { 9 }\n")
            RESTAMP.restamp_leaves(
                root,
                check=False,
                allow_leaf_shrink=False,
                report=lambda _: None,
                registry=registry,
            )
            self.assertEqual(VERIFY.verify_leaf_authorities(root, registry), (2, 3))

    def test_the_spellings_that_reach_a_crate_without_a_plain_path(self) -> None:
        """Each of these executes the leaf and matches no `crate::`, no plain
        `use crate` and no `extern crate crate` -- they were measured passing an
        earlier version of this scan. A bare token is therefore a use."""
        for name, source in (
            ("grouped use with an alias", "use {grammar_leaf as g};\npub fn f() -> usize { g::walk() }\n"),
            ("grouped path", "use {grammar_leaf::walk};\npub fn f() -> usize { walk() }\n"),
            (
                "macro token",
                "macro_rules! call { ($c:ident) => { $c::walk() } }\n"
                "pub fn f() -> usize { call!(grammar_leaf) }\n",
            ),
        ):
            with self.subTest(spelling=name):
                temporary, fixture = self.fixture()
                with temporary:
                    fixture.stamped()
                    fixture.write("extractor/native/reader_three/src/lib.rs", source)
                    with self.assertRaisesRegex(
                        VERIFY.VerificationError, "is not a registered consumer"
                    ):
                        fixture.verify()

    def test_every_public_re_export_spelling_is_refused(self) -> None:
        """A re-export is the one route the token scan cannot see, so all of
        its spellings are refused at the registered consumer that would create
        it -- grouped and `pub extern crate` included."""
        for name, line in (
            ("plain", "pub use grammar_leaf::walk;\n"),
            ("grouped", "pub use {grammar_leaf::walk};\n"),
            ("aliased", "pub(crate) use grammar_leaf as g;\n"),
            ("extern crate", "pub extern crate grammar_leaf;\n"),
        ):
            with self.subTest(spelling=name):
                temporary, fixture = self.fixture()
                with temporary:
                    fixture.stamped()
                    fixture.write(
                        fixture.CONSUMERS[0], fixture.read(fixture.CONSUMERS[0]) + line
                    )
                    with self.assertRaisesRegex(
                        VERIFY.VerificationError, "publicly re-exports grammar_leaf"
                    ):
                        fixture.verify()

    def test_both_markers_in_one_file_are_re_stamped_together(self) -> None:
        """`wire_format/src/d13.rs` carries BOTH markers. Planned from the same
        disk preimage and written independently, the second write erases the
        first -- an atomic re-stamp that leaves one marker stale."""
        temporary = tempfile.TemporaryDirectory()
        with temporary:
            root = Path(temporary.name)
            world = CitationFixture(root, INLINE_RENDER)
            world.add_module(1, ("fold_render",))
            module = "extractor/native/wire_format/src/d1.rs"
            leaf = LeafFixture(root, consumers=(module,) + LeafFixture.CONSUMERS)
            leaf.stamped()
            self.assertEqual(leaf.verify(), (1, 3))
            world.set_world("// an insertion above everything\n" + INLINE_RENDER)
            leaf.write(f"{leaf.ROOT}/src/lib.rs", leaf.WALK.replace("16", "20"))
            with unittest.mock.patch.object(
                VERIFY, "REQUIRED_LEAF_AUTHORITIES", leaf.registry
            ), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(RESTAMP.main(["--repo-root", str(root)]), 0)
            source = (root / module).read_text(encoding="utf-8")
            self.assertIsNotNone(
                VERIFY.WORLD_MARKER_RE.search(source), "the world marker was erased"
            )
            self.assertIsNotNone(
                VERIFY.LEAF_MARKER_RE.search(source), "the leaf marker was erased"
            )
            self.assertEqual(leaf.verify(), (1, 3))
            self.assertEqual(
                VERIFY.verify_repo(root, required_lane_authorities=()), (1, 0, 0, 0)
            )

    def test_a_malformed_registry_is_refused_before_anything_is_checked(self) -> None:
        """The registry is a hand-written literal, and hand-written is not
        correct: a duplicated consumer, an escaping path or a site that is both
        attested and exempt each make some check below quietly weaker."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            cases = (
                (
                    VERIFY.LeafAuthority(
                        root=fixture.ROOT,
                        consumers=fixture.CONSUMERS + (fixture.CONSUMERS[0],),
                    ),
                    "more than once",
                ),
                (
                    VERIFY.LeafAuthority(root="../outside", consumers=fixture.CONSUMERS),
                    "not a normalized repository-relative POSIX path",
                ),
                (
                    VERIFY.LeafAuthority(root=fixture.ROOT, consumers=()),
                    "registers no consumer site",
                ),
                (
                    VERIFY.LeafAuthority(
                        root=fixture.ROOT,
                        consumers=fixture.CONSUMERS,
                        const_only={fixture.CONSUMERS[0]: ("ROW_LEN",)},
                    ),
                    "cannot be both attested and exempt",
                ),
                (
                    VERIFY.LeafAuthority(
                        root=fixture.ROOT,
                        consumers=fixture.CONSUMERS,
                        const_only={"extractor/native/x/src/lib.rs": ()},
                    ),
                    "must name the identifiers it may read",
                ),
            )
            for leaf, expected in cases:
                with self.subTest(expected=expected):
                    with self.assertRaisesRegex(VERIFY.VerificationError, expected):
                        VERIFY.verify_leaf_authorities(
                            fixture.root, {fixture.CRATE: leaf}
                        )

    def test_a_marker_with_the_wrong_file_count_or_root_is_refused(self) -> None:
        """The digest is not the only field that has to be true: a marker that
        attests the right bytes under the wrong root, or claims a different
        number of files, is describing a leaf that is not the one it names."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            source = fixture.read(fixture.CONSUMERS[0])
            for replacement, expected in (
                (source.replace("files=2", "files=3"), "marker attests 3 file"),
                (
                    source.replace(
                        "root=extractor/native/grammar_leaf",
                        "root=extractor/native/other_root",
                    ),
                    "marker attests root",
                ),
            ):
                fixture.write(fixture.CONSUMERS[0], replacement)
                with self.assertRaisesRegex(VERIFY.VerificationError, expected):
                    fixture.verify()
                fixture.write(fixture.CONSUMERS[0], source)

    def test_a_world_closure_refusal_leaves_the_leaf_markers_alone(self) -> None:
        """The reverse direction of the atomicity property: the leaf half has
        writes pending and the world half refuses. Neither may land."""
        temporary = tempfile.TemporaryDirectory()
        with temporary:
            root = Path(temporary.name)
            world = CitationFixture(root, EXTRACTED_RENDER)
            world.add_module(1, ("fold_render",))
            leaf = LeafFixture(root)
            leaf.stamped()
            leaf.write(f"{leaf.ROOT}/src/lib.rs", leaf.WALK.replace("16", "20"))
            before = leaf.read(leaf.CONSUMERS[0])
            world.set_world(MOVED_OUT_OF_FILE)
            with unittest.mock.patch.object(
                VERIFY, "REQUIRED_LEAF_AUTHORITIES", leaf.registry
            ), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(SystemExit):
                    RESTAMP.main(["--repo-root", str(root)])
            self.assertEqual(
                leaf.read(leaf.CONSUMERS[0]),
                before,
                "the leaf half wrote before the world half refused",
            )

    def test_a_plan_whose_file_changed_underneath_it_writes_nothing(self) -> None:
        """A re-stamp is a decision about the bytes it read. If another lane's
        editor saves over one of those files between planning and writing, the
        write would overwrite that work with a re-stamp of the version before
        it."""
        temporary, fixture = self.fixture()
        with temporary:
            fixture.stamped()
            path = fixture.root / fixture.CONSUMERS[0]
            with path.open(encoding="utf-8", newline="") as handle:
                original = handle.read()
            stale = RESTAMP.PlannedWrite(path, "rewritten\n", original + "// later\n", "")
            with self.assertRaises(SystemExit) as exited:
                RESTAMP.commit_plan([stale])
            self.assertIn("changed while this re-stamp was being planned", str(exited.exception))
            with path.open(encoding="utf-8", newline="") as handle:
                self.assertEqual(handle.read(), original)

    def test_the_shipped_registry_still_covers_the_leaf_this_tree_ships(self) -> None:
        """`0 leaf source authorities` passes every structural check in this
        file. What it means is that somebody emptied the registry, and the
        whole mechanism now verifies nothing."""
        self.assertIn("d13_grammar", VERIFY.REQUIRED_LEAF_AUTHORITIES)
        leaf = VERIFY.REQUIRED_LEAF_AUTHORITIES["d13_grammar"]
        self.assertEqual(leaf.root, "extractor/native/d13_grammar")
        self.assertEqual(
            sorted(leaf.consumers),
            [
                "extractor/native/collector/src/attach_log.rs",
                "extractor/native/collector/src/attach_log/domain13_tech_delta_tests.rs",
                "extractor/native/recorder_host/src/domain_data_authority.rs",
                "extractor/native/recorder_host/src/world_fold.rs",
                "extractor/native/wire_format/src/d13.rs",
            ],
        )


if __name__ == "__main__":
    unittest.main()
