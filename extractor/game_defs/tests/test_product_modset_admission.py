from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path


GAME_DEFS = Path(__file__).resolve().parents[1]
if str(GAME_DEFS) not in sys.path:
    sys.path.insert(0, str(GAME_DEFS))

import product_modset_admission as PMA  # noqa: E402
import static_bundle as SB  # noqa: E402


def _product() -> dict:
    return deepcopy(PMA.load_policy()["entries"][0]["product_modset"])


def _bindings(*mod_ids: str) -> dict:
    mods = [
        {
            "id": item,
            "enabled": True,
        }
        for item in mod_ids
    ]
    return {
        "mod_state": {"items": mods},
        "load_order": {
            "items": [
                {"id": "vanilla", "kind": "vanilla"},
                *({"id": item, "kind": "mod"} for item in mod_ids),
            ]
        },
    }


def _write_policy(path: Path, entries: list[dict]) -> Path:
    path.write_text(
        json.dumps(
            {
                "schema": PMA.POLICY_SCHEMA,
                "policy": "exact_registry_match",
                "entries": entries,
            },
            sort_keys=True,
        ),
        encoding="utf-8",
    )
    return path


def _rehash(product: dict) -> dict:
    core = {key: value for key, value in product.items() if key != "product_modset_sha256"}
    product["product_modset_sha256"] = hashlib.sha256(SB.canonical_json(core)).hexdigest()
    return product


class ProductModsetAdmissionTests(unittest.TestCase):
    def test_registry_admits_only_the_exact_approved_vanilla_identity(self) -> None:
        product = _product()
        admitted = PMA.admit(product, _bindings())
        self.assertEqual(admitted["classification"], "vanilla")
        self.assertEqual(admitted["product_modset_sha256"], product["product_modset_sha256"])
        self.assertEqual(admitted["enabled_mod_ids"], [])
        self.assertEqual(admitted["policy_sha256"], PMA.CANONICAL_POLICY_SHA256)

    def test_well_formed_unknown_content_address_fails_closed(self) -> None:
        product = _product()
        product["effective_vfs_rows_sha256"] = "0" * 64
        _rehash(product)
        with self.assertRaisesRegex(SB.BundleError, "not an explicitly supported"):
            PMA.admit(product, _bindings())

    def test_vanilla_entry_cannot_be_reused_for_an_enabled_mod(self) -> None:
        with self.assertRaisesRegex(SB.BundleError, "vanilla product policy"):
            PMA.admit(_product(), _bindings("mod/example.mod"))

    def test_exact_validated_modset_entry_is_admitted(self) -> None:
        with tempfile.TemporaryDirectory(prefix="product-modset-test-") as raw:
            root = Path(raw)
            product = _product()
            product["mod_state_effective_content_sha256"] = "a" * 64
            product["load_order_effective_content_sha256"] = "b" * 64
            product["effective_vfs_namespace_sha256"] = "c" * 64
            product["effective_vfs_rows_sha256"] = "d" * 64
            _rehash(product)
            policy = _write_policy(
                root / "supported.json",
                [{
                    "id": "validated-example",
                    "classification": "validated_modset",
                    "product_modset": product,
                }],
            )
            with self.assertRaisesRegex(SB.BundleError, "canonical repository registry"):
                PMA.admit(product, _bindings("mod/example.mod"), policy)
            admitted = PMA.admit(
                product, _bindings("mod/example.mod"), policy,
                allow_dev_policy_override=True,
            )
            self.assertEqual(admitted["entry_id"], "validated-example")
            self.assertEqual(admitted["enabled_mod_ids"], ["mod/example.mod"])

    def test_registry_rejects_duplicate_keys_and_duplicate_json_members(self) -> None:
        with tempfile.TemporaryDirectory(prefix="product-modset-test-") as raw:
            root = Path(raw)
            product = _product()
            duplicate_key = _write_policy(
                root / "duplicate-key.json",
                [
                    {"id": "one", "classification": "vanilla", "product_modset": product},
                    {"id": "two", "classification": "vanilla", "product_modset": product},
                ],
            )
            with self.assertRaisesRegex(SB.BundleError, "duplicate id or key"):
                PMA.load_policy(duplicate_key, allow_dev_policy_override=True)

            duplicate_member = root / "duplicate-member.json"
            duplicate_member.write_text(
                '{"schema":"extractor.supported_product_modsets.v1",'
                '"policy":"exact_registry_match","policy":"exact_registry_match","entries":[]}',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(SB.BundleError, "duplicate JSON object member"):
                PMA.load_policy(duplicate_member, allow_dev_policy_override=True)


if __name__ == "__main__":
    unittest.main()
