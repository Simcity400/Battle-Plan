#!/usr/bin/env python3
"""Focused deterministic tests for verify_world_fold_citations.py."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path


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


# --- the extraction scenario, as it actually happened ----------------------
#
# d20's wire module transcribes "row decode and object build". Both used to live
# in `fold_divisions`; the build was lifted into a row method, the marker was
# re-stamped over the residual, and the moved rendering left every digest while
# the gate went on printing PASS. These three worlds are that sequence, reduced
# to the parts a gate can see.

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

# The same extraction, but the helper left the file entirely.
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

            # Lift the render out, exactly as the d20 lane did, and re-stamp:
            # the extraction itself is a legitimate refactor and must pass.
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

            # Now change what the MOVED code renders.
            mutated = EXTRACTED_RENDER.replace("dig_in.min(2)", "dig_in.min(1)")
            self.assertNotEqual(mutated, EXTRACTED_RENDER)
            fixture.set_world(mutated)

            # THE OLD GATE'S CHECK STILL PASSES: the marker's line range covers
            # `fold_render` only, and `fold_render` did not change. This is the
            # exact assertion that makes this a regression test rather than a
            # restatement of the digest check.
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
        # The other half of the claim: the closure must not turn every edit to
        # world_fold.rs into a citation failure.
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )
            marker = fixture.marker_of(1)
            for original, replacement in (
                ("self.value += 7;", "self.value += 8;"),  # fold_other
                ("self.value += 11;", "self.value += 12;"),  # unrelated_helper
                # An INSERTION above everything, which shifts every line number
                # in the file. The closure digest is keyed by name, not
                # position, so it must be untouched by that.
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

            # The line-shift case separately: the closure digest must survive it
            # even though the envelope's `lines=` does not.
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
        # The residual this design does NOT close: a helper moved to another
        # file is outside the closure by construction. What it must not do is
        # vanish quietly -- the membership record turns it into a failure whose
        # message names the function, and the re-stamper refuses to launder it.
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

            # Acknowledged, it re-stamps -- and the gate then passes over the
            # smaller closure, which is why the acknowledgement is the review
            # surface rather than a formality.
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
        # `fold_divisions` reaches `DivisionFoldRow::equipment_join_key` only as
        # `.and_then(DivisionFoldRow::equipment_join_key)` -- no parentheses of
        # its own. A scan that only looked for `name(` left that logic outside
        # every digest while the gate passed.
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
            # `Row` itself is covered too: the fold names the type, and a
            # type definition is executable input to what the fold reads.
            self.assertEqual(
                fixture.closure_of(("fold_value_ref",)),
                {"WorldFold::fold_value_ref", "Row::join_key", "Row"},
            )

    def test_a_field_access_is_not_read_as_a_method_call(self) -> None:
        # The other side of that widening: `row.value` is a field, and Rust has
        # no way to take a method by value through a receiver. Treating it as a
        # call would fire the ambiguity refusal on every field that happens to
        # share a method name.
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
            # Transitive, not one hop: `deeper` is only reachable through
            # `shape_row`, and a one-hop closure would leave it uncovered.
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
        # A macro body can call a function without naming it at the call site,
        # so from the moment one exists the closure can no longer claim to cover
        # what a cited fold runs. There are none today; this is the tripwire.
        temporary, fixture = self.fixture(
            "macro_rules! shape {\n    () => {};\n}\n" + INLINE_RENDER
        )
        with temporary:
            with self.assertRaisesRegex(
                VERIFY.VerificationError, "defines macro"
            ):
                VERIFY.index_items(fixture.world)

    def test_an_unparsed_item_scope_stops_the_gate(self) -> None:
        # A scope header this scanner cannot read would attribute the functions
        # inside it to the wrong owner, and their `self.f()` callers would stop
        # resolving -- silent under-coverage. It refuses instead.
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
        # `world_fold.rs` carries two `injected_record_fault` bodies chosen by
        # `#[cfg(..)]`. A span starting at `fn` leaves the selector outside every
        # digest, so swapping which body ships changes nothing the gate sees.
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
        # `crate::FLAG` reaches the same refusal an unreadable receiver does,
        # but that path consulted CALLABLE items only, so a constant behind an
        # unreadable qualifier fell through and vanished from the closure.
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
        # `_item_terminator` tracks `()` and `[]`; a braced const-generic
        # DEFAULT put a `{` in the header, which was mistaken for the body and
        # left the real fields outside the digest.
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
        # `{ 1 << 2 }` -- the angle-depth guard counted each `<` of the shift as
        # a generic open, so the real body brace looked nested and the struct
        # left the index. `>>` must still decrement twice or every
        # `Vec<Vec<u32>>` in the file gets the wrong span, which is how the
        # first version of this guard broke d52.
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
            # The nested generic in the return type must not have shifted the
            # function's own span either.
            self.assertEqual(index.items["WorldFold::fold_x"].span.end, 16)

    def test_an_attribute_survives_a_blank_line_above_its_item(self) -> None:
        # Rust allows `#[cfg(..)]`, a blank line or comment, then the item.
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
        # Every item pattern is line-anchored, so the later item would be
        # invisible rather than refused.
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
        # It would be accepted and would then suppress the real method's
        # ambiguity, omitting the very body it claims to describe.
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
        # `type Alias = m::Row;` aliases `Row`, not the module `m`.
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
        # `impl<'a> From<&'a Row>` / newline / `    for Projection<'a>` /
        # newline / `{` is in world_fold.rs today. Demanding the brace on the
        # header line skipped the scope and indexed `from` as a FREE function,
        # so a caller's `self.from()` would stop resolving -- silent
        # under-coverage. This fails if the brace requirement comes back.
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
        # `macro_rules ! name` is valid Rust and was accepted by the scanner.
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
        # `) -> ` / newline / `impl Iterator<..> {` begins a line with `impl`
        # but is a return type. Treating it as an item scope would mis-own every
        # function after it, and the guard would not fire because the scanner
        # "recognized" it.
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
            # The assertion that observes the false scope: without the
            # continuation check, `impl Iterator<..> {` is read as an item scope
            # that owns `rows`' body, so the nested function is indexed as
            # `Iterator::nested_helper` instead of a free one, and every caller
            # spelling it bare stops resolving.
            self.assertIn("WorldFold::nested_helper", index.items)
            self.assertNotIn("Iterator::nested_helper", index.items)

    def test_an_array_type_in_a_signature_does_not_drop_the_function(self) -> None:
        # `fn helper(values: [i32; 5]) { .. }` is a DEFINITION. Reading the
        # array type's semicolon as the terminator classified it as a bodyless
        # trait declaration and dropped it from the index entirely, so a cited
        # fold could call it and the closure would contain only the fold.
        # `validate_supply_logistics_tombstone` and `a38_hash_hex` in
        # world_fold.rs were both missing for exactly this reason.
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
        # `const PICK: i32 = if c { A } else { B };` -- stopping at the first
        # brace hashed the first branch and left the second outside the digest.
        #
        # The declaration is spread over SEVERAL LINES on purpose. Spans are
        # converted to whole lines and hashing includes the complete ending
        # line, so a one-line fixture hashes the omitted suffix anyway and the
        # assertion passes with or without the fix -- which is exactly how the
        # first version of this test managed to prove nothing.
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
        # The value is on its own lines so the truncation at the semicolon
        # INSIDE `[u8; 4]` actually omits them -- see the note above.
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
        # `type Alias = Row;` -- `Alias::helper()` executes `Row::helper`, but
        # the index stores the method under `Row`, so the `(Alias, helper)`
        # lookup missed and the method left the closure silently.
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
        # `by_name` carries constants and structs too. Treating them as callable
        # made an unambiguous `row.render()` "ambiguous" against an unrelated
        # constant of the same name -- a refusal no re-stamp can clear.
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
        # Declarations are read from the view where comments survive but string
        # literals do not, so a diagnostic quoting the annotation cannot grant
        # itself permission to omit a callee.
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
        # `if row.ready {` and `row.items[0]` are field reads. Reading them as
        # calls fires the ambiguity refusal on every field sharing a method name.
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
        # The declaration used to be keyed by METHOD NAME for the whole
        # function, so `@fold-callee-external as_str` written for two `String`
        # receivers also silently absolved a local `DivisionUid::as_str()` added
        # later. A declaration now governs its own line and the next one, so a
        # second call somewhere else in the same function is undeclared and the
        # gate refuses instead of quietly omitting the local method.
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
        # `crate::f()` / `super::f()` name a module path this scanner does not
        # model, so they reach the same refusal an unreadable receiver does
        # instead of resolving to nothing.
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
        # `DIVISION_FLAG_SUPPLY_GAIN` is a `const`. Re-spelling it `1 << 9`
        # changes which bit the division fold reads while every `fn` in the
        # closure stays byte-identical, and every digest held.
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
        # Every cited fold reads `WorldFold`'s fields directly, so widening one
        # changes what the fold does while the method's own bytes are identical.
        # The owner was a lookup key everywhere; here it is a dependency.
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
        # `use a::f;` decides WHICH `f` a bare call reaches. Re-pointing it
        # changes the executed implementation without changing any other byte.
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
        # `KNOWN_FLAGS = A | B` means nothing without A and B, and changing one
        # changes what it admits while its own bytes stay identical.
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
        # The other half of that decision: following the TYPES a struct's fields
        # name made `WorldFold` pull all 138 state structs into all 45 closures,
        # so every domain's digest moved whenever any other domain's state
        # changed and the membership list stopped describing the domain.
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
        # It is already inside that function's span. Indexing it again would
        # hash it twice and, because such names repeat freely across bodies,
        # would key them `#2`..`#13` by source ORDER -- so inserting one earlier
        # in the file renames the rest and churns membership that did not change.
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
        # A key no module reads is a coverage claim no run verifies -- the same
        # shape as a grandfathered document outside every scanned root, which
        # this gate already refuses.
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
        # Valid JSON, wrong shape. These reached `sorted()`/`set()` later and
        # came out as an AttributeError or TypeError traceback rather than the
        # controlled diagnostic the gate promises.
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
        # A tree whose markers are current but whose membership record is stale
        # still fails the gate, so `--check` calling it clean would send a lane
        # into exactly the wrong diagnosis.
        temporary, fixture = self.fixture(EXTRACTED_RENDER)
        with temporary:
            fixture.add_module(1, ("fold_render",))
            fixture.write_inventory({"1": ["WorldFold::fold_render"]})
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                self.assertEqual(
                    RESTAMP.main(["--repo-root", str(fixture.root), "--check"]), 1
                )
            # The SUMMARY is asserted, not only the exit code: inventory drift is
            # counted separately precisely so the line cannot say "1 of 0
            # modules", and an exit-code-only test would not notice that coming
            # back.
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
        # The marker needs no change, but the prose beside it does. Returning
        # "0 modules re-stamped" here would report a clean run on a tree the
        # gate rejects.
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
        # Not normalized to a plurality: a re-stamp rewrites ONE line, and every
        # other byte of the file goes back exactly as it came.
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
        # The checkout is CRLF (`core.autocrlf=true`). Reading with universal
        # newlines and writing back with `newline=""` rewrites the whole file to
        # LF -- invisible in `git diff`, which normalizes, and wrong on disk.
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
        # A refusal that had already rewritten half the markers would leave the
        # tree failing its own gate for a reason nobody chose, and would tempt
        # the next run into `--allow-closure-shrink` to clean up the mess.
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
        # The module already carries prose equal to the citation the re-stamp is
        # about to write. Rewriting leaves two copies, which the gate rejects, so
        # the tool must refuse rather than report a clean run over a tree that
        # then fails.
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
        # The gate requires the FIRST cited function to be the one
        # `fold_domain_data` dispatches to. Re-stamping such a marker produced
        # valid-looking spans, digests and inventory over an authority the gate
        # then rejected -- and because the closure GREW, the shrink refusal did
        # not catch it either.
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
        # The inventory is keyed by FILENAME domain while the marker carries its
        # own. Re-stamping over the mismatch would record membership under one
        # domain from an authority declared for another.
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
        # It re-derives WHERE an authority is, never WHICH function it is.
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
        # A first-failure-only checker reported one stale citation and hid 31
        # more, and because this check gates Cargo the whole central validation
        # was unreachable while they accumulated. One run must name them all.
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
        # The lane marker pins a capture-lane decoder, not the fold envelope.
        # Dropping it because the module's world-fold line range drifted would
        # turn one stale range into a spurious "lane authority marker set
        # drifted" and bury the real cause.
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

    # --- the free-text line-number ratchet ---------------------------------

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
            # A second use of the same number is still a NEW reference.
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
            # Citing neither is fine: the ratchet only ever turns one way.
            self.assertEqual(
                VERIFY.verify_repo(fixture.root, required_lane_authorities=()),
                (1, 0, 0, 0),
            )
            # Trading a listed number for an unlisted one is not shrinking.
            self._append_comment(fixture, 1, "// see `world_fold.rs:789`\n")
            with self.assertRaisesRegex(
                VERIFY.VerificationError, r"`world_fold\.rs:789` is a free-text"
            ):
                VERIFY.verify_repo(fixture.root, required_lane_authorities=())

    def test_the_gated_citation_is_never_counted_as_free_text(self) -> None:
        temporary, fixture = self.fixture()
        with temporary:
            fixture.add_module(1, ("fold_one",))
            # The fixture module carries only its gated `world_fold.rs:S`-`:E`
            # citation, and no allowance file exists at all.
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
        # Found by review: if every occurrence of the gated text counted as
        # owned, pasting a second unpaired copy would smuggle two unchecked line
        # numbers past the ratchet.  The digest backs one citation.
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

            # And the extraction itself must see the copy's numbers as free text.
            marker = VERIFY.WORLD_MARKER_RE.search(
                module.read_text(encoding="utf-8")
            )
            found = VERIFY.ungated_prose_references(
                module.read_text(encoding="utf-8"), marker
            )
            self.assertEqual(len(found), 1, found)

    def test_rejects_a_new_document_line_number(self) -> None:
        # Documents had NO coverage before this check: every `world_fold.rs:N`
        # in `extractor/documents` was free text nothing read.
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
            # A second citation of the same number is still a NEW reference.
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
        # The budget is per (document, line), so one document's amnesty must not
        # license the same number in another. Nested documents are reached too.
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
        # Scanning one document tree and calling it "the documents" is how this
        # gap existed in the first place: `replay_interface/documents` was
        # citing world_fold.rs by line 28 times while only
        # `extractor/documents` was read. Each configured root must be reached.
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
        # Renaming a root, moving a document out of one, or changing its
        # extension leaves the citations on disk and quietly shrinks the
        # reported count. Absence is still fine -- deleting a document must not
        # force an allowance edit in the same commit -- so the failure is
        # specifically "exists, is grandfathered, was not scanned".
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
        # The whole point of the ratchet: the replacement form is free.
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


if __name__ == "__main__":
    unittest.main()
