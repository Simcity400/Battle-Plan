# Division equipment

## What it shows

What a country's divisions hold, need and are missing. **By equipment**: each equipment type in the divisions, with In divisions, Needed, Missing, Fill and Divisions short; a type opens its models. **By template**: each template with divisions, manpower, strength, equipment fill and types short; a template opens its divisions, and a division its own pool.

## Products it reads

[`division-equipment`](../../../../crates/dashboard-army/src/products/division_equipment.rs) at the shown hour; the drills ask the same product for one template's divisions or one division's pool.

## Settings

One country; View (also the header switch, and a catalogue card each); Expeditionary forces count for; the shared table options; drill placement; Campaign, Follow (default) or Pin.

## Notable behaviour

- Missing adds each division's own shortfall: a surplus in one division never covers another's. Fill is the same measure: each division's holding capped at its own need, summed over the need. In divisions stays the actual holding, including divisions whose need is unknown. Shortfalls use the deficit look; the hatch means unknown only.
- The country is the one identity the choice named at the date it was made, kept with the widget and followed through that identity's renames only, so a later holder of the tag never takes it; a tag that named no country then and several over the campaign asks which one.
- Pools are daily samples dated "Equipment as of"; divisions whose pool or need is unknown are counted, never treated as empty.
- National stock appears only in tooltips, dated, and is never subtracted from what is missing.
- Need belongs to the equipment type; models show what is actually held.

## Files

| File | Responsibility |
|---|---|
| [DivisionEquipmentWidget.tsx](DivisionEquipmentWidget.tsx) | Both views, header switch and the two-level drill. |
| [divisionEquipmentData.ts](divisionEquipmentData.ts) | Validation, fills and tooltips. |
