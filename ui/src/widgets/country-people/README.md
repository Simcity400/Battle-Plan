# Country manpower and politics details

Shared reached-hour details embedded in the Army manpower breakdown and Ruling party view. The `country-people` product reads only the selected frame, with the recorded content stack supplying recruitment defines and modifier semantics.

The manpower view shows available, locked and total pools; applied and target recruitment ratios, daily mobilisation and remaining mobilisation; effective recruitable population; deployed army, navy and air crews; and required navy crews. State rows show their available/locked/total pools and effective recruitment. The politics view shows election eligibility, interval and next date. Both expose the selected exile and governments it hosts, including host, legitimacy, manpower pool/cap and recruited army leaders.

State rows and exile cards are windowed. Exact integer strings stay exact until formatting. Sliced country facts show their supplying date; missing operands stay unavailable, with named/countable diagnostics forwarded to the parent widget. Request identity includes source, revision, branch, country and reached hour.

Run the focused formatting/validation tests with `node --experimental-strip-types --test ui/src/widgets/country-people/peopleData.test.ts` from the repository root.

Exile cards show deployed air crews attributed to the exile, including host-owned wings. The manpower details keep ordinary and exile air manpower separate. Army totals use template origin and native ownership, and effective recruitable population includes the country residual.
