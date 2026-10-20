# Equipment designs

## What it shows

One country's saved designs at the date shown: the current ones, then the obsolete ones in a disclosure closed by default ("Obsolete designs (N)"). Each row has the design's art and in-game name with its equipment type, when it was first recorded and, for an obsolete one, when its obsolete flag was first recorded. No new/obsolete tags and no calendar. The Included types setting (the equipment types the designs use, automatic All types) filters both lists. A row opens Production lines' design stat sheet in the drill; the menu jumps to the dates.

## Products it reads

[`production-catalogue`](../../../../crates/dashboard-economy/src/products/production_catalogue.rs) in its `designs` view (family `production_catalogue`): each saved design's creator and obsolete flag from every complete read of the variant table and the writer-observed edits between reads. `equipment-design-detail` for the stat sheet.

## Notable behaviour

- A design is the country's when the country created it (its origin is kept apart). The recording names no creation, so First seen is the first read showing the design: its date when the designs were read complete the hour before without it, else "By <date>" (already saved at the first read, or first read after hours the designs were not recorded), with the reason in the tooltip.
- Seen obsolete is the first read showing the obsolete flag (an edit observed between daily reads, or the daily read); "By <date>" when the flag was already set at the designs' first read or no current state was recorded just before it. The recording names no successor design, so nothing is called superseded.
- A design made obsolete later is current before that date. Names that cannot be resolved read "Equipment name unavailable" (counted).
