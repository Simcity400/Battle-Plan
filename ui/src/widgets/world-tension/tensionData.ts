import { clipToSpans, runAt } from '../../charts/series.ts';
import type { Breakable, Span } from '../../charts/series';
import { buildStack } from '../../charts/stack.ts';
import type { StackModel, StackRun } from '../../charts/stack';
import { fixedNumber, formatAxisValue, formatFixed } from '../../charts/valueAxis.ts';
import { Lineage } from '../../dashboard/identity.ts';
import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import type { CountryIdentityRun, CountryMetadata, FixedRun, TensionCountry, TensionEvent, TensionRead, TensionReply } from '../../generated/dashboard';

/** World tension and each country's contribution to it (the `world-tension`
 * product): lookups by hour, the layers ranked at a moment, and the events. Values
 * stay the recorded integers; `percent` turns one into a plotting number. */

const RAW = /^-?\d{1,20}$/;

/** Decimal places of a power-of-ten scale, or null for any other scale. */
export function decimalsOf(scale: number): number | null {
  const digits = Math.log10(scale);
  return Number.isInteger(digits) && digits >= 0 && 10 ** digits === scale ? digits : null;
}

/** A country's layer key: its identity (canonical slot) with the tag it held, so two
 * countries sharing a tag are two layers. No tag holds a NUL, so no key is another's. */
export const countryKey = (country: Pick<TensionCountry, 'tag' | 'slot'>) => `${country.tag}\u0000${country.slot}`;

/** Checks the reply's shape: scales are powers of ten, raw values are integers,
 * reads and runs are in hour order and never overlap, and no country is listed twice. */
export function validateTensionReply(data: unknown): TensionReply {
  const reply = data as TensionReply;
  const spans = <T extends { firstHour: number; lastHour: number }>(items: readonly T[]) => items.every((item, index) =>
    Number.isInteger(item.firstHour) && Number.isInteger(item.lastHour) && item.firstHour <= item.lastHour && (index === 0 || items[index - 1].lastHour < item.firstHour));
  const runs = (items: readonly FixedRun[]) => Array.isArray(items) && spans(items) && items.every(run => RAW.test(run.valueRaw));
  const read = (item: TensionRead) => RAW.test(item.tensionRaw) && RAW.test(item.sumRaw) && (item.unnamedRaw === null ? tensionReadPreparing(item, 'unnamedRaw') : RAW.test(item.unnamedRaw))
    && (!('preparing' in item) || Array.isArray(item.preparing) && item.preparing.every(field => typeof field === 'string'));
  const event = (item: TensionEvent) => Number.isInteger(item.hour) && typeof item.reason === 'string' && (item.tag == null || typeof item.tag === 'string')
    && Array.isArray(item.added) && item.added.length > 0 && item.added.every(added => Number.isInteger(added.hour) && RAW.test(added.addedRaw));
  const ok = typeof reply === 'object' && reply != null && decimalsOf(reply.tensionScale) != null && decimalsOf(reply.contributionScale) != null
    && RAW.test(reply.ceilingRaw) && Array.isArray(reply.reads) && spans(reply.reads) && reply.reads.every(read)
    && Array.isArray(reply.countries) && reply.countries.every(country => typeof country.tag === 'string' && Number.isInteger(country.slot) && runs(country.runs)
      && (country.identities === undefined || (Array.isArray(country.identities) && spans(country.identities)
        && country.identities.every(identity => Number.isInteger(identity.slot) && identity.slot === country.slot && Number.isInteger(identity.countryId)))))
    && new Set(reply.countries.map(countryKey)).size === reply.countries.length
    && Array.isArray(reply.events) && reply.events.every(event) && Array.isArray(reply.warnings);
  if (!ok) throw new Error('The world tension reply is malformed.');
  return reply;
}

/** A reply with its lookups built once. Countries are keyed by `countryKey`. */
export interface TensionModel {
  reply: TensionReply;
  tensionDecimals: number;
  contributionDecimals: number;
  /** The raw runs, for exact labels, by key. */
  raw: ReadonlyMap<string, readonly FixedRun[]>;
  /** The tag each key's country held, to name and colour it. */
  tagOf: ReadonlyMap<string, string>;
  /** Countries behind each layer, keeping canonical slots distinct even when tags coincide. */
  countryOf: ReadonlyMap<string, string>;
  identities: ReadonlyMap<string, readonly CountryIdentityRun[]>;
  nativeKeys: ReadonlySet<string>;
  firstContributions: ReadonlyMap<string,number>;
  lineage: Lineage;
}

