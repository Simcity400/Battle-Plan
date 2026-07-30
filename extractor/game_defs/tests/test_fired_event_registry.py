from __future__ import annotations

import sys
from pathlib import Path

_DEFS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(_DEFS))

import compile_fired_event_registry as registry
import resolver as R


def _compile(tmp_path, files: dict[str, str]):
    vanilla = tmp_path / "vanilla"
    events = vanilla / "events"
    events.mkdir(parents=True)
    for name, body in files.items():
        (events / name).write_text(body, encoding="utf-8")
    layers = R.build_layers(vanilla=str(vanilla), mods=[])
    return registry.compile_registry(layers, registry.PINNED_EXE_SHA256)


def test_events_are_keyed_by_namespace_and_numeric_id_not_an_engine_number(tmp_path):
    """Every event row carries the engine-owned key parts and no id number.

    Fails if the compiler goes back to emitting a re-derived numeric event id
    (``combined_id``) or drops either half of the namespace/numeric key.
    """
    result = _compile(
        tmp_path,
        {
            "a.txt": (
                "add_namespace = alpha\n"
                "country_event = { id = alpha.7 desc = alpha.7.d"
                " option = { name = alpha.7.a } }\n"
            ),
            "b.txt": (
                "add_namespace = alpha\n"
                "add_namespace = beta\n"
                "news_event = { id = BETA.3 picture = GFX_beta desc = beta.3.d }\n"
            ),
        },
    )
    assert result["schema"] == "hoi4-xui21-fired-event-registry-v2"
    assert [
        (row["namespace"], row["numeric_id"], row["event_id"])
        for row in result["events"]
    ] == [
        ("alpha", 7, "alpha.7"),
        ("BETA", 3, "BETA.3"),
    ]
    for row in result["events"]:
        assert "combined_id" not in row
    assert result["events"][0]["option_count"] == 1
    assert result["events"][1]["picture"] == "GFX_beta"


def test_namespace_provenance_carries_no_ordinal(tmp_path):
    """The namespace list records declaration order, never an engine slot.

    Fails if the compiler reintroduces an ``ordinal``/index field that another
    tool could mistake for the engine's namespace slot.
    """
    result = _compile(
        tmp_path,
        {
            "a.txt": "add_namespace = alpha\n",
            "b.txt": "add_namespace = beta\n",
        },
    )
    assert result["namespaces"] == [
        {"namespace": "alpha", "source_file": "events/a.txt"},
        {"namespace": "beta", "source_file": "events/b.txt"},
    ]


def test_leading_zero_numeric_ids_keep_their_declared_spelling(tmp_path):
    """``alpha.00`` keys on numeric 0 while ``event_id`` stays verbatim.

    The engine reduces the suffix to a number, so the key must too; the replay
    join still needs the declared string. Fails if the compiler keys on the raw
    suffix text or rewrites ``event_id`` from the parsed number.
    """
    result = _compile(
        tmp_path,
        {
            "a.txt": (
                "add_namespace = alpha\n"
                "country_event = { id = alpha.00 desc = alpha.00.d }\n"
                "country_event = { id = alpha.012 desc = alpha.012.d }\n"
            )
        },
    )
    assert [
        (row["numeric_id"], row["event_id"]) for row in result["events"]
    ] == [(0, "alpha.00"), (12, "alpha.012")]
    assert result["meta"]["event_key_collisions_last_wins"] == []


def test_non_decimal_numeric_suffix_is_aliased_not_guessed(tmp_path):
    """``alpha.02_north`` leaves its own namespace and is listed as an alias.

    The engine accumulates every suffix byte in base ten, giving 7390336. Only
    that reduction is recorded: where the id lands depends on the declaring
    namespace's live slot and a 32-bit wrap. Fails if the compiler admits the
    event into the direct key space under a key it cannot be reached by, drops
    it so the live join cannot see the ambiguity it creates, or pre-splits the
    reduction into a slot offset and a remainder.
    """
    result = _compile(
        tmp_path,
        {
            "a.txt": (
                "add_namespace = alpha\n"
                "country_event = { id = alpha.02_north desc = alpha.02_north.d }\n"
                "country_event = { id = alpha.2 desc = alpha.2.d }\n"
            )
        },
    )
    assert [row["event_id"] for row in result["events"]] == ["alpha.2"]
    assert [
        (row["event_id"], row["namespace"], row["reduced_id"])
        for row in result["aliased_events"]
    ] == [("alpha.02_north", "alpha", 7_390_336)]
    assert result["meta"]["unresolved_event_ids"] == []


