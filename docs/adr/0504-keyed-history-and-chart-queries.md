# Separate keyed observations, exact samples, and chart summaries

A keyed window records which hours were observed and the read that supplies an observed hour. Its decimal runs preserve exact values and observation breaks. A missing key in a known hour means there was no value for it, while an hour outside known spans was not recorded. A daily display uses the last visible hour of each clipped day. Keyboard and Jump navigation stop at changes in observed presence or value. A composite name follows the service's delimiter contract, including tag and owner slot.

Each chart line carries a country and a part. Tile summaries use one lane per declared series and load only bins the view needs, with every part of a country sharing that country's membership spans. Tile replies validate the requested line order. A landed tile wakes chart drawing through the latest lane without a React render during playback. Read exact state on hover or an explicit reached-hour request, use the followed answer at the live head, and keep the continuously moving edge on tile summaries. Parts are selected using the service's owner tag and low key bits. An absent part at the head stays absent and its missing reason stays associated with its read.

[Series data](../../ui/src/dashboard/keyedSeriesData.ts) · [Stability detail projection](../../ui/src/widgets/stability/stabilityDetailProjection.ts)
