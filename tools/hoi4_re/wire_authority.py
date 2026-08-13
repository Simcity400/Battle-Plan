#!/usr/bin/env python3
"""Production-derived wire and early-render constants for RE verifiers.

This module belongs to the repository-level facade, which is owned by neither
tree, for the same reason the sink-grader loader does: the XN15L verifier filed
under the replay side must reach the acquisition tree's own declarations without
NAMING an acquisition path, and `replay_dependency_boundary.rs` enforces exactly
that. Every acquisition path this derivation reads is spelled here.

Why derive at all. The XN15L verifier used to restate the wire vocabulary, the
early-render receipt contract and the d77 frame schema as Python literals, and
every one of those restatements drifted: `FRAME_END` moved from record type 4 to
2 and 4 became `STREAM_HEADER`, the staged hook count moved from 62 to the
group-count sum, and the d77 frame schema moved from v12 to v13. A restated
table is a table that silently disagrees with the producer.

Nothing here is a fallback. There is no default for a constant that cannot be
found, because a default is exactly the silent disagreement this module exists
to prevent. The facts that remain restated by a consumer are listed in
`RESTATED_WITHOUT_DERIVATION`, and each one carries a cross-check test.
"""

from __future__ import annotations

import ast
import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from .acquisition import acquisition_repository_root


class AuthorityError(RuntimeError):
    """A production constant this verifier depends on was not found."""


# Facts the verifier states itself because no production source states them in a
# form that can be parsed. Each one has a cross-check test.
RESTATED_WITHOUT_DERIVATION = {
    "launch_epoch_preimage_shape": (
        "render_stream::production_render_launch_epoch_sha256 feeds the two "
        "receipt digests as their 64-char hex ASCII (`.as_bytes()`), not as 32 "
        "raw bytes; the shape of a Rust function body is not a parseable "
        "constant, so the Python reimplementation asserts it by source shape."
    ),
    "runtime_dlc_roster_preimage_shape": (
        "canonical_runtime_dlc_roster_sha256 lays its preimage out as a label, a "
        "u32 count, then per name a u32 ordinal, a u32 UTF-8 byte length and the "
        "bytes. Only the label is a constant; the layout is the function body, "
        "so the reimplementation asserts that body's shape."
    ),
    "xn15l_no_diagnostic_hook_subset": (
        "EarlyRenderCaptureReceipt::validate ACCEPTS a diagnostic staged-hook "
        "subset whose count matches its mask. XN15L refuses one outright, "
        "because a diagnostic subset is not a promotable acquisition. This is "
        "XN15L policy sitting on top of production, not a claim about it."
    ),
    "host_fold_attestation_keys": (
        "`host_finalize.fold` is assembled as a serde_json object literal in "
        "recorder_host, so its key spellings are not constants. The two the "
        "verifier reads are asserted to still occur in that source."
    ),
    "xn15l_exact_semantic_arguments": (
        "production accepts any launcher-effective argv that satisfies "
        "validate_launcher_effective_arguments; XN15L additionally requires the "
        "exact two-argument acquisition profile. The two token spellings are "
        "derived; the exactness is XN15L's own tighter policy."
    ),
}


# ---------------------------------------------------------------------------
# Rust source scraping
# ---------------------------------------------------------------------------


_INT_TOKEN = re.compile(r"\b(0x[0-9A-Fa-f_]+|[0-9][0-9_]*)(?:u8|u16|u32|u64|usize|i8|i16|i32|i64|isize)?\b")


def _normalise_expression(raw: str) -> str:
    """Rewrite a Rust constant expression into a Python-evaluable one."""

    text = raw.strip()
    # `crate::module::NAME` / `module::NAME` -> `NAME`, so a cross-module
    # reference resolves through the same name table.
    text = re.sub(r"(?:\b[A-Za-z_][A-Za-z0-9_]*\s*::\s*)+([A-Za-z_][A-Za-z0-9_]*)", r"\1", text)
    text = _INT_TOKEN.sub(lambda m: m.group(1).replace("_", ""), text)
    text = text.replace(" as usize", "").replace(" as u32", "").replace(" as u64", "")
    return text


_ALLOWED_BINOPS = (ast.Add, ast.Sub, ast.Mult, ast.LShift, ast.RShift, ast.BitOr, ast.BitAnd)


def _evaluate(expression: str, resolve: Callable[[str], int], where: str) -> int:
    try:
        tree = ast.parse(_normalise_expression(expression), mode="eval")
    except SyntaxError as error:
        raise AuthorityError(f"{where}: cannot parse constant expression {expression!r}: {error}")

    def walk(node: ast.AST) -> int:
        if isinstance(node, ast.Expression):
            return walk(node.body)
        if isinstance(node, ast.Constant) and isinstance(node.value, int) and not isinstance(node.value, bool):
            return node.value
        if isinstance(node, ast.BinOp) and isinstance(node.op, _ALLOWED_BINOPS):
            left, right = walk(node.left), walk(node.right)
            if isinstance(node.op, ast.Add):
                return left + right
            if isinstance(node.op, ast.Sub):
                return left - right
            if isinstance(node.op, ast.Mult):
                return left * right
            if isinstance(node.op, ast.LShift):
                return left << right
            if isinstance(node.op, ast.RShift):
                return left >> right
            if isinstance(node.op, ast.BitOr):
                return left | right
            return left & right
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, (ast.USub, ast.UAdd, ast.Invert)):
            operand = walk(node.operand)
            if isinstance(node.op, ast.USub):
                return -operand
            if isinstance(node.op, ast.UAdd):
                return operand
            return ~operand
        if isinstance(node, ast.Name):
            return resolve(node.id)
        raise AuthorityError(
            f"{where}: constant expression {expression!r} uses an unsupported form "
            f"({type(node).__name__}); derive it explicitly rather than guessing"
        )

    return walk(tree)


