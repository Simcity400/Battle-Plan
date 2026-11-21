# Index overlapping recording intervals by their boundaries

Build the interval index once per reply identity. Store each interval on the segment tree nodes that cover it exactly, over half-open slabs split by all endpoints. A read binary-searches its slab and walks a single root-to-slab path. This produces all overlapping items without scanning the inventory on each hover or frame. [Interval index](../../ui/src/charts/intervalIndex.ts)
