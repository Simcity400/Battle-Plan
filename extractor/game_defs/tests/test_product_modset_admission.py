"""Contract tests for provenance-derived product-modset admission.

Every corpus built here is SYNTHETIC TEST DATA: the trees, descriptors, mods
and DLC below are fabricated in a temporary directory and describe no real
installation.  They exist to prove that an arbitrary player's modset -- one
this repository has never seen and holds no entry for -- is admitted purely
from its own live-resolution provenance, and that breaking any single link in
that provenance chain fails closed with a named reason.
"""

from __future__ import annotations

import json
import re
import sys
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path
from typing import Any


GAME_DEFS = Path(__file__).resolve().parents[1]
if str(GAME_DEFS) not in sys.path:
    sys.path.insert(0, str(GAME_DEFS))

import live_binding_resolver as LR
import prepare_product_modset_capture as PREPARE
import product_modset_admission as PMA
import static_bundle as SB


SYNTHETIC_VERSION = "Synthetic Fixture v1.19.2.0.a729 (test) build time: never"


def _write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(text.encode("utf-8"))


def _redigest(document: dict[str, Any]) -> dict[str, Any]:
    """Re-seal a deliberately mutated normalized document.

    Admission re-derives every binding document from its own rows, so a
    negative test that only edited rows would be stopped by that digest check
    before it reached the obligation it is aiming at.  Callers must leave the
    rows in the order `static_bundle._validate_binding_document` normalizes to.
    """
    document["item_count"] = len(document["items"])
    document["effective_content_sha256"] = SB._content_digest(document["items"])
    return document


_MODDED_SPECS: list[tuple[str, str]] = [
    ("mod/zz-synthetic-last.mod", "Synthetic Zed Mod"),
    ("mod/aa-synthetic-first.mod", "Synthetic Alpha Mod"),
]