def test_out_of_span_numeric_id_is_aliased(tmp_path):
    """``alpha.100000`` is a whole slot past ``alpha``, i.e. the next namespace.

    That is the plain case of one engine id owned by two script events. Fails if
    the compiler keeps such an id in the direct key space, where it would be
    unreachable while the event it collides with resolved unconditionally.
    """
    result = _compile(
        tmp_path,
        {
            "a.txt": (
                "add_namespace = alpha\n"
                "add_namespace = beta\n"
                "country_event = { id = alpha.100000 desc = alpha.100000.d }\n"
                "country_event = { id = beta.0 desc = beta.0.d }\n"
            )
        },
    )
    assert [row["event_id"] for row in result["events"]] == ["beta.0"]
    assert [
        (row["event_id"], row["reduced_id"]) for row in result["aliased_events"]
    ] == [("alpha.100000", 100_000)]


def test_a_decimal_suffix_that_wraps_is_aliased(tmp_path):
    """``alpha.4294967296`` reduces to 0, which is not what it reads as.

    Reading it as a number would key it on 4294967296 (rejected) or 0 (wrong
    event). Fails if the compiler keys on the parsed integer instead of the
    engine's 32-bit accumulator.
    """
    result = _compile(
        tmp_path,
        {
            "a.txt": (
                "add_namespace = alpha\n"
                "country_event = { id = alpha.4294967296 desc = a.d }\n"
            )
        },
    )
    assert result["events"] == []
    assert [
        (row["event_id"], row["reduced_id"]) for row in result["aliased_events"]
    ] == [("alpha.4294967296", 0)]


def test_namespaceless_events_are_catalogued_on_their_literal_id(tmp_path):
    """``id = 999999`` is a namespace-less event the engine accepts.

    Its id is the literal itself, and an aliased id can wrap onto it, so it has
    to be catalogued for the live join to see that collision. The engine
    rejects the declaration at 1000000, so that one is not an event id at all.
    Fails if bare ids are dropped again, or if the limit is not enforced.
    """
    result = _compile(
        tmp_path,
        {
            "a.txt": (
                "country_event = { id = 999999 desc = a.d }\n"
                "country_event = { id = 1000000 desc = b.d }\n"
            )
        },
    )
    assert result["events"] == []
    assert [
        (row["event_id"], row["numeric_id"]) for row in result["bare_events"]
    ] == [("999999", 999_999)]
    assert result["meta"]["unresolved_event_ids"] == [
        {"event_id": "1000000", "source_file": "events/a.txt"}
    ]


def test_unicode_digits_are_not_digits(tmp_path):
    """The engine reduces raw bytes, so ``٢`` is not the number two.

    Fails if the compiler uses Python's Unicode-aware ``isdecimal``/``int``,
    which would key the event on a numeric id the engine never produces.
    """
    result = _compile(
        tmp_path,
        {
            "a.txt": (
                "add_namespace = alpha\n"
                "country_event = { id = alpha.٢ desc = alpha.d }\n"
            )
        },
    )
    assert result["events"] == []
    assert [row["event_id"] for row in result["aliased_events"]] == ["alpha.٢"]


def test_namespace_folding_is_ascii_only(tmp_path):
    """Namespaces are folded ASCII-only, matching the engine and the Rust reader.

    Python's ``casefold`` would map ``STRASSE`` and ``straße`` together; the
    engine does not. Fails if the compiler folds beyond ASCII and merges two
    namespaces the engine keeps apart.
    """
    result = _compile(
        tmp_path,
        {
            "a.txt": (
                "add_namespace = straße\n"
                "add_namespace = STRASSE\n"
                "country_event = { id = straße.1 desc = a.d }\n"
                "country_event = { id = STRASSE.1 desc = b.d }\n"
            )
        },
    )
    assert [row["namespace"] for row in result["namespaces"]] == ["straße", "STRASSE"]
    assert sorted(row["event_id"] for row in result["events"]) == [
        "STRASSE.1",
        "straße.1",
    ]
    assert result["meta"]["event_key_collisions_last_wins"] == []


