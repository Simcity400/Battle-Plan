# Preserve unit save presence and ordering

## Decision

Treat each unit save as a projection of its owning writer. A full reference keeps both words, independently of enclosing roster country and logical owner. The unit and supply-consumer writers can emit the same reference twice. Accept equal repeats and reject conflicting ones. Preserve typed handles and ordering across rosters. A task force's own combat vector is not serialized, so naval combat membership comes from its side arrays. The railway gun's serialized combat references are its own ordered array and do not expose its separate unit-combat vector.

Retain the stored-length and writer-mode rules for text. Names generally appear for nonzero stored length outside checksum mode and end at the first NUL. A written empty prefix differs from an omitted stored-empty name. Preserve optional ship-name fields independently of empty, false and zero values. For a template composition keep the existing byte projection, plus independent presence of the name and localisation key. An unassigned division name group is omitted. Grid entries retain their authored kind, coordinates and literal unit names. Proven required restriction and equipment keys keep missing evidence rather than using a plausible default.

| Unit domain | Writer evidence |
|---|---|
| Movement | A nonnull target writes `support_attack`. Ordinary `path` and alternate `full_path` are written when their arrays are nonempty. Zero movement progress is omitted. The command list has one ordered entry of its type per queued action and absence is known empty. |
| Flags and references | Retreat, withdrawal, exile and separate-counter flags certify boolean truth only. A location, standard template or alternate template is written for a nonnull pointer. The exile country needs a signed-positive reference. `can_execute` is a signed byte spelling that preserves all stored bits. |
| Strength and supply | Strength is current absolute hit points in fixed5 units. Division XP is independent of exercise state and country XP. Fuel requested and stored fuel and days out of supply are omitted at zero. Maximum supply and ratio are unconditional. A missing disrupted supply amount proves nonpositive. |
| Priority and names | Priority one is omitted. The writer has separate keys for the documented priority values, and unknown words also take the front-order branch. A front-order key alone therefore cannot certify the exact stored word. Division names keep unconditional type, optional nonzero order, clear ordered flag, custom name presence and the set-only programmatic flag. |
| Viewer intel and acclimatization | Intel is a flat triple per viewer, containing signed viewer key, low 32 bits of decay accumulator and sign-extended level. Keep ordering and nonpositive levels. An empty intel array is omitted. Keep nonzero acclimatization weights in category-token order, followed by the progress fields. The maximum is omitted at 100. |
| Fleet, wings and participants | Ordered ship entries keep their positions. A critical-elements wrapper can be present when unnamed entries were omitted. The embedded pool wrapper is required. Ordinary unconditional wing bytes or text keep missing evidence separately. Wing mission flags preserve their positive and absent rules, and region changes carry signed fixed5 penalty. Relocation-only operands provide neither total distance nor destination registration order. Wing positions count across pools in file order within each country. Participant arrays retain direct side units independently of the front subset. Serialized naval transfers retain all slots, including zeros or stale handles. |

Keep the division's ordered career and medal lists from their actual serializer. Read no additional template, hint or membership from an omitted occurrence.

[Unit save reader and tests](../../crates/installed-content-save/src/units.rs)