class RustSource:
    """One production Rust file, read once and scraped by constant name."""

    def __init__(self, root: Path, relative: str) -> None:
        self.relative = relative
        self.path = root / relative
        if not self.path.is_file():
            raise AuthorityError(f"production source {relative} is missing at {self.path}")
        self.text = self.path.read_text(encoding="utf-8")
        self._cache: dict[str, int] = {}

    def _declaration(self, name: str) -> str:
        pattern = re.compile(
            r"^\s*(?:pub(?:\([^)]*\))?\s+)?const\s+" + re.escape(name) + r"\s*:[^=]+=\s*(.*?);",
            re.MULTILINE | re.DOTALL,
        )
        match = pattern.search(self.text)
        if match is None:
            raise AuthorityError(
                f"{self.relative}: constant {name} is gone; the XN15L verifier depended on it"
            )
        return match.group(1)

    def integer(self, name: str, siblings: "tuple[RustSource, ...]" = ()) -> int:
        if name in self._cache:
            return self._cache[name]

        def resolve(identifier: str) -> int:
            for source in (self,) + siblings:
                try:
                    return source.integer(identifier, siblings)
                except AuthorityError:
                    continue
            raise AuthorityError(
                f"{self.relative}: constant {name} refers to {identifier}, which no "
                f"scraped production source declares"
            )

        value = _evaluate(self._declaration(name), resolve, self.relative)
        self._cache[name] = value
        return value

    def byte_string(self, name: str) -> bytes:
        declaration = self._declaration(name)
        match = re.search(r'b"((?:[^"\\]|\\.)*)"', declaration)
        if match is None:
            raise AuthorityError(f"{self.relative}: constant {name} is not a byte-string literal")
        return _decode_rust_string(match.group(1)).encode("latin-1")

    def string(self, name: str) -> str:
        declaration = self._declaration(name)
        match = re.search(r'"((?:[^"\\]|\\.)*)"', declaration)
        if match is None:
            raise AuthorityError(f"{self.relative}: constant {name} is not a string literal")
        return _decode_rust_string(match.group(1))

    def integer_array(self, name: str) -> list[int]:
        declaration = self._declaration(name)
        match = re.search(r"\[(.*)\]", declaration, re.DOTALL)
        if match is None:
            raise AuthorityError(f"{self.relative}: constant {name} is not an array literal")
        return [
            _evaluate(item, lambda i: self.integer(i), self.relative)
            for item in match.group(1).split(",")
            if item.strip()
        ]

    def struct_fields(self, name: str) -> list[str]:
        """Field names of `struct NAME`, in serde serialisation order."""

        body = self._struct_body(name)
        return re.findall(r"^\s*(?:pub\s+)?([a-z_][a-z0-9_]*)\s*:", body, re.MULTILINE)

    def struct_optional_fields(self, name: str) -> set[str]:
        """Fields carrying `skip_serializing_if`, i.e. absent when unset."""

        body = self._struct_body(name)
        optional: set[str] = set()
        pending = False
        for line in body.splitlines():
            stripped = line.strip()
            if stripped.startswith("#["):
                pending = pending or "skip_serializing_if" in stripped
                continue
            match = re.match(r"(?:pub\s+)?([a-z_][a-z0-9_]*)\s*:", stripped)
            if match is not None:
                if pending:
                    optional.add(match.group(1))
                pending = False
            elif stripped and not stripped.startswith("//"):
                pending = False
        return optional

    def struct_typed_fields(self, name: str) -> list:
        """`(field, rust_type)` pairs of `struct NAME`, in declaration order.

        Declaration order IS serialisation order for `serde_json::to_vec`, which
        is what the recorder digests. A consumer that re-encodes a receipt parsed
        from the session manifest must use this order and not the order the file
        happens to carry: the manifest embeds the receipt through
        `serde_json::to_value`, and without serde_json's `preserve_order`
        feature that produces a `BTreeMap` -- alphabetical, not declaration
        order -- while the recorded digest is taken over the struct.
        """

        body = self._struct_body(name)
        fields = []
        for line in _strip_rust_comments(body).splitlines():
            stripped = line.strip()
            match = re.match(r"(?:pub\s+)?([a-z_][a-z0-9_]*)\s*:\s*(.+?),?\s*$", stripped)
            if match is None or stripped.startswith("#["):
                continue
            fields.append((match.group(1), match.group(2).rstrip(",").strip()))
        return fields

    def declares_struct(self, name: str) -> bool:
        try:
            self._struct_body(name)
        except AuthorityError:
            return False
        return True

    def _struct_body(self, name: str) -> str:
        pattern = re.compile(r"\bstruct\s+" + re.escape(name) + r"\b[^{;]*\{", re.MULTILINE)
        match = pattern.search(self.text)
        if match is None:
            raise AuthorityError(f"{self.relative}: struct {name} is gone")
        depth = 0
        for index in range(match.end() - 1, len(self.text)):
            character = self.text[index]
            if character == "{":
                depth += 1
            elif character == "}":
                depth -= 1
                if depth == 0:
                    return self.text[match.end() : index]
        raise AuthorityError(f"{self.relative}: struct {name} is unterminated")

    # (see `_balanced` below for the shared brace matcher)

    def string_list_after(self, anchor: str, count: int) -> list[str]:
        """The first `count` string literals following `anchor`."""

        position = self.text.find(anchor)
        if position < 0:
            raise AuthorityError(f"{self.relative}: anchor {anchor!r} is gone")
        found = re.findall(r'"((?:[^"\\]|\\.)*)"', self.text[position : position + 4000])
        if len(found) < count:
            raise AuthorityError(
                f"{self.relative}: anchor {anchor!r} is followed by {len(found)} string "
                f"literals, fewer than the {count} the verifier reads"
            )
        return [_decode_rust_string(item) for item in found[:count]]

    def function_body(self, name: str, within: str | None = None) -> str:
        """The body of `fn name`, optionally scoped to one `impl` block.

        `within` matters: `early_render_launch.rs` declares several `fn
        validate`, and picking the first would read a different type's contract.
        """

        haystack = self.text if within is None else self.impl_body(within)
        where = self.relative if within is None else f"{self.relative} impl {within}"
        pattern = re.compile(r"\bfn\s+" + re.escape(name) + r"\b")
        for match in pattern.finditer(haystack):
            # A signature may carry `;` inside a return type such as `[u8; 32]`,
            # so the body's opening brace is found by bracket depth rather than
            # by "no semicolon before it".
            depth = 0
            for index in range(match.end(), len(haystack)):
                character = haystack[index]
                # Only `()` and `[]` are tracked: a generic `<...>` never holds a
                # `;`, while `->` would otherwise unbalance the depth.
                if character in "([":
                    depth += 1
                elif character in ")]":
                    depth -= 1
                elif depth == 0 and character == ";":
                    break  # a declaration without a body
                elif depth == 0 and character == "{":
                    return self._balanced(haystack, index, f"{where}: fn {name}")
        raise AuthorityError(f"{where}: fn {name} is gone")

    def impl_body(self, type_name: str) -> str:
        pattern = re.compile(r"^impl\s+" + re.escape(type_name) + r"\s*\{", re.MULTILINE)
        match = pattern.search(self.text)
        if match is None:
            raise AuthorityError(f"{self.relative}: impl {type_name} is gone")
        return self._balanced(self.text, match.end() - 1, f"{self.relative}: impl {type_name}")

    def _balanced(self, haystack: str, open_index: int, where: str) -> str:
        depth = 0
        for index in range(open_index, len(haystack)):
            character = haystack[index]
            if character == "{":
                depth += 1
            elif character == "}":
                depth -= 1
                if depth == 0:
                    return haystack[open_index + 1 : index]
        raise AuthorityError(f"{where} is unterminated")


