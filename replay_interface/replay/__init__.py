"""Replay data backbone: canonical snapshot schema, store/writer, diff engine.

Import-clean: pure stdlib, no game/network. See replay/SCHEMA.md for the contract.
"""
from . import schema  # noqa: F401

__all__ = ["schema"]
