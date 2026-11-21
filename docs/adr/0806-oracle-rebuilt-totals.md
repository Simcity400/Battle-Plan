# Derive saved stock totals only when history gives a unique answer

A variant counts as a convoy according to its normalized type or allowed archetype flags. A content module may change a modular type's flags. An essential raid or plane stock total comes from variant amounts in whole units and is capped by the largest requirement. A load may compare a value rebuilt from the saved variants, provided every contributing variant has the required definition and every admissible cap order gives one total. Repeated variants replace earlier amounts. Same-sign inputs can settle at the cap in any order, while mixed inputs can make capped results depend on order. Withhold a nonunique total. Away from a load, history can retain another value, but still cannot claim an impossible cap, an absent required row or a nonessential archetype.

The save's sparse table compares an in-range missing lane as zero and its greatest index bounds its row count. The absence of a recorded table or a country outside its sampled slice does not prove zero. A wrong saved-only or recording-only row counts as a difference when its owner was actually read at the compared boundary.

[Rebuilt stock and sparse row tests](../../crates/recorder-oracle/src/oracle/tests.rs)
