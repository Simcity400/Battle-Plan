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

The division designer's stats are not a product of their own: the game-window designer
reads them through [designer_stats.rs](src/products/designer_stats.rs).

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

## Designer stats

[designer_stats](src/products/designer_stats.rs) computes what the game's division designer shows
for a recorded template.

- **Calculator.** [kernel.rs](src/products/designer_stats/kernel.rs) is the game's division stat
  calculator, the one it runs for a deployed division before its modifier pass and for the
  designer's preview with nothing after it. It works group by group in composition order:
  equipment terms are the held or chosen designs' effective stats, times one plus the
  definition's coefficient, times the share of the requirement they fill. Bases are the
  definition's words for the stats that have one. Support companies' category grants adjust
  both. Each stat is then finished as the game finishes it: a sum, a mean over every real cell
  (over non-support cells for hardness), a blend of the largest group with the mean (armour and
  piercing), or a total times its factor (suppression, supply, fuel). Speed is selected group
  by group: each group whose definition affects speed offers its equipment speed per sub-unit,
  the slowest offer wins, and the loaded slowest-speed define floors it after every group.
  Fixed5 arithmetic truncates at every stage the game takes.
- **Definitions.** [definitions.rs](src/products/designer_stats/definitions.rs) builds the
  effective definition. A stored global definition keeps its authored words. A country-local
  one adds each researched technology's unit bonus times its researched level, then the country's
  persistent unit bonuses with a multiplier of one, in stored order. Every sub-unit, category,
  stat key and grant resolves through the recorded content stack; an authored bonus form the
  calculator does not reproduce refuses the definition by name.
- **Preview.** For each required equipment type the preview holds the whole requirement of
  one design: the first design of the roster owner's production roster, by the game's design
  precedence, that is module-valid and not excluded by the template. It uses the template's
  own manpower and full fuel. A type with no such design has no pool entry. A group's
  critical-equipment fill weighs each critical type's holding and requirement by the type's own
  build cost, so a short critical type lowers that group's equipment terms as the game's
  does. A preview whose pool does not read refuses the equipment-dependent stats by name while
  manpower, training time and the requirement rows stay shown.
- **Rows.** [rows.rs](src/products/designer_stats/rows.rs) lists the designer's three fixed grids
  in the game's order. The roster is fixed, so a stat a mod adds gets no row, as in the game.
  Each value is formatted by its stat's own `_VALUE` localisation, or trimmed for width,
  manpower and training time. A row shows its value only when its stat is in `PROVEN`, meaning
  it matched the game's recorded division caches. Every other row keeps its label and names
  why its value is unavailable. After the equipment grid's fixed rows come its requirement
  rows: each required equipment type in the equipment database's order, named by the
  designer's country's own short name where it has one, its quantity rounded up to a whole
  number, a zero quantity omitted.
- **Proof.** [validation.rs](src/products/designer_stats/validation.rs) is an ignored test that runs
  the calculator for every deployed division of a recording and compares it with the recorded
  panel cache. A stat the deployed modifier pass rewrites is compared after rebuilding that
  pass ([post.rs](src/products/designer_stats/validation/post.rs)) from the division's recorded
  country and commander modifier roots, for a medal-free division; speed is also compared with
  the moving divisions' recorded stat-block speed. Templates' width, manpower and training time
  are compared with their own caches. A second ignored test lists named countries' templates
  with every designer row, for checking against the game's own designer. Both read the
  recording and installation named by the environment (see the file) and write nothing beside
  the recording.

`cargo test -p dashboard-army` runs these products' synthetic tests.
