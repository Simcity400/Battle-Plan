# Preserve scripted localisation resolution and uncertainty

## Decision

Read direct `.txt` files in `common/scripted_localisation` in resolved content order. Each file constructs a lexer in mode zero. The top-level loop reads every entry without requiring `defined_text`. The last name assignment takes the string's native value and a missing name registers the empty name. The parser's native token-consumption boundaries apply to malformed input too. A definition at end of file leaves the loop with an empty entry and a scalar name reads a single token even when that token opens a block.

Retain ordered text alternatives and the first localization key or random list per pair. A later key or list is refused and skipped. The evaluator skips absent text values and uses the first alternative whose conjunction passes. An authored empty trigger still starts a scoped-variable context. A trigger not authored starts none. Trigger bodies use the same field dispatch as other triggers. A modifier inside a random list uses the shared mean-time modifier reader.

A random list preserves authored member order and the ordered seed assignments because reassigning only some operands retains their prior fields. A number key supplies a signed integer weight scaled by 100000. A missing member key is empty. Unsupported bound-localisation blocks inside a member make the current entry and the remainder of that file uncertain.

Lookup of the final bracket segment after built-in properties uses the authored name's exact bytes. Later definitions replace earlier ones. An unread suffix can define an otherwise absent name or redefine an existing one. A later readable definition supersedes uncertainty from earlier files. The evaluator can report an undefined name as empty only when the remaining files cannot supply it. The inventory reports where each unsettled read began and why.

[Scripted localisation reader and tests](../../crates/installed-content-scripted-localisation/src/scripted_localisation.rs)