def _strip_rust_comments(text: str) -> str:
    """Drop `//`-style lines so a doc comment is never read as a field."""

    return "\n".join(
        line for line in text.splitlines() if not line.strip().startswith("//")
    )


def _decode_rust_string(raw: str) -> str:
    out: list[str] = []
    index = 0
    while index < len(raw):
        character = raw[index]
        if character != "\\":
            out.append(character)
            index += 1
            continue
        index += 1
        escape = raw[index]
        if escape == "0":
            out.append("\0")
            index += 1
        elif escape == "n":
            out.append("\n")
            index += 1
        elif escape == "t":
            out.append("\t")
            index += 1
        elif escape == "r":
            out.append("\r")
            index += 1
        elif escape == "x":
            out.append(chr(int(raw[index + 1 : index + 3], 16)))
            index += 3
        else:
            out.append(escape)
            index += 1
    return "".join(out)


# ---------------------------------------------------------------------------
# The derived authority
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class WireAuthority:
    """Everything the XN15L verifier needs, read from the production sources."""

    repository_root: Path

    # H4CR framing.
    record_magic: bytes
    record_version: int
    record_header_len: int
    max_payload_len: int

    # Record-type vocabulary, name -> id, exactly as production declares it.
    record_types: dict[str, int]
    frame_scoped_record_types: frozenset
    domain_frame: int
    domain_stream_header: int

    # Header flags.
    flag_payload_schema_v2: int
    flag_perf_diagnostic_trailer: int
    payload_schema_v2_domains: frozenset

    # Payload family grammar the envelope check applies.
    frame_end_payload_len: int
    domain_gap_payload_len: int
    domain_gap_magic: bytes
    domain_gap_version: int
    perf_base_len: int
    cid1_magic: bytes
    cid1_version: int
    perf_reserved_word_offset: int
    perf_trailer_magic: bytes
    perf_trailer_versions: frozenset
    frame_refusal_magic: bytes
    frame_refusal_versions: frozenset
    frame_refusal_header_len: int
    frame_degraded_magic: bytes
    frame_degraded_versions: frozenset
    frame_degraded_header_len: int
    capture_termination_magic: bytes
    capture_termination_versions: frozenset
    capture_termination_header_len: int
    clock_unreadable_magic: bytes
    clock_unreadable_versions: frozenset
    clock_unreadable_payload_len: int
    untrusted_clock_magic: bytes
    untrusted_clock_versions: frozenset
    untrusted_clock_payload_len: int

    # Storage container.
    zstd_frame_magic: bytes
    released_prefix_sidecar_suffix: str
    trim_journal_schema: str
    trim_journal_layout: tuple
    compression_tokens: dict

    # Calendar.
    date_epoch_const: int

    # d77 / render stream.
    d77_domain_id: int
    d77_domain_name: str
    render_frame_schema: str
    render_frame_incompatible_schemas: frozenset
    launch_epoch_label: bytes

    # Early-render contract.
    early_launch_manifest_schema: str
    early_capture_receipt_schema: str
    early_staged_hook_generation: int
    early_staged_hook_count: int
    early_hook_completeness_mask: int
    early_completeness_fields: tuple
    early_receipt_fields: tuple
    early_receipt_optional_fields: frozenset
    early_transcript_events: tuple
    early_transcript_fields: tuple
    #: Declaration-order layouts for every struct that enters a digest.
    struct_layouts: dict
    runtime_dlc_schema: str
    runtime_dlc_source: str
    runtime_dlc_sample_count: int
    runtime_dlc_roster_label: bytes
    runtime_dlc_manager_getter_rva: int
    runtime_dlc_enumerator_rva: int
    runtime_dlc_teardown_rva: int
    launcher_continue_argument: str
    launcher_gdpr_argument: str
    #: Serialised layer-kind token -> its VFS phase ordinal, from the production
    #: `match` that assigns them. The ORDER is the contract, not the names.
    launch_layer_phases: dict
    launch_layer_base_token: str
    requested_profile_selection_schema: str

    # Raw source text, kept so the cross-check tests can assert shapes that are
    # not expressible as constants.
    sources: dict = field(default_factory=dict, repr=False)

    def record_type_name(self, record_type: int) -> str | None:
        for name, value in self.record_types.items():
            if value == record_type:
                return name
        return None

    def allowed_record_flags(self, domain_id: int, record_type: int) -> int:
        """Mirror of `compact_record_allowed_flags`."""

        allowed = 0
        if record_type == self.record_types["DOMAIN_PERF"]:
            allowed |= self.flag_perf_diagnostic_trailer
        if record_type == self.record_types["DOMAIN_DATA"] and domain_id in self.payload_schema_v2_domains:
            allowed |= self.flag_payload_schema_v2
        return allowed

    def rejected_record_flags(self, domain_id: int, record_type: int, record_flags: int) -> int:
        return record_flags & ~self.allowed_record_flags(domain_id, record_type)


