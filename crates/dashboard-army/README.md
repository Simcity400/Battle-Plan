# dashboard-army

The army widgets' products: one country or one division at a reached hour,
read through the source's shared frame cache and kept in its observation cache
(`dashboard_land_battles::products::reached::cached`), the pattern of the Army
breakdown's template roster and the Navy size drill. They prepare nothing in
the background; each request reads the hour asked and the frames it shares
with every other query of the source.

| Product | Module | What it answers |
|---|---|---|
| `division-templates` | [division_templates.rs](src/products/division_templates.rs) | A country's templates in roster order, used or not: battalions and support companies, base combat width, settings and fielded divisions under the Army size attribution. |
| `training-queue` | [training_queue.rs](src/products/training_queue.rs) | A country's deployment orders in queue order, from the daily read at or before the hour, with the divisions in training under each. |
| `division-careers` | [division_careers.rs](src/products/division_careers.rs) | Every division's medal total, awards, experience and kill counter, for the chosen countries. |
| `division-record` | [division_careers.rs](src/products/division_careers.rs) | One division's header facts and recorded career and awards. |
| `division-equipment` | [division_equipment.rs](src/products/division_equipment.rs) | What a country's divisions hold, need and miss by equipment type and by template; one template's divisions; one division's pool. |

Shared naming and artwork live in [army_content.rs](src/products/army_content.rs);
template equipment needs in [division_needs.rs](src/products/division_needs.rs), which
reads dashboard-text's canonical sub-unit requirements (the map's model choice
reads the same) with recorded country-local clones at the template's hour.

## Rules

- Roster ownership selects templates; deployed divisions only count them.
  Fielded counts follow Army size's Owner/Host attribution and leave out
  intelligence copies of divisions. An unread division census leaves counts
  unknown, never zero.
- The base combat width is the recorded sum of sub-unit widths before country
  and commander factors; the designer's final width is not derived here.
- Template priority names come from the designer's own headings for the
  stages the content's defines allow. A deployment order's priority is the
  localised tooltip of its button in the effective interface files, so a mod
  that rebinds the button's text is followed; a scoped expression in it is
  evaluated for the queue's country with the operands its read witnessed, or
  refused and counted, never shown as written; its location is the owning
  state's display name of its province, or the deployment view's own text for
  none. A value outside the proven stages is refused by value, never shown as
  a number.
- The training queue is the latest daily read within a day of the hour, dated
  with that read, for the country holding the tag at the hour: the read is
  used only while its slot holds that same country identity, so a renamed
  country keeps its queue and a reused tag never lends another's. Training,
  manpower and equipment keep their stored signed values; a held design whose
  type is unresolved leaves that division's fill unknown while its known needs
  and holdings stay; an amount ordered never becomes invented divisions; a stored
  halting reason is reported as present without a name; no ending date or
  readiness is derived.
- Medal totals are the history summary's recorded amount, not a sum of award
  entries; a refused or missing history is unknown, never zero medals. The
  kill counter is the game's career counter; battles won or fought are not
  recorded per division and are not derived. Entry descriptions use the
  game's description keys through the shared
  [division history](../dashboard-text/src/division_history.rs) rules.
- Division equipment compares each division's witnessed pool with its current
  template's need, type by type, before summing: a surplus in one division
  never offsets another's shortfall. Daily pools are dated by their read; the
  hourly pools of divisions minted since then join them. A division whose pool,
  need or design classification is unknown is counted, never compared as
  empty; what its read pool holds still counts in the type's holdings, and
  unclassified holdings keep their own bucket. A type's fill is each known
  division's holding capped at its own need, summed, never the aggregate
  holding. National stock is dated context, read only while its slot holds
  the same country, and is never netted against what is missing.
- Names, icons, medals, levels and places resolve through the recorded content
  stack; each refusal is named and counted in the reply's warnings, and keeps
  the row with its known values.

`cargo test -p dashboard-army` runs these products' synthetic tests.