class SyntheticModset:
    """One fabricated, fully cross-validated six-document binding corpus."""

    def __init__(self, root: Path, *, mod_specs: list[tuple[str, str]]) -> None:
        self.root = root
        self.vanilla = root / "vanilla"
        self.userdir = root / "userdir"
        _write(self.vanilla / "common" / "base.txt", "synthetic vanilla payload")

        _write(self.vanilla / "console_history.txt", "tag fra\ninstantconstruction\n")
        _write(self.vanilla / "imgui.ini", "[Window][Debug##Default]\nPos=60,60\n")
        _write(self.vanilla / "common" / "console_history.txt", "authored content, not a sink")

        self.integrated_dlc = [
            ("dlc903_synthetic_integrated", "Synthetic Integrated Pack"),
        ]
        for dlc_id, name in self.integrated_dlc:
            _write(
                self.vanilla / "integrated_dlc" / dlc_id / f"{dlc_id}.dlc",
                f'name="{name}"\npath="dlc/{dlc_id}"\n',
            )
            _write(
                self.vanilla / "integrated_dlc" / dlc_id / "common" / "integrated.txt",
                f"synthetic integrated payload for {dlc_id}",
            )

        self.installed_dlc = [
            ("dlc900_synthetic_alpha", "Synthetic Alpha Pack"),
            ("dlc901_synthetic_beta", "Synthetic Beta Pack"),
            ("dlc902_synthetic_gamma", "Synthetic Gamma Pack"),
        ]
        for dlc_id, name in self.installed_dlc:
            _write(
                self.vanilla / "dlc" / dlc_id / f"{dlc_id}.dlc",
                f'name="{name}"\npath="dlc/{dlc_id}"\n',
            )
            _write(
                self.vanilla / "dlc" / dlc_id / "gfx" / "payload.txt",
                f"synthetic payload for {dlc_id}",
            )
        self.active_dlc_ids = ["dlc900_synthetic_alpha", "dlc901_synthetic_beta"]
        self.launcher_disabled_dlc_ids = ["dlc901_synthetic_beta"]

        sources: list[dict[str, Any]] = []
        for token, name in sorted(mod_specs, key=lambda row: LR._engine_byte_key(row[0])):
            descriptor = self.userdir / token
            content = self.userdir / "mods" / Path(token).stem
            _write(
                descriptor,
                f'name="{name}"\npath="mods/{Path(token).stem}"\n',
            )
            _write(content / "common" / "unit.txt", f"synthetic content for {token}")
            content_sha, _rows = LR.hash_tree(content, layer_kind="mod", layer_id=token)
            sources.append(
                {
                    "id": token,
                    "ordinal": len(sources),
                    "name": name,
                    "descriptor_path": str(descriptor),
                    "descriptor_sha256": SB.sha256_file(descriptor),
                    "content_kind": "directory",
                    "content_path": str(content),
                    "content_sha256": content_sha,
                }
            )
        self.sources = sources
        self.identity = {
            "playset_id": "runtime-playset:" + "5" * 64,
            "profile_id": "runtime-profile:" + "6" * 64,
        }
        self.inventory = LR.recompute_source_inventory(
            self.vanilla, sources, self.active_dlc_ids, [], self.identity
        )
        self.documents_wire = self._build_documents()
        self.report = self._build_report()

    def _dlc_rows(self) -> list[dict[str, Any]]:
        rows = []
        for dlc_id, name in self.installed_dlc:
            content_sha, _rows = LR.hash_tree(
                self.vanilla / "dlc" / dlc_id, layer_kind="dlc", layer_id=dlc_id
            )
            rows.append(
                {
                    "id": dlc_id,
                    "name": name,
                    "content_sha256": content_sha,
                    "enabled": dlc_id in self.active_dlc_ids,
                }
            )
        return sorted(rows, key=lambda row: LR._engine_byte_key(row["id"]))

    def _build_documents(self) -> dict[str, dict[str, Any]]:
        mod_rows = sorted(
            (
                {
                    "id": source["id"],
                    "name": source["name"],
                    "descriptor_sha256": source["descriptor_sha256"],
                    "content_sha256": source["content_sha256"],
                    "enabled": True,
                }
                for source in self.sources
            ),
            key=lambda row: LR._engine_byte_key(row["id"]),
        )
        dlc_rows = self._dlc_rows()
        by_id = {row["id"]: row["content_sha256"] for row in mod_rows + dlc_rows}
        base = next(row for row in self.inventory["namespaces"] if row["binding"] == "base")
        load_rows = [
            {
                "id": "vanilla",
                "kind": "vanilla",
                "ordinal": 0,
                "content_sha256": base["content_sha256"],
            }
        ]
        for namespace in self.inventory["namespaces"]:
            if namespace["binding"] not in {"active_product", "active_mod"}:
                continue
            load_rows.append(
                {
                    "id": namespace["id"],
                    "kind": namespace["kind"],
                    "ordinal": len(load_rows),
                    "content_sha256": by_id[namespace["id"]],
                }
            )
        return {
            "mod_state": {
                "schema": SB.MOD_STATE_SCHEMA,
                "identity": self.identity,
                "mods": mod_rows,
                "effective_content_sha256": SB._content_digest(mod_rows),
            },
            "dlc_state": {
                "schema": SB.DLC_STATE_SCHEMA,
                "identity": self.identity,
                "dlc": dlc_rows,
                "effective_content_sha256": SB._content_digest(dlc_rows),
            },
            "load_order": {
                "schema": SB.LOAD_ORDER_SCHEMA,
                "identity": self.identity,
                "entries": load_rows,
                "effective_content_sha256": SB._content_digest(load_rows),
            },
        }

    def _build_report(self) -> dict[str, Any]:
        installed = [dlc_id for dlc_id, _name in self.installed_dlc]
        requested = sorted(
            set(installed) - set(self.launcher_disabled_dlc_ids),
            key=LR._engine_byte_key,
        )
        names = {dlc_id: name for dlc_id, name in self.installed_dlc}
        return {
            "schema": LR.RESOLUTION_SCHEMA,
            "passed": True,
            "process": {
                "pid": 4242,
                "created_utc": "2026-08-14T00:00:00+00:00",
                "creation_filetime_utc": 133700000000000000,
                "exe_name": "hoi4.exe",
                "exe_bytes": 55680632,
                "exe_sha256": LR.EXPECTED_EXE_SHA256,
            },
            "runtime": {
                "first_time": "00:00:00",
                "version": SYNTHETIC_VERSION,
                "active_dlc": [names[dlc_id] for dlc_id in self.active_dlc_ids],
                "active_mods": [source["name"] for source in self.sources],
            },
            "launcher_dlc_conflicts": sorted(
                set(self.active_dlc_ids) - set(requested), key=LR._engine_byte_key
            ),
            "requested_profile_dlc_ids": requested,
            "effective_launch_dlc_ids": list(self.active_dlc_ids),
            "descriptorless_asset_layers": [],
            "excluded_descriptorless_asset_layers": [],
            "engine_runtime_artifacts": {
                "schema": LR.ENGINE_RUNTIME_ARTIFACTS_SCHEMA,
                "class": LR.ENGINE_RUNTIME_ARTIFACTS_CLASS,
                "authority_sha256": LR.ENGINE_RUNTIME_ARTIFACTS_SHA256,
                "observed_in_base_layer": LR.observe_engine_runtime_artifacts(self.vanilla),
            },
            "binding_identity": self.identity,
            "source_inventory_rows": len(self.inventory["rows"]),
            "source_inventory_sha256": self.inventory["inventory_sha256"],
            "resolved_mod_sources": self.sources,
        }

    def retired_v1_base_digest(self) -> str:
        """The digest ``extractor.load_order.v1`` recorded in the vanilla row.

        A deliberate reproduction of the RETIRED derivation -- the resolver's
        second full-tree hash over `vanilla` minus `dlc/`, which included
        `integrated_dlc/` and therefore matched no namespace in the effective
        VFS.  It exists only so the negative tests below can present a corpus
        shaped exactly like every v1 recording.
        """
        digest, _rows = LR.hash_tree(
            self.vanilla, layer_kind="vanilla", layer_id="vanilla", exclude_top={"dlc"}
        )
        return digest

    def documents(self) -> dict[str, dict[str, Any]]:
        return {
            role: SB._validate_binding_document(deepcopy(document), role)
            for role, document in self.documents_wire.items()
        }

    def product(self) -> dict[str, Any]:
        return SB.product_modset_binding(
            LR.EXPECTED_EXE_SHA256, self.documents(), self.inventory
        )

    def admit(self, **overrides: Any) -> dict[str, Any]:
        payload = {
            "product_modset": self.product(),
            "bindings": self.documents(),
            "source_inventory": deepcopy(self.inventory),
            "resolution_report": deepcopy(self.report),
        }
        payload.update(overrides)
        return PMA.admit(
            payload["product_modset"],
            payload["bindings"],
            payload["source_inventory"],
            payload["resolution_report"],
        )


class ProvenanceAdmissionTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="product-modset-test-")
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)

    def _modded(self) -> SyntheticModset:
        return SyntheticModset(self.root / "modded", mod_specs=list(_MODDED_SPECS))

    def _vanilla(self) -> SyntheticModset:
        return SyntheticModset(self.root / "vanilla-only", mod_specs=[])

    def test_canonical_policy_holds_no_content_address(self) -> None:
        raw = json.loads(PMA.DEFAULT_POLICY.read_text(encoding="utf-8"))
        flat: list[str] = []

        def walk(value: Any) -> None:
            if isinstance(value, dict):
                for child in value.values():
                    walk(child)
            elif isinstance(value, list):
                for child in value:
                    walk(child)
            elif isinstance(value, str):
                flat.append(value)

        walk(raw)
        self.assertTrue(flat)
        for value in flat:
            self.assertFalse(
                len(value) == 64 and not (set(value.lower()) - set("0123456789abcdef")),
                f"policy carries a content address: {value}",
            )
        self.assertNotIn("entries", raw)

    def test_unregistered_modded_playset_is_admitted_on_its_own_provenance(self) -> None:
        fixture = self._modded()
        admitted = fixture.admit()
        self.assertEqual(admitted["schema"], PMA.ADMISSION_SCHEMA)
        self.assertEqual(admitted["classification"], "modded")
        self.assertEqual(admitted["policy_sha256"], PMA.CANONICAL_POLICY_SHA256)
        self.assertEqual(
            admitted["product_modset_sha256"],
            fixture.product()["product_modset_sha256"],
        )
        self.assertEqual(
            admitted["engine_mount_order"],
            ["mod/aa-synthetic-first.mod", "mod/zz-synthetic-last.mod"],
        )
        self.assertEqual(admitted["enabled_dlc_ids"], fixture.active_dlc_ids)
        self.assertEqual(
            admitted["launcher_profile_conflict_dlc_ids"],
            fixture.launcher_disabled_dlc_ids,
        )
        self.assertEqual(
            admitted["satisfied_provenance"], list(PMA.IMPLEMENTED_PROVENANCE)
        )

    def test_two_unrelated_playsets_admit_against_the_same_policy(self) -> None:
        modded = self._modded().admit()
        plain = self._vanilla().admit()
        self.assertEqual(plain["classification"], "vanilla")
        self.assertEqual(plain["enabled_mod_ids"], [])
        self.assertEqual(plain["engine_mount_order"], [])
        self.assertNotEqual(
            modded["product_modset_sha256"], plain["product_modset_sha256"]
        )
        self.assertEqual(modded["policy_sha256"], plain["policy_sha256"])

    def test_player_authored_mod_display_names_cannot_break_the_handoff(self) -> None:
        fixture = SyntheticModset(
            self.root / "hostile-names",
            mod_specs=[
                ("mod/aa-path-named.mod", r"C:\Program Files\Not A Path"),
                ("mod/zz-hex-named.mod", "0x140000000"),
            ],
        )
        admitted = fixture.admit()
        self.assertEqual(admitted["classification"], "modded")
        self.assertEqual(
            admitted["enabled_mod_ids"],
            ["mod/aa-path-named.mod", "mod/zz-hex-named.mod"],
        )
        PREPARE._reject_pointers(admitted)

    def test_mod_the_engine_did_not_activate_is_refused(self) -> None:
        fixture = self._modded()
        report = deepcopy(fixture.report)
        report["runtime"]["active_mods"] = report["runtime"]["active_mods"][:1]
        with self.assertRaisesRegex(SB.BundleError, "engine-reported active mod roster"):
            fixture.admit(resolution_report=report)

    def test_dlc_the_engine_did_not_activate_is_refused(self) -> None:
        fixture = self._modded()
        documents = fixture.documents()
        for row in documents["dlc_state"]["items"]:
            if row["id"] == "dlc902_synthetic_gamma":
                row["enabled"] = True
        _redigest(documents["dlc_state"])
        with self.assertRaisesRegex(SB.BundleError, "engine-reported active DLC roster"):
            fixture.admit(bindings=documents)

    def test_launcher_conflict_roster_must_be_the_derived_set(self) -> None:
        fixture = self._modded()
        report = deepcopy(fixture.report)
        report["launcher_dlc_conflicts"] = []
        with self.assertRaisesRegex(SB.BundleError, "launcher DLC conflicts"):
            fixture.admit(resolution_report=report)

    def test_mount_order_that_contradicts_the_engine_is_refused(self) -> None:
        fixture = self._modded()
        documents = fixture.documents()
        entries = documents["load_order"]["items"]
        mods = [row for row in entries if row["kind"] == "mod"]
        mods[0]["ordinal"], mods[1]["ordinal"] = mods[1]["ordinal"], mods[0]["ordinal"]
        entries.sort(key=lambda row: row["ordinal"])
        _redigest(documents["load_order"])
        with self.assertRaisesRegex(SB.BundleError, "engine mount order"):
            fixture.admit(bindings=documents)

    def test_binding_document_that_does_not_describe_its_rows_is_refused(self) -> None:
        fixture = self._modded()
        documents = fixture.documents()
        documents["mod_state"]["item_count"] = 999
        with self.assertRaisesRegex(SB.BundleError, "re-derive from its own content rows"):
            fixture.admit(bindings=documents)

        documents = fixture.documents()
        documents["dlc_state"]["effective_content_sha256"] = "a" * 64
        with self.assertRaisesRegex(SB.BundleError, "effective_content_sha256"):
            fixture.admit(bindings=documents)

    def test_source_inventory_that_does_not_describe_its_rows_is_refused(self) -> None:
        fixture = self._modded()
        inventory = deepcopy(fixture.inventory)
        inventory["rows_sha256"] = "b" * 64
        with self.assertRaisesRegex(SB.BundleError, "digest mismatch"):
            fixture.admit(source_inventory=inventory)

        inventory = deepcopy(fixture.inventory)
        inventory["namespaces"][0]["file_count"] += 1
        with self.assertRaisesRegex(SB.BundleError, "file_count mismatch"):
            fixture.admit(source_inventory=inventory)

    def test_base_load_order_layer_must_name_the_base_namespace(self) -> None:
        fixture = self._modded()
        documents = fixture.documents()
        documents["load_order"]["items"][0]["id"] = "not-the-base-namespace"
        _redigest(documents["load_order"])
        with self.assertRaisesRegex(SB.BundleError, "does not name the effective-VFS base"):
            fixture.admit(bindings=documents)

    def test_resolver_derives_the_base_row_from_the_base_namespace(self) -> None:
        """The PRODUCTION derivation of the vanilla load-order row, under test.

        ``live_binding_resolver.resolve_live_bindings`` needs a running game and
        no test can execute it, so the one decision inside it that defines a
        product-modset key input lives in its own function,
        ``base_layer_load_row``, and is exercised here.  The corpus carries an
        integrated_dlc layer, which is what makes the retired derivation
        (`vanilla` minus `dlc/`) and the base namespace (`vanilla` minus `dlc/`
        AND `integrated_dlc/`) cover different files at all.

        These live in THIS file, not in test_live_binding_resolver.py, because
        .github/workflows/rust-production.yml runs this file directly with
        `python <file>` and does not run pytest: a pytest-only test of this
        derivation would never execute in CI.

        kills: changing base_layer_load_row to hash a tree of its own, or to
        return any digest other than the base namespace's, fails the
        content_sha256 assertion below -- and only that one; the row-shape and
        retired-versus-reconstructed assertions do not depend on what the
        function returns.
        """
        fixture = self._modded()
        row = LR.base_layer_load_row(fixture.inventory)
        self.assertEqual(
            (row["id"], row["kind"], row["ordinal"]), ("vanilla", "vanilla", 0)
        )
        base = next(
            namespace for namespace in fixture.inventory["namespaces"]
            if namespace["binding"] == "base"
        )
        reconstructed = SB.sha256_bytes(
            SB.canonical_json([
                {"relpath": item["relpath"], "bytes": item["bytes"], "sha256": item["sha256"]}
                for item in fixture.inventory["rows"]
                if item["namespace_ordinal"] == base["ordinal"]
            ])
        )
        self.assertEqual(row["content_sha256"], reconstructed)
        self.assertNotEqual(fixture.retired_v1_base_digest(), reconstructed)
        SB.join_base_layer_to_inventory([row], fixture.inventory)

    def test_resolver_refuses_an_effective_vfs_without_exactly_one_base(self) -> None:
        """kills: relaxing ``len(base) != 1`` in base_layer_load_row.

        Both directions are exercised, because each is defeated by a different
        weakening: dropping to ``not base`` still refuses zero bases but accepts
        TWO and silently takes the first, and dropping the emptiness half
        accepts zero.
        """
        fixture = self._modded()

        no_base = deepcopy(fixture.inventory)
        no_base["namespaces"][0]["binding"] = "asset_only"
        with self.assertRaisesRegex(LR.ResolutionError, "exactly one base namespace"):
            LR.base_layer_load_row(no_base)

        two_bases = deepcopy(fixture.inventory)
        second = next(
            namespace for namespace in two_bases["namespaces"]
            if namespace["binding"] != "base"
        )
        second["binding"] = "base"
        with self.assertRaisesRegex(LR.ResolutionError, "exactly one base namespace"):
            LR.base_layer_load_row(two_bases)

    def test_base_load_order_row_reconstructs_offline_from_the_inventory_rows(
        self,
    ) -> None:
        """The join lane JA proved impossible against load_order.v1.

        Reconstructs the vanilla row's CONTENT digest from the source
        inventory's own base-namespace rows -- relpath/bytes/sha256 only,
        never the namespace's declared content_sha256 -- so a holder of the
        inventory alone can check the digest bound into the product-modset key.

        kills: deleting the CONTENT comparison in
        ``static_bundle.join_base_layer_to_inventory`` -- the single
        implementation both admission call sites reach -- makes the v1-shaped
        corpus in the second half admit, so the assertRaises fails.  Building
        this corpus without its integrated_dlc layer makes the retired and
        current derivations hash the same files, so the assertNotEqual fails.

        It does NOT kill a change to the resolver: this corpus is built by the
        fixture, not by ``resolve_live_bindings``, which needs a running game.
        The production derivation is covered by
        test_resolver_derives_the_base_row_from_the_base_namespace above.
        Removing only the call in ``_engine_mount_order`` is likewise NOT
        killed here: ``admit`` reaches the same join again through
        ``validate_source_inventory_bindings``, which is deliberate -- see the
        comment at that call site.
        """
        fixture = self._modded()
        inventory = fixture.inventory
        base = next(
            namespace
            for namespace in inventory["namespaces"]
            if namespace["binding"] == "base"
        )
        content_rows = [
            {"relpath": row["relpath"], "bytes": row["bytes"], "sha256": row["sha256"]}
            for row in inventory["rows"]
            if row["namespace_ordinal"] == base["ordinal"]
        ]
        reconstructed = SB.sha256_bytes(SB.canonical_json(content_rows))
        vanilla_row = fixture.documents()["load_order"]["items"][0]
        self.assertEqual(vanilla_row["kind"], "vanilla")
        self.assertEqual(vanilla_row["content_sha256"], reconstructed)

        retired = fixture.retired_v1_base_digest()
        self.assertNotEqual(retired, reconstructed)

        documents = fixture.documents()
        documents["load_order"]["items"][0]["content_sha256"] = retired
        _redigest(documents["load_order"])
        product = SB.product_modset_binding(
            LR.EXPECTED_EXE_SHA256, documents, fixture.inventory
        )
        with self.assertRaisesRegex(
            SB.BundleError, "content digest is not the effective-VFS base namespace digest"
        ):
            fixture.admit(bindings=documents, product_modset=product)

    def test_v1_load_order_document_is_refused_by_schema_not_by_mismatch(self) -> None:
        """An old-derivation recording refuses on WHAT IT IS, before any digest.

        kills: leaving ``static_bundle.LOAD_ORDER_SCHEMA`` at
        ``extractor.load_order.v1`` -- a v1 document would then be read as a
        current one and refused, if at all, only as an unexplained digest
        mismatch.
        """
        fixture = self._modded()
        wire = deepcopy(fixture.documents_wire["load_order"])
        wire["schema"] = "extractor.load_order.v1"
        wire["entries"][0]["content_sha256"] = fixture.retired_v1_base_digest()
        wire["effective_content_sha256"] = SB._content_digest(wire["entries"])
        with self.assertRaisesRegex(SB.BundleError, "schema must be"):
            SB._validate_binding_document(wire, "load_order")

    def test_asset_layer_roster_must_match_the_mounted_namespaces(self) -> None:
        fixture = self._modded()
        report = deepcopy(fixture.report)
        report["descriptorless_asset_layers"] = [{"id": "dlc999_phantom"}]
        with self.assertRaisesRegex(SB.BundleError, "asset layer the resolution did not declare"):
            fixture.admit(resolution_report=report)

    def test_resolution_that_did_not_pass_is_refused(self) -> None:
        fixture = self._modded()
        report = deepcopy(fixture.report)
        report["passed"] = False
        with self.assertRaisesRegex(SB.BundleError, "fail-closed gate"):
            fixture.admit(resolution_report=report)

    def test_foreign_engine_build_is_refused(self) -> None:
        fixture = self._modded()
        report = deepcopy(fixture.report)
        report["process"]["exe_sha256"] = "b" * 64
        with self.assertRaisesRegex(SB.BundleError, "reverse-engineered engine build"):
            fixture.admit(resolution_report=report)

    def test_inventory_that_is_not_the_reported_one_is_refused(self) -> None:
        fixture = self._modded()
        report = deepcopy(fixture.report)
        report["source_inventory_sha256"] = "c" * 64
        with self.assertRaisesRegex(SB.BundleError, "effective-VFS inventory"):
            fixture.admit(resolution_report=report)

    def test_key_that_is_not_the_key_of_these_bindings_is_refused(self) -> None:
        fixture = self._modded()
        product = fixture.product()
        product["effective_vfs_rows_sha256"] = "0" * 64
        core = {
            key: value
            for key, value in product.items()
            if key != "product_modset_sha256"
        }
        product["product_modset_sha256"] = SB.sha256_bytes(SB.canonical_json(core))
        with self.assertRaisesRegex(SB.BundleError, "not the key of the resolved bindings"):
            fixture.admit(product_modset=product)

    def test_resolved_mod_source_content_drift_is_refused(self) -> None:
        fixture = self._modded()
        report = deepcopy(fixture.report)
        report["resolved_mod_sources"][0]["content_sha256"] = "d" * 64
        with self.assertRaisesRegex(SB.BundleError, "differs from the mod state roster"):
            fixture.admit(resolution_report=report)