_CONTRACT = "extractor/native/compact_contract/src"
_HOST = "extractor/native/recorder_host/src"

#: The production function whose preimage `launch_epoch_sha256` reproduces.
LAUNCH_EPOCH_FUNCTION = "compute_render_launch_epoch_sha256"
#: The production function whose preimage `runtime_dlc_roster_sha256` reproduces.
RUNTIME_DLC_ROSTER_FUNCTION = "canonical_runtime_dlc_roster_sha256"


def load_wire_authority(root: Path | None = None) -> WireAuthority:
    """Read every production constant the XN15L verifier depends on."""

    root = Path(root) if root is not None else Path(acquisition_repository_root())

    header = RustSource(root, f"{_CONTRACT}/record_header.rs")
    semantics = RustSource(root, f"{_CONTRACT}/record_semantics.rs")
    identity = RustSource(root, f"{_CONTRACT}/campaign_identity.rs")
    refusal = RustSource(root, f"{_CONTRACT}/frame_refusal.rs")
    degraded = RustSource(root, f"{_CONTRACT}/frame_degraded.rs")
    termination = RustSource(root, f"{_CONTRACT}/capture_termination.rs")
    clock_gap = RustSource(root, f"{_CONTRACT}/clock_unreadable_interval.rs")
    clock_untrusted = RustSource(root, f"{_CONTRACT}/untrusted_clock_recovery.rs")
    calendar = RustSource(root, f"{_CONTRACT}/calendar.rs")
    early_control = RustSource(root, f"{_CONTRACT}/early_render_control.rs")
    early_dlc = RustSource(root, f"{_CONTRACT}/early_render_runtime_dlc.rs")
    launch = RustSource(root, f"{_HOST}/early_render_launch.rs")
    dlc_roster = RustSource(root, f"{_HOST}/runtime_dlc_roster.rs")
    render = RustSource(root, "portable_replay/native/render_stream/src/lib.rs")
    storage = RustSource(root, "portable_replay/native/compact_stream/src/lib.rs")
    spool = RustSource(root, f"{_HOST}/spool_trim.rs")

    record_types = _record_types(semantics, identity)
    d77_id, d77_name = _d77_from_registry(root)

    return WireAuthority(
        repository_root=root,
        record_magic=header.byte_string("COMPACT_RECORD_MAGIC"),
        record_version=header.integer("COMPACT_RECORD_VERSION"),
        record_header_len=header.integer("COMPACT_RECORD_HEADER_LEN"),
        max_payload_len=header.integer("COMPACT_MAX_PAYLOAD_LEN"),
        record_types=record_types,
        frame_scoped_record_types=_frame_scoped_record_types(semantics),
        domain_frame=semantics.integer("COMPACT_DOMAIN_FRAME"),
        domain_stream_header=identity.integer("COMPACT_DOMAIN_STREAM_HEADER"),
        flag_payload_schema_v2=semantics.integer("COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2"),
        flag_perf_diagnostic_trailer=semantics.integer("COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER"),
        payload_schema_v2_domains=_payload_schema_v2_domains(semantics),
        frame_end_payload_len=semantics.integer("COMPACT_FRAME_END_PAYLOAD_LEN"),
        domain_gap_payload_len=semantics.integer("COMPACT_DOMAIN_GAP_PAYLOAD_LEN"),
        domain_gap_magic=semantics.byte_string("COMPACT_DOMAIN_GAP_MAGIC"),
        domain_gap_version=semantics.integer("COMPACT_DOMAIN_GAP_VERSION"),
        perf_base_len=semantics.integer("COMPACT_DOMAIN_PERF_BASE_LEN"),
        cid1_magic=identity.byte_string("CID1_MAGIC"),
        cid1_version=identity.integer("CID1_VERSION"),
        perf_reserved_word_offset=_perf_reserved_word_offset(semantics),
        perf_trailer_magic=semantics.byte_string("COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC"),
        perf_trailer_versions=frozenset(
            {
                semantics.integer("COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION"),
                semantics.integer("COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2"),
            }
        ),
        frame_refusal_magic=refusal.byte_string("FRAME_REFUSAL_MAGIC"),
        frame_refusal_versions=frozenset({refusal.integer("FRAME_REFUSAL_VERSION")}),
        frame_refusal_header_len=refusal.integer("FRAME_REFUSAL_HEADER_LEN"),
        frame_degraded_magic=degraded.byte_string("FRAME_DEGRADED_MAGIC"),
        frame_degraded_versions=frozenset({degraded.integer("FRAME_DEGRADED_VERSION")}),
        frame_degraded_header_len=degraded.integer("FRAME_DEGRADED_HEADER_LEN"),
        capture_termination_magic=termination.byte_string("CAPTURE_TERMINATION_MAGIC"),
        capture_termination_versions=frozenset({termination.integer("CAPTURE_TERMINATION_VERSION")}),
        capture_termination_header_len=termination.integer("CAPTURE_TERMINATION_HEADER_LEN"),
        clock_unreadable_magic=clock_gap.byte_string("CLOCK_UNREADABLE_INTERVAL_MAGIC"),
        clock_unreadable_versions=frozenset({clock_gap.integer("CLOCK_UNREADABLE_INTERVAL_VERSION")}),
        clock_unreadable_payload_len=clock_gap.integer("CLOCK_UNREADABLE_INTERVAL_PAYLOAD_LEN"),
        untrusted_clock_magic=clock_untrusted.byte_string("UNTRUSTED_CLOCK_RECOVERY_MAGIC"),
        untrusted_clock_versions=frozenset(
            {clock_untrusted.integer("UNTRUSTED_CLOCK_RECOVERY_VERSION")}
        ),
        untrusted_clock_payload_len=clock_untrusted.integer("UNTRUSTED_CLOCK_RECOVERY_PAYLOAD_LEN"),
        zstd_frame_magic=_zstd_magic(storage),
        released_prefix_sidecar_suffix=storage.string("RELEASED_PREFIX_SIDECAR_SUFFIX"),
        trim_journal_schema=spool.string("TRIM_JOURNAL_SCHEMA"),
        trim_journal_layout=tuple(spool.struct_typed_fields("TrimJournal")),
        compression_tokens=_compression_tokens(storage),
        date_epoch_const=calendar.integer("DATE_EPOCH_CONST"),
        d77_domain_id=d77_id,
        d77_domain_name=d77_name,
        render_frame_schema=render.string("SCHEMA"),
        render_frame_incompatible_schemas=_incompatible_render_schemas(render),
        launch_epoch_label=_literal_label(
            render, LAUNCH_EPOCH_FUNCTION, "hoi4.production-render-launch-epoch"
        ),
        early_launch_manifest_schema=launch.string("EARLY_LAUNCH_MANIFEST_SCHEMA"),
        early_capture_receipt_schema=launch.string("EARLY_CAPTURE_RECEIPT_SCHEMA"),
        early_staged_hook_generation=early_control.integer("EARLY_STAGED_HOOK_GENERATION"),
        early_staged_hook_count=early_control.integer("EARLY_STAGED_HOOK_COUNT"),
        early_hook_completeness_mask=early_control.integer("EARLY_HOOK_COMPLETENESS_MASK"),
        early_completeness_fields=tuple(launch.struct_fields("EarlyHookCompleteness")),
        early_receipt_fields=tuple(launch.struct_fields("EarlyRenderCaptureReceipt")),
        early_receipt_optional_fields=frozenset(
            launch.struct_optional_fields("EarlyRenderCaptureReceipt")
        ),
        early_transcript_events=tuple(_transcript_events(launch)),
        early_transcript_fields=tuple(launch.struct_fields("Transcript")),
        struct_layouts=_struct_layouts(
            launch,
            ("EarlyRenderCaptureReceipt", "EarlyRenderLaunchManifest", "Transcript"),
        ),
        runtime_dlc_schema=dlc_roster.string("EFFECTIVE_RUNTIME_DLC_SCHEMA"),
        runtime_dlc_source=_runtime_dlc_source(launch),
        runtime_dlc_sample_count=_runtime_dlc_sample_count(launch),
        runtime_dlc_roster_label=_literal_label(
            early_dlc, RUNTIME_DLC_ROSTER_FUNCTION, "hoi4-a729-runtime-dlc-roster"
        ),
        runtime_dlc_manager_getter_rva=early_dlc.integer("A729_DLC_MANAGER_GETTER_RVA"),
        runtime_dlc_enumerator_rva=early_dlc.integer("A729_ACTIVE_DLC_ENUMERATOR_RVA"),
        runtime_dlc_teardown_rva=early_dlc.integer("A729_STRING_VECTOR_TEARDOWN_RVA"),
        launcher_continue_argument=_launcher_argument(launch, "continuelastsave"),
        launcher_gdpr_argument=_launcher_argument(launch, "gdpr-compliant"),
        launch_layer_phases=_launch_layer_phases(launch),
        launch_layer_base_token=_snake_case("Base"),
        requested_profile_selection_schema=dlc_roster.string(
            "REQUESTED_PROFILE_SELECTION_SCHEMA"
        ),
        sources={
            "record_semantics": semantics,
            "render_stream": render,
            "early_render_launch": launch,
            "early_render_control": early_control,
            "early_render_runtime_dlc": early_dlc,
            "compact_stream": storage,
            "spool_trim": spool,
            # Read lazily-by-name so a consumer can cross-check a JSON key
            # spelling the recorder emits without naming the file itself.
            "recorder_main": RustSource(root, f"{_HOST}/main.rs"),
        },
    )