export function tensionModel(reply: TensionReply, lineage = new Lineage([]), metadata: readonly CountryMetadata[] = [],
  firstContributions = new Map(reply.countries.flatMap(country => country.runs.length ? [[countryKey(country),country.runs[0].firstHour] as const] : []))): TensionModel {
  const contributionDecimals = decimalsOf(reply.contributionScale)!;
  const raw = new Map(reply.countries.map(country => [countryKey(country), country.runs] as const));
  const tagOf = new Map(reply.countries.map(country => [countryKey(country), country.tag] as const));
  const slots = new Map<number, string>();
  for (const country of metadata) {
    if (country.observedBranchId === reply.branchId && !slots.has(Number(country.id))) {
      slots.set(Number(country.id), lineage.countryAt(country.tag, country.observedHour));
    }
  }
  // The earliest observation ties every tag entry of one canonical slot to the
  // same country. Metadata can date a dead country's tag before it was reused.
  const observed = [...reply.countries].sort((a, b) => (a.runs[0]?.firstHour ?? Infinity) - (b.runs[0]?.firstHour ?? Infinity));
  for (const country of observed) if (!slots.has(country.slot)) slots.set(country.slot, lineage.countryAt(country.tag, country.runs[0]?.firstHour ?? 0));
  const countryOf = new Map(reply.countries.map(country => [countryKey(country), slots.get(country.slot)!]));
  const identities = new Map(reply.countries.map(country => [countryKey(country), country.identities ?? []]));
  const nativeKeys = new Set(reply.countries.filter(country => country.identities !== undefined).map(countryKey));
  return { reply, tensionDecimals: decimalsOf(reply.tensionScale)!, contributionDecimals, raw, tagOf, countryOf, identities, nativeKeys, lineage,firstContributions };
}

/** A native layer's own proved alias at this hour; only omitted suffixes use the
 * historical rename path. Missing native spans never acquire a label. */
export const layerTagAt = (model: TensionModel, key: string, hour: number): string | null => model.nativeKeys.has(key)
  ? runAt(model.identities.get(key) ?? [], Math.floor(hour)) ? model.tagOf.get(key)! : null
  : model.lineage.tagOf(model.countryOf.get(key)!, hour);

/** The read holding an hour, or null where none was recorded. */
export function readAt(model: TensionModel, hour: number): TensionRead | null {
  return runAt(model.reply.reads, Math.floor(hour));
}

/** A country's contribution at an hour (by key), raw: "0" in a read it has no run
 * in, null where no read holds the hour. */
export function contributionAt(model: TensionModel, key: string, hour: number): string | null {
  if (!readAt(model, hour)) return null;
  return runAt(model.raw.get(key) ?? [], Math.floor(hour))?.valueRaw
    ?? (tensionReadPreparing(readAt(model, hour)!, 'historicalContributors') ? null : '0');
}

/** World tension and contributions as percentages of world tension. */
export const tensionPercent = (model: TensionModel, raw: string) => fixedNumber(raw, model.tensionDecimals);
export const contributionPercent = (model: TensionModel, raw: string) => fixedNumber(raw, model.contributionDecimals);

/** Exact percentages, as labels show them. */
export function formatTension(model: TensionModel, raw: string, digits = 1): string {
  return `${formatFixed(raw, model.tensionDecimals, digits)}%`;
}
export function formatContribution(model: TensionModel, raw: string, signed = false, digits = 2): string {
  return `${signed && BigInt(raw) > 0n ? '+' : ''}${formatFixed(raw, model.contributionDecimals, digits)}%`;
}

/** A value-axis label: a percentage with the decimals the ticks drawn with it need
 * (the kit's exact `formatAxisValue`), so ticks zoomed in close never read the same. */
export const formatTensionAxis = (value: number, ticks: readonly number[]) => `${formatAxisValue(value, ticks)}%`;

/** Every country contributing at an hour (not zero), largest first, ties by key (tag
 * first): the order the layers and the lists take. `among` narrows it to chosen
 * countries, by tag (null: every country). */
