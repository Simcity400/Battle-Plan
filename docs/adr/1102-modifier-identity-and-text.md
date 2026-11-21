# 1102. Resolve modifier descriptions from recorded identity

Modifier rows are sparse and keyed by recorded definition index. Resolve the latest registration for a token, identified by the highest index, through the sorted lexical token table. The lexer uses case-folded FNV-1a and case-insensitive comparison. Two distinct recorded tokens that fold to the same name violate the uniqueness the reader depends on, so describe the ambiguity by name. A missing definition, dictionary, required feature mask, or requested row also produces a named refusal. An authored name with no registered definition is dropped, as the loader drops it, and its absence determines whether the corresponding conditional block was skipped.

Cache the name index on the recording table identities. Comparing strong references by pointer equality lets shared frames reuse the index and keeps a later allocation from reusing the pointer while a dependent cache still holds it. Sorting by the native index preserves description order independently of gaps in the recorded table.

Apply eligibility before formatting an entry. An unselected feature admits the raw value, and a null selector callback makes a selected feature ineligible. The clamping flag limits effective values to zero through one. For duplicate authored entries, the last value wins. Preserve separate hidden values, the added-modifier marker, and custom tooltip keys. Weather-event eligibility scans even hidden entries for nonzero effective values in its included category. Displayed trait lines use precision two without colour, apply the caller's prefix, and keep explicit display-zero flags. Custom keys are emitted by hash-table slot, skipping empty keys. Copies rebuild the key table in source slot order.

Resolve names and substitutions through localisation, honoring the definition's optional argument and keeping an absent key's own text visible. The caller can cap precision, force a sign, and invert positive and negative colour roles. Report invalid encoded definition text by field.

[Modifier description](../../crates/viewer-combat/src/modifier_text.rs) · [Modifier tests](../../crates/viewer-combat/src/modifier_text/tests.rs)