class MechanismPolicyTests(unittest.TestCase):
    def _policy(self) -> dict[str, Any]:
        return json.loads(PMA.DEFAULT_POLICY.read_text(encoding="utf-8"))

    def test_canonical_policy_loads_and_pins_its_own_digest(self) -> None:
        policy = PMA.load_policy()
        self.assertEqual(policy["sha256"], PMA.CANONICAL_POLICY_SHA256)
        self.assertEqual(policy["policy"], PMA.POLICY_NAME)
        self.assertEqual(policy["engine_build_id"], "a729")
        self.assertEqual(
            policy["required_provenance"], list(PMA.IMPLEMENTED_PROVENANCE)
        )

    def test_noncanonical_policy_file_is_refused_with_no_override(self) -> None:
        with tempfile.TemporaryDirectory(prefix="product-modset-policy-") as raw:
            path = Path(raw) / "policy.json"
            path.write_bytes(PMA.DEFAULT_POLICY.read_bytes() + b"\n")
            with self.assertRaisesRegex(SB.BundleError, "canonical repository digest"):
                PMA.load_policy(path)

    def test_policy_naming_an_unimplemented_schema_is_refused(self) -> None:
        value = self._policy()
        value["resolution_mechanism"]["source_inventory_schema"] = (
            "extractor.live_binding_source_inventory.v4"
        )
        with self.assertRaisesRegex(SB.BundleError, "does not name the implemented schema"):
            PMA.normalize_policy(value)

    def test_policy_naming_another_engine_build_is_refused(self) -> None:
        value = self._policy()
        value["resolution_mechanism"]["engine_build_id"] = "not-a729"
        with self.assertRaisesRegex(SB.BundleError, "implemented engine build"):
            PMA.normalize_policy(value)

    def test_policy_dropping_an_obligation_is_refused(self) -> None:
        value = self._policy()
        value["required_provenance"] = value["required_provenance"][:-1]
        with self.assertRaisesRegex(SB.BundleError, "implemented obligation list"):
            PMA.normalize_policy(value)

    def test_policy_regaining_a_content_allowlist_is_refused(self) -> None:
        value = self._policy()
        value["entries"] = [{"id": "anything", "product_modset_sha256": "e" * 64}]
        with self.assertRaisesRegex(SB.BundleError, "unknown or missing fields"):
            PMA.normalize_policy(value)