def _record_types(semantics: RustSource, identity: RustSource) -> dict[str, int]:
    """Every `COMPACT_RECORD_TYPE_*` production declares, from both modules.

    The vocabulary is read as a SET rather than a fixed list, so a record type
    added to production shows up here rather than reaching the verifier as an
    unknown shape it silently walks past.
    """

    found: dict[str, int] = {}
    for source in (semantics, identity):
        for name in re.findall(r"const\s+(COMPACT_RECORD_TYPE_[A-Z0-9_]+)\s*:", source.text):
            short = name[len("COMPACT_RECORD_TYPE_") :]
            value = source.integer(name)
            if found.get(short, value) != value:
                raise AuthorityError(
                    f"record type {short} is declared twice with different values "
                    f"({found[short]} and {value})"
                )
            found[short] = value
    required = {"DOMAIN_PERF", "FRAME_END", "DOMAIN_DATA", "STREAM_HEADER", "DOMAIN_GAP", "FRAME_REFUSED"}
    missing = required - set(found)
    if missing:
        raise AuthorityError(f"production no longer declares record type(s) {sorted(missing)}")
    if len(set(found.values())) != len(found):
        raise AuthorityError("two production record types share one id")
    return found


def _payload_schema_v2_domains(semantics: RustSource) -> frozenset:
    """The domains whose DOMAIN_DATA may carry `PAYLOAD_SCHEMA_V2`.

    Production states this inside `compact_record_allowed_flags` as a
    `matches!` arm rather than as a constant, so the arm itself is read.
    """

    body = semantics.function_body("compact_record_allowed_flags")
    match = re.search(r"matches!\s*\(\s*domain_id\s*,([^)]*)\)", body)
    if match is None:
        raise AuthorityError(
            "record_semantics.rs: compact_record_allowed_flags no longer selects the "
            "PAYLOAD_SCHEMA_V2 domains with a matches! arm; re-derive it"
        )
    domains = {
        int(token.strip())
        for token in match.group(1).split("|")
        if token.strip().isdigit()
    }
    if not domains:
        raise AuthorityError("record_semantics.rs: the PAYLOAD_SCHEMA_V2 domain arm parsed empty")
    return frozenset(domains)