def _compile_with_mod(
    tmp_path,
    vanilla_files: dict[str, str],
    mod_files: dict[str, str],
    replace_path: str | None = None,
):
    """Compile through a REAL mod layer: a real mod root plus a real ``.mod``
    descriptor resolved by the shared Clausewitz resolver, not a hand-built
    layer list. The mod root lives under a user-data dir and the descriptor's
    ``path`` is relative to it, exactly as the launcher writes it."""
    vanilla = tmp_path / "vanilla"
    (vanilla / "events").mkdir(parents=True)
    for name, body in vanilla_files.items():
        (vanilla / "events" / name).write_text(body, encoding="utf-8")

    userdata = tmp_path / "userdata"
    mod_root = userdata / "mod" / "namespace_adder"
    (mod_root / "events").mkdir(parents=True)
    for name, body in mod_files.items():
        (mod_root / "events" / name).write_text(body, encoding="utf-8")

    descriptor = userdata / "mod" / "namespace_adder.mod"
    lines = [
        'name="Namespace Adder"',
        'path="mod/namespace_adder"',
        'supported_version="1.19.*"',
    ]
    if replace_path is not None:
        lines.append(f'replace_path="{replace_path}"')
    descriptor.write_text("\n".join(lines) + "\n", encoding="utf-8")

    layers = R.build_layers(
        vanilla=str(vanilla),
        mods=[str(descriptor)],
        userdata=str(userdata),
        strict_mods=True,
    )
    assert len(layers) == 2, "the mod layer must really resolve"
    return registry.compile_registry(layers, registry.PINNED_EXE_SHA256)


def test_a_mod_namespace_does_not_move_any_vanilla_event_key(tmp_path):
    """A mod that adds namespaces ahead of vanilla ones leaves every vanilla
    event key byte-identical.

    This is the mod-compatibility gate for event identity. The engine numbers a
    namespace by its position in its own load-order table, so a mod that
    declares a namespace shifts the engine slot of every namespace loaded after
    it. Keying an event on ``(namespace, numeric id)`` is what makes that shift
    invisible to the catalogue; the mod's own events must simply appear
    alongside vanilla's.

    Fails if the compiler ever reintroduces a position-derived number (an
    ordinal, a slot, a ``combined_id``) into an event key, because the added
    ``aaa_*`` namespaces sort ahead of ``vanilla_ns`` and would renumber it.
    """
    vanilla_files = {
        "z_vanilla.txt": (
            "add_namespace = vanilla_ns\n"
            "country_event = { id = vanilla_ns.7 desc = vanilla_ns.7.d }\n"
        )
    }
    baseline = _compile(tmp_path / "baseline", vanilla_files)
    modded = _compile_with_mod(
        tmp_path / "modded",
        vanilla_files,
        {
            "aaa_mod.txt": (
                "add_namespace = aaa_mod_first\n"
                "add_namespace = aaa_mod_second\n"
                "country_event = { id = aaa_mod_first.7 desc = aaa_mod_first.7.d }\n"
            )
        },
    )

    def key_of(result, event_id):
        rows = [row for row in result["events"] if row["event_id"] == event_id]
        assert len(rows) == 1, f"{event_id} not uniquely catalogued"
        return rows[0]["namespace"], rows[0]["numeric_id"]

    assert key_of(baseline, "vanilla_ns.7") == ("vanilla_ns", 7)
    assert key_of(modded, "vanilla_ns.7") == key_of(baseline, "vanilla_ns.7")
    assert key_of(modded, "aaa_mod_first.7") == ("aaa_mod_first", 7)
    # Only the SET matters. Cross-layer namespace order is file provenance the
    # catalogue deliberately does not own, so pinning a position here would
    # re-establish exactly the authority this design hands back to the engine.
    assert {row["namespace"] for row in modded["namespaces"]} == {
        "vanilla_ns",
        "aaa_mod_first",
        "aaa_mod_second",
    }
    for row in modded["events"] + modded["namespaces"]:
        assert not {"combined_id", "ordinal", "slot"} & set(row)


def test_a_mod_file_override_replaces_the_vanilla_events_it_shadows(tmp_path):
    """A mod file at a vanilla relpath replaces it, and the vanilla-only events
    it shadowed are gone from the catalogue.

    Fails if the compiler ever merges both copies of an overridden file, which
    would leave a live ``CEvent`` joinable to an event the running engine does
    not have.
    """
    modded = _compile_with_mod(
        tmp_path,
        {
            "shared.txt": (
                "add_namespace = shared_ns\n"
                "country_event = { id = shared_ns.1 desc = vanilla.1.d }\n"
                "country_event = { id = shared_ns.2 desc = vanilla.2.d }\n"
            )
        },
        {
            "shared.txt": (
                "add_namespace = shared_ns\n"
                "country_event = { id = shared_ns.1 desc = modded.1.d }\n"
            )
        },
    )
    rows = {row["event_id"]: row for row in modded["events"]}
    assert set(rows) == {"shared_ns.1"}
    assert rows["shared_ns.1"]["desc_keys"] == ["modded.1.d"]
    assert rows["shared_ns.1"]["source_file"] == "events/shared.txt"