export function rankedAt(model: TensionModel, hour: number, among: ReadonlySet<string> | null = null,
  countryIds?: ReadonlySet<string> | null): { key: string; tag: string | null; raw: string }[] {
  if (!readAt(model, hour)) return [];
  const ranked: { key: string; tag: string | null; raw: string; value: bigint }[] = [];
  for (const [key, runs] of model.raw) {
    const tag = layerTagAt(model, key, hour);
    const identity = runAt(model.identities.get(key) ?? [], Math.floor(hour));
    if (countryIds ? identity === null || !countryIds.has(countryIdentityKey(identity)) : among && (tag === null || !among.has(tag))) continue;
    const run = runAt(runs, Math.floor(hour));
    if (run && run.valueRaw !== '0') ranked.push({ key, tag, raw: run.valueRaw, value: BigInt(run.valueRaw) });
  }
  ranked.sort((a, b) => (a.value < b.value ? 1 : a.value > b.value ? -1 : a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  return ranked.map(({ key, tag, raw }) => ({ key, tag, raw }));
}

/** The layers at a moment, by key: the `count` largest contributors there (fewer
 * when fewer contribute), among the chosen countries when `among` is set, in rank
 * order. Every other country is "Other". */
export function layerKeys(model: TensionModel, hour: number, count: number, among: ReadonlySet<string> | null = null,
  countryIds?: ReadonlySet<string> | null): string[] {
  return rankedAt(model, hour, among, countryIds).slice(0, Math.max(0, count)).map(item => item.key);
}

/** What the layers other than `keys` add up to in a read, raw: the read's sum less
 * the named layers (so it includes contributions no recorded country names). */
export function otherRaw(model: TensionModel, read: TensionRead, keys: readonly string[]): string {
  let rest = BigInt(read.sumRaw);
  for (const key of keys) rest -= BigInt(runAt(model.raw.get(key) ?? [], read.firstHour)?.valueRaw ?? '0');
  return rest.toString();
}

/** A contribution's share of world tension in a read, in contribution units: the
 * contribution scaled by world tension over the sum of every contribution, so the
 * shares add up to world tension (the game caps it at 100% and at zero, while the
 * sum can pass either). Zero when the sum is not positive, as world tension is then
 * zero. Exact to the unit, rounded toward zero. */
export function shareRaw(model: TensionModel, read: TensionRead, raw: string): string {
  const sum = BigInt(read.sumRaw);
  if (sum <= 0n) return '0';
  // World tension in contribution units.
  const tension = BigInt(read.tensionRaw) * BigInt(model.reply.contributionScale / model.reply.tensionScale);
  return (BigInt(raw) * tension / sum).toString();
}

/** A layer's value in a read as drawn: its share of world tension, or its recorded contribution. */
export function layerRaw(model: TensionModel, read: TensionRead, raw: string, share: boolean): string {
  return share ? shareRaw(model, read, raw) : raw;
}

/** The same native-holder and membership clipping for exact stacks and summary requests. */
export function selectedContributionSpans<T extends Breakable>(model: TensionModel, key: string, runs: readonly T[],
  membership: ReadonlyMap<string, readonly Span[]> | null, membersAreCountries: boolean, countryIds: ReadonlySet<string> | null): readonly T[] {

    if (!countryIds && !(membership && membersAreCountries)) return membership ? clipToSpans(runs, membership.get(model.tagOf.get(key)!) ?? []) : runs;
    const byCountry = new Map<string, CountryIdentityRun[]>();
    for (const identity of model.identities.get(key) ?? []) {
      const key = countryIdentityKey(identity);
      if (countryIds && !countryIds.has(key)) continue;
      const spans = byCountry.get(key);
      if (spans) spans.push(identity); else byCountry.set(key, [identity]);
    }
    const known = [...byCountry].flatMap(([key, spans]) => membership && membersAreCountries
      ? clipToSpans(spans, membership.get(key) ?? []) : spans);
    const selected = clipToSpans(runs, known);
    return membership && !membersAreCountries ? clipToSpans(selected, membership.get(model.tagOf.get(key)!) ?? []) : selected;
}

/** The stack of the `keys` countries' contributions and Other (shares of world
 * tension, or as recorded), as plotting percentages: a run per read and layer, each
 * held until the next read, for the kit's signed stack (`buildStack`). It breaks
 * where a read follows a gap and where a layer's own run starts a segment, so the
 * stack never joins what the reply keeps apart. */
export function tensionStack(model: TensionModel, keys: readonly string[], share: boolean,
  membership: ReadonlyMap<string, readonly Span[]> | null = null, membersAreCountries = false,
  countryIds: ReadonlySet<string> | null = null): StackModel {
  const bands: StackRun[][] = Array.from({ length: keys.length + 1 }, () => []);
  const sources = keys.map(key => model.raw.get(key) ?? []);
  const held = sources.map((runs,index) => selectedContributionSpans(model,keys[index],runs,membership,membersAreCountries,countryIds));
  // Membership can change between daily reads. Split there too, moving the
  // contribution to Other while keeping the value supplied by the same read.
  const cuts = membership || countryIds ? [...new Set(held.flatMap(runs => runs.flatMap(run => [run.firstHour, run.lastHour + 1])))].sort((a, b) => a - b) : [];
  let cut = 0;
  for (const read of model.reply.reads) {
    if (tensionReadPreparing(read, 'historicalContributors')) continue;
    while (cut < cuts.length && cuts[cut] <= read.firstHour) cut++;
    let firstHour = read.firstHour;
    while (firstHour <= read.lastHour) {
      const lastHour = Math.min(read.lastHour, (cuts[cut] ?? Infinity) - 1);
      const raws = held.map(runs => runAt(runs, firstHour)?.valueRaw ?? '0');
      const rest = BigInt(read.sumRaw) - raws.reduce((sum, raw) => sum + BigInt(raw), 0n);
      [...raws, rest.toString()].forEach((raw, layer) => {
        const run = layer < sources.length ? runAt(sources[layer], firstHour) : null;
        // A membership boundary is known data, not a recording gap.
        const startsSegment = (firstHour === read.firstHour && read.startsSegment)
          || (run != null && run.firstHour === firstHour && run.startsSegment);
        bands[layer].push({ firstHour, lastHour, startsSegment, value: contributionPercent(model, layerRaw(model, read, raw, share)) });
      });
      firstHour = lastHour + 1;
      if (cuts[cut] === firstHour) cut++;
    }
  }
  return buildStack(bands);
}

/** The same sampled hours for the layers and the world tension line. A final
 * zero-width band carries the recorded total through the stack's clipping and
 * sampling even when contributions differ. It is stroked, not filled. */
export function tensionPlot(model: TensionModel, keys: readonly string[], share: boolean,
  membership: ReadonlyMap<string, readonly Span[]> | null = null, membersAreCountries = false,
  countryIds: ReadonlySet<string> | null = null): StackModel {
  const stack = tensionStack(model, keys, share, membership, membersAreCountries, countryIds);
  stack.edges.forEach((edges, index) => {
    if (!edges) return;
    const value = tensionPercent(model, readAt(model, stack.hours[index])!.tensionRaw);
    edges.push({ low: value, high: value });
  });
  return stack;
}

/** What an event adds as known at an hour: the value of the latest read at or
 * before it, else (an hour before its first read) its first read's. */
export function addedAt(event: TensionEvent, hour: number): string {
  let found = event.added[0];
  for (const added of event.added) if (added.hour <= hour) found = added;
  return found.addedRaw;
}

/** Several events of one hour, for a compact tooltip: the tension they generated
 * together (each as first read), and the events largest first (by size, either
 * sign), ties in their order. */
export function eventGroup(events: readonly TensionEvent[]): { totalRaw: string; largest: TensionEvent[] } {
  const size = (event: TensionEvent) => { const raw = BigInt(event.added[0].addedRaw); return raw < 0n ? -raw : raw; };
  const totalRaw = events.reduce((sum, event) => sum + BigInt(event.added[0].addedRaw), 0n).toString();
  const largest = events.map((event, index) => ({ event, index }))
    .sort((a, b) => (size(a.event) < size(b.event) ? 1 : size(a.event) > size(b.event) ? -1 : a.index - b.index)).map(item => item.event);
  return { totalRaw, largest };
}

/** The events the chart marks: those dated inside the hours shown. */
export function eventsWithin(events: readonly TensionEvent[], from: number, to: number): TensionEvent[] {
  return events.filter(event => event.hour >= from && event.hour <= to);
}

/** The events marked at an hour no read holds (dated inside a gap, or before the
 * first read), for a tooltip with no badge under it: a keyboard stop at such an
 * event shows it as hovering its marker does, not only "Not recorded". None where a
 * read holds the hour, whose tooltip lists its events itself. */
export function eventsOutsideReads(model: TensionModel, events: readonly TensionEvent[], hour: number): TensionEvent[] {
  const at = Math.floor(hour);
  return readAt(model, at) ? [] : events.filter(event => event.hour === at);
}

/** Frame-only reads cannot establish historical contributor zeroes or unnamed attribution. */
export function tensionReadPreparing(read: TensionRead, field: string): boolean {
  return 'preparing' in read && Array.isArray(read.preparing) && read.preparing.includes(field);
}