def _zstd_magic(storage: RustSource) -> bytes:
    values = storage.integer_array("ZSTD_FRAME_MAGIC")
    if len(values) != 4 or any(not 0 <= value <= 0xFF for value in values):
        raise AuthorityError("compact_stream: ZSTD_FRAME_MAGIC is not four bytes")
    return bytes(values)


#: One serialisable member of a production struct.
#:
#: `kind` is what the encoder has to do with the value, not what Rust calls it:
#: ``scalar`` is emitted as-is, ``struct`` recurses into `element`'s layout,
#: ``vec`` recurses per item, and ``map`` is a `BTreeMap`, whose members serde
#: emits in SORTED key order regardless of how a parsed file happened to carry
#: them.
@dataclass(frozen=True)
class FieldLayout:
    name: str
    kind: str
    element: str | None
    optional: bool


def _classify_rust_type(rust_type: str, source: RustSource) -> tuple:
    # `&'a EffectiveRuntimeDlcReceipt` is the same serialised shape as the owned
    # struct; leaving the borrow on would classify it as an opaque scalar and
    # silently re-encode its members in the wrong order.
    text = re.sub(r"^&\s*(?:'[a-z_][a-z0-9_]*\s+)?", "", rust_type.strip()).strip()
    optional = False
    match = re.match(r"Option\s*<(.+)>$", text)
    if match is not None:
        optional = True
        text = match.group(1).strip()
    match = re.match(r"(?:Vec|BTreeSet|HashSet)\s*<(.+)>$", text)
    if match is not None:
        inner, _ = _classify_rust_type(match.group(1), source)
        return FieldLayout("", "vec", inner.element if inner.kind == "struct" else None, optional), optional
    match = re.match(r"(?:BTreeMap|HashMap)\s*<[^,]+,(.+)>$", text)
    if match is not None:
        inner, _ = _classify_rust_type(match.group(1), source)
        return FieldLayout("", "map", inner.element if inner.kind == "struct" else None, optional), optional
    bare = text.split("::")[-1].strip()
    if re.fullmatch(r"[A-Z][A-Za-z0-9]*", bare) and source.declares_struct(bare):
        return FieldLayout("", "struct", bare, optional), optional
    return FieldLayout("", "scalar", None, optional), optional