class EngineRuntimeArtifactDomainTests(unittest.TestCase):
    """The install-root class that is OUT of the product-modset key domain.

    the game opens three bare working-directory-relative filenames as sinks it
    writes: ``console_history.txt`` (appended once per accepted console
    command) and ImGui's ``imgui.ini`` / ``imgui_log.txt``.  None is reached
    through the Clausewitz layered VFS, so none is content of the base
    namespace -- but the base namespace was built by walking the install
    directory, so all three landed in it, and typing one console command moved
    every product-modset key on the machine.
    """

    def _authority(self) -> dict[str, Any]:
        return json.loads(LR.ENGINE_RUNTIME_ARTIFACTS_PATH.read_text(encoding="utf-8"))

    def _load_edited(self, value: dict[str, Any]) -> None:
        """Load an edited authority past its digest pin, to reach the rest."""
        with tempfile.TemporaryDirectory(prefix="runtime-artifacts-") as raw:
            path = Path(raw) / "engine_runtime_artifacts.json"
            path.write_bytes(json.dumps(value, indent=2).encode("utf-8") + b"\n")
            original = LR.ENGINE_RUNTIME_ARTIFACTS_SHA256
            LR.ENGINE_RUNTIME_ARTIFACTS_SHA256 = SB.sha256_file(path)
            try:
                LR.load_engine_runtime_artifacts(path)
            finally:
                LR.ENGINE_RUNTIME_ARTIFACTS_SHA256 = original

    def test_authority_is_the_exact_implemented_excluded_class(self) -> None:
        authority = LR.load_engine_runtime_artifacts()
        self.assertEqual(
            tuple(row["relpath"] for row in authority["artifacts"]),
            PMA.IMPLEMENTED_RUNTIME_ARTIFACTS,
        )
        self.assertEqual(
            authority["sha256"], SB.sha256_file(LR.ENGINE_RUNTIME_ARTIFACTS_PATH)
        )
        self.assertEqual(
            LR.engine_runtime_artifact_relpaths(),
            frozenset(PMA.IMPLEMENTED_RUNTIME_ARTIFACTS),
        )

    def test_every_census_matches_the_retained_string_xref_sweep(self) -> None:
        """The census is only a completeness proof if something checks it.

        `load_engine_runtime_artifacts` can only verify that a row's declared
        census accounts for the addresses that row NAMES -- internal
        consistency.  A row could name two references, declare a census of two,
        and leave three real ones unexamined, and the loader would pass it.
        This closes that by re-deriving the census from the RAW GHIDRA SWEEP
        retained beside this lane, which is the artifact the numbers actually
        came from, and requiring exact address-set equality.

        If the retained evidence is ever deleted, this fails rather than skips:
        the authority's central claim -- "every reference to this literal in the
        image is named here, and none of them is a content read" -- stops being
        provable at that moment, and a silent skip would hide that.
        """
        evidence_dir = GAME_DEFS.parent / "native/runs/jq_console_key"
        sweeps = sorted(evidence_dir.glob("re_string_xrefs*.txt"))
        self.assertTrue(sweeps, f"the retained string-xref sweep is missing from {evidence_dir}")
        swept: dict[str, set[str]] = {}
        literal: str | None = None
        for path in sweeps:
            for line in path.read_text(encoding="utf-8").splitlines():
                header = re.match(r'^== "(.+)" ==$', line.strip())
                if header:
                    literal = header.group(1)
                    swept.setdefault(literal, set())
                    continue
                if literal is None:
                    continue
                found = re.match(r"^\s*(?:occurrence @|ref from )([0-9a-f]+)\b", line)
                if found:
                    swept[literal].add("0x" + found.group(1))

        authority = json.loads(LR.ENGINE_RUNTIME_ARTIFACTS_PATH.read_text(encoding="utf-8"))
        for row in authority["artifacts"]:
            relpath = row["relpath"]
            evidence = row["evidence"]
            self.assertIn(relpath, swept, f"no retained sweep covers {relpath}")
            declared = set(evidence["reference_addresses"])
            if evidence["string_pointer_address"]:
                declared.add(evidence["string_pointer_address"])
            declared.add(evidence["string_address"])
            self.assertEqual(
                declared,
                swept[relpath],
                f"{relpath}: the authority and the raw sweep name different addresses",
            )
            self.assertEqual(
                evidence["reference_census"],
                len(swept[relpath]) - 1,
                f"{relpath}: declared census is not the swept reference count",
            )

    def test_edited_authority_is_refused_by_its_digest_pin(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-artifacts-") as raw:
            path = Path(raw) / "engine_runtime_artifacts.json"
            path.write_bytes(LR.ENGINE_RUNTIME_ARTIFACTS_PATH.read_bytes() + b"\n")
            with self.assertRaisesRegex(
                LR.ResolutionError, "canonical repository digest"
            ):
                LR.load_engine_runtime_artifacts(path)

    def test_a_nested_path_cannot_join_the_class(self) -> None:
        value = self._authority()
        value["artifacts"][0]["relpath"] = "common/console_history.txt"
        with self.assertRaisesRegex(
            LR.ResolutionError, "not a bare working-directory name"
        ):
            self._load_edited(value)

    def test_an_unevidenced_path_cannot_join_the_class(self) -> None:
        value = self._authority()
        del value["artifacts"][0]["evidence"]["writer_function"]
        with self.assertRaisesRegex(LR.ResolutionError, "evidence is malformed"):
            self._load_edited(value)
        value = self._authority()
        value["artifacts"][0]["evidence"]["string_address"] = "the console file"
        with self.assertRaisesRegex(LR.ResolutionError, "hex address"):
            self._load_edited(value)

    def test_authority_for_another_engine_build_is_refused(self) -> None:
        value = self._authority()
        value["engine_build_id"] = "a730"
        with self.assertRaisesRegex(LR.ResolutionError, "sinks are not the RE'd ones"):
            self._load_edited(value)
        value = self._authority()
        value["executable_sha256"] = "f" * 64
        with self.assertRaisesRegex(LR.ResolutionError, "not evidenced against"):
            self._load_edited(value)

    def test_using_the_console_does_not_move_the_product_modset_key(self) -> None:
        """The whole point, end to end over the synthetic corpus."""
        with tempfile.TemporaryDirectory(prefix="console-key-") as raw:
            fixture = SyntheticModset(Path(raw), mod_specs=_MODDED_SPECS)
            before = fixture.product()["product_modset_sha256"]
            self.assertEqual(fixture.admit()["product_modset_sha256"], before)

            history = fixture.vanilla / "console_history.txt"
            history.write_text(
                history.read_text(encoding="utf-8") + "annex fra\n", encoding="utf-8"
            )
            (fixture.vanilla / "imgui_log.txt").write_text("debug ui\n", encoding="utf-8")
            rebuilt = SyntheticModset.__new__(SyntheticModset)
            rebuilt.__dict__.update(fixture.__dict__)
            rebuilt.inventory = LR.recompute_source_inventory(
                fixture.vanilla,
                fixture.sources,
                fixture.active_dlc_ids,
                [],
                fixture.identity,
            )
            rebuilt.documents_wire = rebuilt._build_documents()
            rebuilt.report = rebuilt._build_report()
            self.assertEqual(rebuilt.product()["product_modset_sha256"], before)
            self.assertEqual(rebuilt.admit()["product_modset_sha256"], before)
            self.assertEqual(
                rebuilt.admit()["engine_runtime_artifacts"]["observed_in_base_layer"],
                ["console_history.txt", "imgui.ini", "imgui_log.txt"],
            )

    def test_real_content_of_the_same_name_still_moves_the_key(self) -> None:
        with tempfile.TemporaryDirectory(prefix="console-key-content-") as raw:
            fixture = SyntheticModset(Path(raw), mod_specs=_MODDED_SPECS)
            before = fixture.product()["product_modset_sha256"]
            nested = fixture.vanilla / "common" / "console_history.txt"
            nested.write_text("authored content, EDITED", encoding="utf-8")
            after = LR.recompute_source_inventory(
                fixture.vanilla,
                fixture.sources,
                fixture.active_dlc_ids,
                [],
                fixture.identity,
            )
            self.assertNotEqual(after["rows_sha256"], fixture.inventory["rows_sha256"])
            fixture.inventory = after
            fixture.documents_wire = fixture._build_documents()
            self.assertNotEqual(fixture.product()["product_modset_sha256"], before)

    def test_a_mod_layer_keeps_a_file_of_the_same_name(self) -> None:
        with tempfile.TemporaryDirectory(prefix="console-key-mod-") as raw:
            fixture = SyntheticModset(Path(raw), mod_specs=_MODDED_SPECS)
            before = fixture.product()["product_modset_sha256"]
            content = Path(fixture.sources[0]["content_path"])
            (content / "console_history.txt").write_text("mod payload", encoding="utf-8")
            content_sha, _rows = LR.hash_tree(
                content, layer_kind="mod", layer_id=fixture.sources[0]["id"]
            )
            fixture.sources[0]["content_sha256"] = content_sha
            fixture.inventory = LR.recompute_source_inventory(
                fixture.vanilla,
                fixture.sources,
                fixture.active_dlc_ids,
                [],
                fixture.identity,
            )
            self.assertIn(
                "console_history.txt",
                {
                    row["relpath"]
                    for row in fixture.inventory["rows"]
                    if row["layer_kind"] == "mod"
                },
            )
            fixture.documents_wire = fixture._build_documents()
            self.assertNotEqual(fixture.product()["product_modset_sha256"], before)

    def test_a_differently_cased_spelling_on_disk_is_still_excluded(self) -> None:
        with tempfile.TemporaryDirectory(prefix="console-key-case-") as raw:
            root = Path(raw) / "vanilla"
            _write(root / "common" / "base.txt", "payload")
            _write(root / "Console_History.TXT", "tag fra\n")
            excluded, _rows = LR.hash_tree(
                root,
                layer_kind="vanilla",
                layer_id="vanilla",
                exclude_relpaths=LR.engine_runtime_artifact_relpaths(),
            )
            (root / "Console_History.TXT").unlink()
            absent, _rows = LR.hash_tree(root, layer_kind="vanilla", layer_id="vanilla")
            self.assertEqual(excluded, absent)

    def test_a_differently_cased_base_row_is_still_refused_by_admission(self) -> None:
        with tempfile.TemporaryDirectory(prefix="console-key-case-admit-") as raw:
            fixture = SyntheticModset(Path(raw), mod_specs=_MODDED_SPECS)
            stale = deepcopy(fixture.inventory)
            base = next(
                namespace
                for namespace in stale["namespaces"]
                if namespace["binding"] == "base"
            )
            stale["rows"].append(
                {
                    "namespace_ordinal": base["ordinal"],
                    "layer_kind": "vanilla",
                    "layer_id": "vanilla",
                    "relpath": "Console_History.TXT",
                    "bytes": 8,
                    "sha256": "a" * 64,
                    "storage_kind": "directory",
                    "source_container_sha256": None,
                    "source_byte_offset": 0,
                    "source_byte_count": 8,
                    "source_range_sha256": "a" * 64,
                }
            )
            with self.assertRaisesRegex(
                SB.BundleError, "still carries the engine runtime artifact"
            ):
                PMA._engine_runtime_artifacts_excluded(stale, fixture.report)

    def test_the_production_capture_driver_pins_the_current_admission_schema(self) -> None:
        driver = (
            GAME_DEFS.parent / "native/scripts/Start-Hoi4ProductionCaptureSafe.ps1"
        ).resolve()
        text = driver.read_text(encoding="utf-8-sig")
        pinned = set(
            re.findall(r"extractor\.product_modset_policy_admission\.v\d+", text)
        )
        self.assertTrue(pinned, f"{driver} names no admission schema at all")
        self.assertEqual(pinned, {PMA.ADMISSION_SCHEMA})

    def test_the_base_namespace_never_lists_a_sink(self) -> None:
        with tempfile.TemporaryDirectory(prefix="console-key-rows-") as raw:
            fixture = SyntheticModset(Path(raw), mod_specs=_MODDED_SPECS)
            base = {
                namespace["ordinal"]
                for namespace in fixture.inventory["namespaces"]
                if namespace["binding"] == "base"
            }
            base_paths = {
                row["relpath"]
                for row in fixture.inventory["rows"]
                if row["namespace_ordinal"] in base
            }
            self.assertFalse(base_paths & set(PMA.IMPLEMENTED_RUNTIME_ARTIFACTS))
            self.assertIn("common/console_history.txt", base_paths)

    def test_an_inventory_built_without_the_exclusion_is_refused(self) -> None:
        with tempfile.TemporaryDirectory(prefix="console-key-refuse-") as raw:
            fixture = SyntheticModset(Path(raw), mod_specs=_MODDED_SPECS)
            stale = LR.recompute_source_inventory(
                fixture.vanilla,
                fixture.sources,
                fixture.active_dlc_ids,
                [],
                fixture.identity,
            )
            base = next(
                namespace
                for namespace in stale["namespaces"]
                if namespace["binding"] == "base"
            )
            stale["rows"].append(
                {
                    "namespace_ordinal": base["ordinal"],
                    "layer_kind": "vanilla",
                    "layer_id": "vanilla",
                    "relpath": "console_history.txt",
                    "bytes": 8,
                    "sha256": "a" * 64,
                    "storage_kind": "directory",
                    "source_container_sha256": None,
                    "source_byte_offset": 0,
                    "source_byte_count": 8,
                    "source_range_sha256": "a" * 64,
                }
            )
            with self.assertRaisesRegex(
                SB.BundleError, "still carries the engine runtime artifact"
            ):
                PMA._engine_runtime_artifacts_excluded(stale, fixture.report)

    def test_a_report_from_another_excluded_class_is_refused(self) -> None:
        with tempfile.TemporaryDirectory(prefix="console-key-report-") as raw:
            fixture = SyntheticModset(Path(raw), mod_specs=_MODDED_SPECS)
            report = deepcopy(fixture.report)
            report["engine_runtime_artifacts"]["authority_sha256"] = "b" * 64
            with self.assertRaisesRegex(
                SB.BundleError, "different engine runtime artifact authority"
            ):
                PMA._engine_runtime_artifacts_excluded(fixture.inventory, report)

            report = deepcopy(fixture.report)
            del report["engine_runtime_artifacts"]
            with self.assertRaisesRegex(
                SB.BundleError, "does not state its engine runtime artifact domain"
            ):
                PMA._engine_runtime_artifacts_excluded(fixture.inventory, report)

            report = deepcopy(fixture.report)
            report["engine_runtime_artifacts"]["observed_in_base_layer"] = [
                {"relpath": "settings.txt", "sink": "console_command_history"}
            ]
            with self.assertRaisesRegex(SB.BundleError, "outside the authority"):
                PMA._engine_runtime_artifacts_excluded(fixture.inventory, report)


if __name__ == "__main__":
    unittest.main()
