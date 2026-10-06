# Occupation trend

The `occupation-trend` product reuses the Occupied states reader. This widget
starts on all occupiers and lines over time, with resistance selected. Countries
can be chosen or narrowed to any recorded smart group, including the top 10 by
the selected measure. Every recorded country is eligible, independently of
human/AI status. Tags remain opaque.
Chosen countries and smart groups have no widget country cap; only the shared
product reply budget can refuse an oversized response.

The mean ports `140FE98F0`: unweighted recorded state values and the native
wrapping signed fixed5 arithmetic. It extends that state mean across all of an
occupier's occupied-country groups. The cached native group means are not
recorded: `140FE1EC0` refreshes them before updating the state scalars. This
widget therefore computes its mean from the scalars at their supplying read,
without claiming the native cached country-row value. Active occupation membership
follows the Occupied states product's native garrison/resistance rules. Neither
legal ownership nor controller different from owner alone defines occupation.
Only readable held state values contribute; a tooltip names the number covered
out of the held-state count. An incomplete held inventory says the total is not
recorded. An occupier with no readable held state has a gap, including after it
ceases to exist. Nothing is interpolated or supplied as zero.

The shared [CountryRaceView](../../charts/race/CountryRaceView.tsx) draws the
Style A clean lines, ringed end dots, labels and hairline grid from shared chart
tokens. Its View setting also offers a race at the shown moment and one
country's independent measure lines. Resistance is solid; in Both, compliance
uses the shared secondary dash. A separate setting chooses which measure ranks
Both. Chart labels default to two decimal places, configurable through five;
tooltips retain the full fixed5 mean. Shared playback, end-label, axis and corner
date settings remain available. Lines use smooth monotone curves and race bars
ease within an unchanged recorded stretch; gaps still break the drawing. These
presentation choices never change the recorded tooltip values. Right-click
offers Jump to that moment.

Race row names and flags use the shared sampled country looks, held within the
same identity segment while the next read loads. Country names and flags in
tooltips come from the hovered hour through the
shared country-label hook, including renamed occupiers and occupied countries.
Each average and collaboration value names its recorded supplying read directly.
Read dates never become change dates.

Country collaboration context is optional and off by default. It comes from
`CountryCollaboration`, shown natively in Politics → Exiles & Collaborations,
and is explicitly separate from state compliance. When enabled, each occupied
country's stored ratio is shown as a percentage with its own read date. Missing
collaboration says Not recorded. Content names, colours and flags resolve
through the recording's own content stack; this widget defines no game content
catalogue or asset names.
The request ships collaboration context and its additional run boundaries only
when the setting is enabled.

Node tests cover gained/lost held states, partial coverage, unread hours,
countries ceasing to exist, hovered identities, exact ranking before geometry,
two independent measures, missing collaboration and 5,000 countries. Browser
layout, paint and live comparison remain unverified.