def _struct_layouts(source: RustSource, roots: tuple) -> dict:
    """Declaration-order layouts for `roots` and every struct they reach.

    This is what lets a consumer reproduce `serde_json::to_vec` byte-for-byte
    from a receipt parsed out of the session manifest. Without it the nested
    members are re-encoded in whatever order the file carried, and every
    receipt digest comparison is a coin toss.
    """

    layouts: dict[str, tuple] = {}
    pending = list(roots)
    while pending:
        name = pending.pop()
        if name in layouts:
            continue
        optional_fields = source.struct_optional_fields(name)
        fields = []
        for field, rust_type in source.struct_typed_fields(name):
            layout, _ = _classify_rust_type(rust_type, source)
            fields.append(
                FieldLayout(field, layout.kind, layout.element, field in optional_fields)
            )
            if layout.element is not None and layout.element not in layouts:
                pending.append(layout.element)
        if not fields:
            raise AuthorityError(f"{source.relative}: struct {name} parsed with no members")
        layouts[name] = tuple(fields)
    return layouts


def _perf_reserved_word_offset(semantics: RustSource) -> int:
    """Where `validate_domain_perf` requires the reserved base word to be zero."""

    body = semantics.function_body("validate_domain_perf")
    match = re.search(r"payload\[(\d+)\.\.(\d+)\]\.try_into\(\)[^)]*\)\s*\)\s*!=\s*0", body)
    if match is None:
        match = re.search(r"payload\[(\d+)\.\.(\d+)\]", body)
    if match is None:
        raise AuthorityError(
            "record_semantics.rs: validate_domain_perf no longer names a reserved base word"
        )
    return int(match.group(1))


def _frame_scoped_record_types(semantics: RustSource) -> frozenset:
    """Record types production requires to ride `COMPACT_DOMAIN_FRAME`.

    Read out of `validate_compact_record_semantics` rather than listed here: the
    match arm and the domain guard are the rule, so a record type that gains or
    loses the guard moves this set with it.
    """

    body = semantics.function_body("validate_compact_record_semantics")
    scoped = set()
    for arm, tail in re.findall(
        r"COMPACT_RECORD_TYPE_([A-Z0-9_]+)\s*=>\s*\{(.*?)\n        \}", body, re.DOTALL
    ):
        if "domain_id != COMPACT_DOMAIN_FRAME" in tail:
            scoped.add(arm)
    if not scoped:
        raise AuthorityError(
            "record_semantics.rs: validate_compact_record_semantics no longer guards any "
            "record type against COMPACT_DOMAIN_FRAME; re-derive the frame-scoped set"
        )
    return frozenset(scoped)


def _compression_tokens(storage: RustSource) -> dict:
    """`StreamCompression::parse`'s token table, token -> canonical name.

    Read rather than restated because a spool trim journal records the storage
    encoding as one of these tokens, and defaulting an unrecognised one to raw
    is precisely the silent misread the production guard refuses.
    """

    body = storage.function_body("parse", within="StreamCompression")
    tokens: dict[str, str] = {}
    for arms, variant in re.findall(r"((?:\"[a-z]+\"\s*\|?\s*)+)=>\s*Some\(Self::(\w+)\)", body):
        for token in re.findall(r'"([a-z]+)"', arms):
            tokens[token] = variant.lower()
    if "zstd" not in tokens or "none" not in tokens:
        raise AuthorityError(
            f"compact_stream: StreamCompression::parse no longer names both storage "
            f"encodings; parsed {sorted(tokens)}"
        )
    return tokens


def _incompatible_render_schemas(render: RustSource) -> frozenset:
    names = re.findall(r"const\s+(INCOMPATIBLE_[A-Z0-9_]*SCHEMA)\s*:", render.text)
    if not names:
        raise AuthorityError("render_stream: no INCOMPATIBLE_*_SCHEMA constants remain")
    return frozenset(render.string(name) for name in names)


def _literal_label(source: RustSource, function: str, expected_prefix: str) -> bytes:
    body = source.function_body(function)
    match = re.search(r'b"((?:[^"\\]|\\.)*)"', body)
    if match is None:
        raise AuthorityError(f"{source.relative}: fn {function} carries no labelled preimage literal")
    label = _decode_rust_string(match.group(1)).encode("utf-8")
    if not label.startswith(expected_prefix.encode("utf-8")):
        raise AuthorityError(
            f"{source.relative}: fn {function} labels its preimage {label!r}, which is not "
            f"the {expected_prefix!r} family the verifier reproduces"
        )
    return label


def _transcript_events(launch: RustSource) -> list[str]:
    body = launch.function_body("expected_transcript_sha256")
    # The struct field declares `[&'static str; N]`; only the initializer holds
    # literals, so the match is anchored on the opening string.
    match = re.search(r"events\s*:\s*\[\s*(\".*?)\]", body, re.DOTALL)
    if match is None:
        raise AuthorityError(
            "early_render_launch.rs: expected_transcript_sha256 no longer builds an `events` array"
        )
    events = [_decode_rust_string(item) for item in re.findall(r'"((?:[^"\\]|\\.)*)"', match.group(1))]
    declared = re.search(r"events\s*:\s*\[&'static str;\s*(\d+)\]", body)
    if declared is not None and len(events) != int(declared.group(1)):
        raise AuthorityError(
            f"early_render_launch.rs: transcript declares {declared.group(1)} events but "
            f"{len(events)} literals were read"
        )
    if not events:
        raise AuthorityError("early_render_launch.rs: the transcript event list parsed empty")
    return events


