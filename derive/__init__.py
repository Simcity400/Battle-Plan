"""derive -- offline computation of the DERIVED ("D"-source) replay layers.

Pure offline Python. Reads a stored snapshot (replay/SCHEMA.md shape) + static
map assets, and computes the schema's D-tagged fields that ship null until a
compute pass exists:

  * ``map.front_lines[]``                control-adjacency edges
  * ``map.controlled_industry_share``   factory share by controller
  * division ``encircled``              same-controller supply-graph estimate
  * ``military.ships_by_hull``          OOB hull census (tonnage stays null)
  * ``military.casualties``             per-tag war losses (total; killed null)
  * ``military.manpower`` proxy         deployed_strength_proxy (people stays null)
  * ``economy.gross_ic_by_equipment``   two-snapshot production delta (else null)

No fabrication: a field stays null when its inputs are absent. See ``layers.py``
for the per-field model and ``overlay.py`` for how they land on a snapshot.
"""

from __future__ import annotations

from .overlay import derive_layers
from . import layers, adjacency

__all__ = ["derive_layers", "layers", "adjacency"]