def _runtime_dlc_source(launch: RustSource) -> str:
    body = launch.function_body("validate", within="EffectiveRuntimeDlcReceipt")
    match = re.search(r'self\.source\s*!=\s*"((?:[^"\\]|\\.)*)"', body)
    if match is None:
        raise AuthorityError(
            "early_render_launch.rs: EffectiveRuntimeDlcReceipt::validate no longer pins `source`"
        )
    return _decode_rust_string(match.group(1))


def _runtime_dlc_sample_count(launch: RustSource) -> int:
    body = launch.function_body("validate", within="EffectiveRuntimeDlcReceipt")
    match = re.search(r"self\.sample_count\s*!=\s*(\d+)", body)
    if match is None:
        raise AuthorityError(
            "early_render_launch.rs: EffectiveRuntimeDlcReceipt::validate no longer pins "
            "`sample_count`"
        )
    return int(match.group(1))


def _snake_case(variant: str) -> str:
    """serde's `rename_all = "snake_case"` spelling of an enum variant."""

    return re.sub(r"(?<!^)(?=[A-Z])", "_", variant).lower()


def _launch_layer_phases(launch: RustSource) -> dict:
    """Layer-kind token -> VFS phase ordinal, from the production `match`.

    The launch manifest's layer ORDER is a contract (base, then integrated DLC,
    then DLC, then mods), and the ordinals that express it live in a match arm
    rather than in constants. Restating them would let the order drift silently.
    """

    body = launch.function_body("validate", within="EarlyRenderLaunchManifest")
    phases = {}
    # A braced arm puts its ordinal last inside the block; a bare arm is the
    # ordinal itself. Both shapes are read rather than one being assumed.
    for variant, braced, bare in re.findall(
        r"EarlyLaunchLayerKind::(\w+)\s*=>\s*(?:\{([^}]*?)\}|(\d+))", body, re.DOTALL
    ):
        digits = re.findall(r"\b(\d+)\b", braced) if braced else [bare]
        if not digits:
            continue
        phases[_snake_case(variant)] = int(digits[-1])
    if len(phases) < 2 or len(set(phases.values())) != len(phases):
        raise AuthorityError(
            f"early_render_launch.rs: the launch-layer phase ordering no longer parses "
            f"({phases}); re-derive it"
        )
    return phases


def _launcher_argument(launch: RustSource, needle: str) -> str:
    body = launch.function_body("validate_launcher_effective_arguments")
    for literal in re.findall(r'eq_ignore_ascii_case\("((?:[^"\\]|\\.)*)"\)', body):
        token = _decode_rust_string(literal)
        if token == f"--{needle}":
            return token
    raise AuthorityError(
        f"early_render_launch.rs: validate_launcher_effective_arguments no longer names --{needle}"
    )


def _d77_from_registry(root: Path) -> tuple:
    """d77's id and name, from the generated domain registry.

    The registry is the same authority `verify_sink_reconciliation.py` reads for
    the production roster, so the verifier and the sink grader cannot disagree
    about which domain d77 is.
    """

    path = root / "extractor/native/domain_registry.json"
    if not path.is_file():
        raise AuthorityError(f"the generated domain registry is missing at {path}")
    registry = json.loads(path.read_text(encoding="utf-8"))
    expected_schema = "battle-plan.compact_domain_registry.v1"
    if registry.get("schema") != expected_schema:
        raise AuthorityError(
            f"domain_registry.json schema is {registry.get('schema')!r}, not {expected_schema!r}"
        )
    rows = registry.get("domains")
    if not isinstance(rows, list):
        raise AuthorityError("domain_registry.json carries no `domains` list")
    matches = [
        row
        for row in rows
        if isinstance(row, dict) and row.get("name") == "production_render_submits_live"
    ]
    if len(matches) != 1:
        raise AuthorityError(
            f"domain_registry.json names production_render_submits_live {len(matches)} times; "
            f"the XN15L verifier needs exactly one"
        )
    domain_id = matches[0].get("id")
    if not isinstance(domain_id, int) or isinstance(domain_id, bool) or not 0 < domain_id <= 0xFFFF:
        raise AuthorityError("domain_registry.json gives production_render_submits_live no valid id")
    return domain_id, "production_render_submits_live"


def describe_wire_authority(authority: WireAuthority) -> dict[str, Any]:
    """A JSON-safe echo of what was derived, for the verifier's evidence block."""

    return {
        "record_types": dict(sorted(authority.record_types.items(), key=lambda row: row[1])),
        "domain_frame": authority.domain_frame,
        "d77_domain_id": authority.d77_domain_id,
        "render_frame_schema": authority.render_frame_schema,
        "frame_refusal_versions": sorted(authority.frame_refusal_versions),
        "payload_schema_v2_domains": sorted(authority.payload_schema_v2_domains),
        "early_staged_hook_count": authority.early_staged_hook_count,
        "early_completeness_field_count": len(authority.early_completeness_fields),
        "date_epoch_const": authority.date_epoch_const,
        "restated_without_derivation": sorted(RESTATED_WITHOUT_DERIVATION),
    }
