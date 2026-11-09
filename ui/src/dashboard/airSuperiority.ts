import { countryNameDetails } from '../charts/tooltip.ts';
import type { AirCell, AirReply as WireAirReply, AirSide, AirSpan } from '../generated/dashboard';
import type { HeatPeriod, HeatRow, HeatTail } from '../charts/heat/layout';
import type { TooltipContent } from '../charts/tooltip';
import type { CountryLabel } from './countryFlagsAt';
import { formatAmount, rescale } from '../charts/valueAxis.ts';
import { int, object } from './validation.ts';
import type { PreparedCoverage } from '../shell/preparation/contract';
import { preparedPending } from './preparedPrefix.ts';
import { isOrderedSpans } from '../charts/series.ts';
import { runAt } from '../charts/series.ts';

export type AirTimeline = Extract<WireAirReply, { view: 'timeline' }> & { coverage?: PreparedCoverage; approximate?: AirSpan[]; pending?: AirSpan[]; rowsApproximate?: boolean; sampleHours?: (number | null)[] };
export type AirMoment = Omit<Extract<WireAirReply, { view: 'moment' }>, 'sides'> & { sides: AirSide[] };
export type AirChoices = Extract<WireAirReply, { view: 'choices' }>;
type AirReply = AirTimeline | AirMoment | AirChoices;
export type AirState = 'known' | 'unknown' | 'inactive' | 'filtered' | 'no-air-object' | 'gap';
export interface AirPart { id: string; label: string; color: string; share: number; score: bigint; side?: AirSide; tag?: string }
export interface AirView { state: AirState; parts: readonly AirPart[]; total: bigint; unknown: readonly AirSide[]; noAirObject: readonly AirSide[]; excluded: number; reason?: string; wings: number | null; hour: number | null; theatreRead?: number; scale: number; badBelow: number; goodAbove: number; viewpoint: AirLooks['viewpoint'] }
/** `lookAt` names what `color` and `viewpointAt` give at an hour: hours with the same
 * value look alike, so a projection need not ask again for each. */
export interface AirLooks { name(side: AirSide): string; color(side: AirSide, hour?: number): string; viewpointAt?(hour: number): AirLooks['viewpoint']; lookAt?(hour: number): unknown; viewpoint: { name: string; tag?: string; color: string }; enemy: string; scale: number; badBelow: number; goodAbove: number; mode?: 'sides' | 'viewpoint' }
export interface AirTimelineViews { rows: HeatRow[]; periods: HeatPeriod[]; tail?: HeatTail; viewAt(region: number, day: number): AirView | undefined }
export const FILTERED_NOTE = 'No selected side has a score (others may)';

export const airSegmentAt = (segments: Readonly<Partial<Record<string, readonly AirSpan[]>>>, tag: string, hour: number): number | null =>
  runAt(segments[tag] ?? [], hour)?.firstHour ?? null;

export function airTooltipTags(view: AirView): string[] {
  return [...new Set([
    ...view.parts.flatMap(part => [part.tag, part.side?.tag, part.side?.leader].flatMap(tag => tag ? [tag] : [])),
    ...[...view.unknown, ...view.noAirObject].flatMap(side => [side.leader, side.tag].flatMap(tag => tag ? [tag] : [])),
    ...(view.viewpoint.tag ? [view.viewpoint.tag] : []),
  ])];
}

export function airPercent(score: bigint, total: bigint): string { return total > 0n ? `${(score * 100n + total / 2n) / total}%` : '—'; }
/** A raw score in units of `scale` (the reply's denominator), as the game shows it. */
export const airScore = (score: bigint, scale: number): string => formatAmount(rescale(score, scale, 2), 2);

/** Dictionary order is first appearance; factions use numeric ids, never decimal-string ordering. */
export function sideOrder(sides: readonly AirSide[]): Map<string, number> {
  const factions = [...new Set(sides.flatMap(side => side.faction == null ? [] : [side.faction]))].sort((a, b) => {
    const first = BigInt(a), second = BigInt(b); return first < second ? -1 : first > second ? 1 : 0;
  });
  const ranks = new Map(factions.map((id, index) => [id, index]));
  const order = new Map<string, number>();
  for (const side of sides) if (!order.has(side.key)) order.set(side.key, side.faction == null ? factions.length + order.size : ranks.get(side.faction)!);
  return order;
}

/** Present country labels are separate from the historical anchor's colour read. */
export function airSideTag(side: AirSide): string | undefined {
  return side.tag ?? side.leader ?? undefined;
}

/** Names follow present members; the historical key is only the colour identity. */
export function airSideName(side: AirSide, country: (tag: string) => string, factions?: ReadonlyMap<string, string | null>): string {
  if (side.factions?.length) return side.factions.map(([id, name]) => factions?.get(id) ?? name).join(' / ');
  if (side.faction) return factions?.get(side.faction) ?? side.name;
  const tag = side.tag ?? side.leader;
  const name = tag ? country(tag) : 'Coalition (name not recorded)';
  return side.key.startsWith('w:') && side.coBelligerents > 0 ? `${name} and allies` : name;
}

/** How many of the side's reads are dated at or before `hour`. */
function readsBy(side: AirSide, hour: number): number {
  const reads = side.reads ?? [];
  let lo = 0, hi = reads.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (reads[mid][0] <= hour) lo = mid + 1; else hi = mid; }
  return lo;
}

export function sideAt(side: AirSide, hour: number): AirSide {
  const read = side.reads?.[readsBy(side, hour) - 1];
  return read ? { ...side, name: read[1], leader: read[2], tag: read[3], coBelligerents: read[4], factions: read[5], colourFaction: read[6] } : side;
}

/** Recorded scores as exact integers, parsed once per reply rather than per hour shown. */
const exactScores = new WeakMap<object, { values: readonly bigint[]; total: bigint }>();
function scoresOf(scores: readonly (string | readonly [number, string])[]): { values: readonly bigint[]; total: bigint } {
  let exact = exactScores.get(scores);
  if (!exact) {
    const values = scores.map(score => BigInt(typeof score === 'string' ? score : score[1]));
    exact = { values, total: values.reduce((sum, value) => sum + value, 0n) };
    exactScores.set(scores, exact);
  }
  return exact;
}

/** Looks include the regional coalition winner, even when it never led a faction. */
export function airCountryTags(reply: AirReply): string[] {
  const tags = new Set<string>();
  const add = (tag: string | null) => { if (tag != null) tags.add(tag); };
  for (const side of reply.sides) {
    add(side.leader); add(side.tag); add(airSideTag(side) ?? null);
    for (const read of side.reads ?? []) { add(read[2]); add(read[3]); }
  }
  const cell = (value: AirCell) => {
    if (value == null || typeof value === 'string' || Array.isArray(value)) return;
    if ('view' in value) add(value.leader);
    else for (const [, tag] of value.viewpoints ?? []) add(tag);
  };
  if (reply.view === 'timeline') for (const row of reply.rows) for (const [, value] of row.cells) cell(value);
  else if (reply.view === 'moment') {
    for (const row of reply.regions) for (const [, , value] of row.runs) cell(value);
    for (const [, , value] of reply.fallback) cell(value);
  }
  return [...tags];
}

export function cellView(cell: AirCell, sides: readonly AirSide[], looks: AirLooks, order: ReadonlyMap<string, number>, hour: number | null, wings: number | null = null): AirView {
  const base: AirView = { state: 'inactive', parts: [], total: 0n, unknown: [], noAirObject: [], excluded: 0, wings, hour, scale: looks.scale, badBelow: looks.badBelow, goodAbove: looks.goodAbove, viewpoint: hour == null ? looks.viewpoint : looks.viewpointAt?.(hour) ?? looks.viewpoint };
  if (hour == null) return { ...base, state: 'gap' };
  if (typeof cell === 'string') return { ...base, state: cell === 'no-air-object' ? 'no-air-object' : cell === 'outside-theatres' ? 'filtered' : 'unknown', reason: cell };
  if (cell == null) return base;
  const rawSide = !Array.isArray(cell) && 'view' in cell ? sideAt(sides[cell.side], hour) : undefined;
  const sideView = rawSide;
  const pair = sideView ? (cell as { view: [string, string] }).view : Array.isArray(cell) && typeof cell[0] === 'string' ? cell as [string, string] : null;
  if (pair) {
    const viewpoint = sideView ? { name: looks.name(sideView), color: looks.color(sideView, hour), tag: !Array.isArray(cell) && 'view' in cell ? cell.leader ?? undefined : undefined } : base.viewpoint;
    const { values: [friendly, enemy], total } = scoresOf(pair);
    return { ...base, viewpoint, state: total ? 'known' : 'inactive', total, parts: [
      { id: 'friendly', label: `${viewpoint.name} (friendly)`, color: viewpoint.color, score: friendly, share: total ? Number(friendly) / Number(total) : 0, tag: viewpoint.tag, side: sideView },
      { id: 'enemy', label: `Enemies of ${viewpoint.name}`, color: looks.enemy, score: enemy, share: total ? Number(enemy) / Number(total) : 0 },
    ] };
  }
  const scores = Array.isArray(cell) ? cell as [number, string][] : (cell as { scores: [number, string][] }).scores;
  const { values, total } = scoresOf(scores);
  const leaders = !Array.isArray(cell) && 'scores' in cell ? new Map(cell.viewpoints ?? []) : null;
  const parts = scores.map(([index], at) => {
    const baseSide = sideAt(sides[index], hour);
    const leader = leaders?.get(index);
    const side = baseSide, score = values[at];
    return { id: side.key, label: looks.name(side), color: looks.color(side, hour), score, share: total ? Number(score) / Number(total) : 0, side, tag: leader ?? side.leader ?? side.tag ?? undefined };
  }).sort((a, b) => (order.get(a.id) ?? 0) - (order.get(b.id) ?? 0));
  const details = Array.isArray(cell) ? {} : cell as { unknown?: number[]; noAirObject?: number[]; excluded?: number; filtered?: boolean };
  const unknown = (details.unknown ?? []).map(index => sideAt(sides[index], hour));
  const noAirObject = (details.noAirObject ?? []).map(index => sideAt(sides[index], hour));
  const missingSide = looks.mode === 'viewpoint' ? unknown[0] ?? noAirObject[0] : undefined;
  const viewpoint = missingSide ? { name: looks.name(missingSide), color: looks.color(missingSide, hour), tag: missingSide.leader ?? missingSide.tag ?? undefined } : base.viewpoint;
  return { ...base, total, parts, unknown, noAirObject, viewpoint, excluded: details.excluded ?? 0,
    state: unknown.length ? 'unknown' : total ? 'known' : looks.mode === 'viewpoint' && noAirObject.length ? 'no-air-object' : details.filtered ? 'filtered' : 'inactive' };
}

export function airPeriods(reply: AirTimeline): HeatPeriod[] {
  return reply.days.map((readHour, index) => ({ firstHour: reply.firstHour + index * reply.dayHours, lastHour: Math.min(reply.lastHour, reply.firstHour + (index + 1) * reply.dayHours - 1), readHour: reply.sampleHours?.[index] ?? readHour }));
}

/** The sides whose dated reads shape a cell's parts. */
function partSides(cell: AirCell, sides: readonly AirSide[]): AirSide[] {
  if (cell == null || typeof cell === 'string') return [];
  if (!Array.isArray(cell) && 'view' in cell) return [sides[cell.side]];
  if (Array.isArray(cell) && typeof cell[0] === 'string') return [];
  return (Array.isArray(cell) ? cell as [number, string][] : (cell as { scores: [number, string][] }).scores).map(([index]) => sides[index]);
}

/** Only the server-selected rows are projected. Equal RLE cells share one immutable
 * geometry; a day whose sides and looks read as the day before's takes its geometry
 * without projecting the cell again. Day views are projected when asked for. */
export function timelineViews(reply: AirTimeline, looks: AirLooks): AirTimelineViews {
  const order = sideOrder(reply.sides), runs = new Map<number, { start: number; cell: AirCell; view: AirView }[]>();
  const rows = reply.rows.map(row => {
    const cells: HeatRow['cells'][number][] = [], starts: { start: number; cell: AirCell; view: AirView }[] = [];
    for (const [repeat, cell] of row.cells) {
      const start = cells.length;
      const view = cellView(cell, reply.sides, looks, order, reply.sampleHours?.[start] ?? reply.days[start] ?? 0);
      starts.push({ start, cell, view });
      const geometry = { parts: view.parts, unknown: view.state === 'unknown', filtered: view.reason === 'outside-theatres' };
      const actors = partSides(cell, reply.sides);
      let before: { read: number; cell: HeatRow['cells'][number] } | null = null;
      for (let day = start; day < start + repeat; day++) {
        const read = reply.sampleHours?.[day] ?? reply.days[day];
        if (read == null || day === start) { cells.push(geometry); if (read != null) before = { read, cell: geometry }; continue; }
        let shown: HeatRow['cells'][number];
        const previous = before;
        if (looks.lookAt && previous && looks.lookAt(read) === looks.lookAt(previous.read) && actors.every(side => readsBy(side, read) === readsBy(side, previous.read))) {
          // Alike in every input, so alike in parts; a part list of its own keeps its own stripe.
          shown = previous.cell === geometry ? geometry : { ...previous.cell, parts: [...previous.cell.parts] };
        } else {
          const projected = cellView(cell, reply.sides, looks, order, read);
          shown = projected.parts.every((part, index) => part.id === geometry.parts[index]?.id && part.color === geometry.parts[index]?.color) ? geometry
            : { parts: projected.parts, unknown: projected.state === 'unknown', filtered: projected.reason === 'outside-theatres' };
        }
        cells.push(shown);
        before = { read, cell: shown };
      }
    }
    runs.set(row.region, starts);
    return { id: String(row.region), label: row.name, cells };
  });
  const viewAt = (region: number, day: number): AirView | undefined => {
    const starts = runs.get(region);
    if (!starts || !Number.isInteger(day) || day < 0 || day >= reply.days.length) return undefined;
    let low = 0, high = starts.length;
    while (low < high) { const middle = (low + high) >>> 1; if (starts[middle].start <= day) low = middle + 1; else high = middle; }
    const { cell, view } = starts[low - 1];
    const displayed = reply.days[day];
    const read = reply.sampleHours?.[day] ?? displayed;
    const dated = read == null ? view : cellView(cell, reply.sides, looks, order, read);
    const excluded = view.excluded + (read == null ? 0 : tupleAt(reply.excluded, displayed ?? read) ?? 0);
    const theatreRead = read == null ? undefined : tupleAt(reply.theatreReads, displayed ?? read);
    return excluded === dated.excluded && theatreRead === undefined ? dated : { ...dated, excluded, theatreRead };
  };
  return { rows, periods: airPeriods(reply), viewAt };
}

export function tupleAt<T>(runs: readonly [number, number, T][], hour: number): T | undefined {
  let low = 0, high = runs.length;
  while (low < high) { const middle = (low + high) >>> 1; if (runs[middle][0] <= hour) low = middle + 1; else high = middle; }
  const run = runs[low - 1]; return run && hour <= run[1] ? run[2] : undefined;
}
export function inSpans(spans: readonly { firstHour: number; lastHour: number }[], hour: number): boolean {
  let low = 0, high = spans.length;
  while (low < high) { const middle = (low + high) >>> 1; if (spans[middle].firstHour <= hour) low = middle + 1; else high = middle; }
  return low > 0 && hour <= spans[low - 1].lastHour;
}
/** A partial day uses its last reached read, including before an intra-day gap. */
export function lastRecordedAt(spans: readonly AirSpan[], hour: number): number | null {
  let lo = 0, hi = spans.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (spans[mid].firstHour <= hour) lo = mid + 1; else hi = mid; }
  return lo ? Math.min(hour, spans[lo - 1].lastHour) : null;
}
export interface AirRegionView { region: { id: number; name: string }; view: AirView; active: boolean }
/** Build once per window, then resolve playback with one binary search. */
export function momentReadHours(reply: AirMoment): number[] {
  const hours = new Set([reply.firstHour]);
  for (const spans of [reply.recorded, reply.complete, reply.wingsKnown]) for (const span of spans) {
    hours.add(span.firstHour); if (span.lastHour < reply.lastHour) hours.add(span.lastHour + 1);
  }
  for (const region of reply.regions) for (const runs of [region.runs, region.wings]) for (const [first, last] of runs) {
    hours.add(first); if (last < reply.lastHour) hours.add(last + 1);
  }
  for (const region of reply.regions) for (const span of region.active ?? []) {
    hours.add(span.firstHour); if (span.lastHour < reply.lastHour) hours.add(span.lastHour + 1);
  }
  for (const [first, last] of reply.fallback) { hours.add(first); if (last < reply.lastHour) hours.add(last + 1); }
  for (const [first, last] of [...reply.excluded, ...reply.theatreReads]) { hours.add(first); if (last < reply.lastHour) hours.add(last + 1); }
  for (const side of reply.sides) for (const [read] of side.reads ?? []) if (read >= reply.firstHour && read <= reply.lastHour) hours.add(read);
  return [...hours].sort((a, b) => a - b);
}
export function momentReadAt(hours: readonly number[], hour: number): number | null {
  let low = 0, high = hours.length;
  while (low < high) { const middle = (low + high) >>> 1; if (hours[middle] <= hour) low = middle + 1; else high = middle; }
  return hours[low - 1] ?? null;
}
export function momentViews(reply: AirMoment, hour: number, looks: AirLooks, choices?: Pick<AirChoices, 'regions'>): AirRegionView[] {
  const order = sideOrder(reply.sides), recorded = inSpans(reply.recorded, hour);
  const fallback = tupleAt(reply.fallback, hour);
  const rows: AirRegionView[] = reply.regions.map(region => {
    const supplied = tupleAt(region.runs, hour);
    const cell = supplied !== undefined ? supplied : fallback !== undefined ? fallback : inSpans(reply.complete, hour) ? null : 'scores-unavailable';
    const view = cellView(cell, reply.sides, looks, order, recorded ? hour : null, inSpans(reply.wingsKnown, hour) ? tupleAt(region.wings, hour) ?? 0 : null);
    return { region, view, active: inSpans(region.active ?? [], hour) || view.total > 0n || (view.wings ?? 0) > 0 };
  });
  if (choices) {
    const present = new Set(reply.regions.map(region => region.id));
    for (const region of choices.regions) if (!present.has(region.id)) rows.push({ region, active: false, view: cellView(fallback !== undefined ? fallback : inSpans(reply.complete, hour) ? null : 'scores-unavailable', reply.sides, looks, order, recorded ? hour : null, inSpans(reply.wingsKnown, hour) ? 0 : null) });
  }
  const excluded = tupleAt(reply.excluded ?? [], hour) ?? 0;
  const theatreRead = tupleAt(reply.theatreReads, hour);
  return rows.map(row => ({ ...row, view: { ...row.view, excluded: row.view.excluded + excluded, theatreRead } }));
}
export function orderedRegions(rows: readonly AirRegionView[], order: 'name' | 'active' | 'contested'): AirRegionView[] {
  const lead = (view: AirView) => { if (view.state !== 'known' || view.total <= 0n) return Infinity; let first = 0, second = 0; for (const part of view.parts) { if (part.share > first) { second = first; first = part.share; } else if (part.share > second) second = part.share; } return first - second; };
  return rows.filter(row => row.view.reason !== 'outside-theatres' && (order === 'name' || row.active || row.view.total > 0n || (row.view.wings ?? 0) > 0)).sort((a, b) => {
    if (order === 'name') return a.region.name.localeCompare(b.region.name) || a.region.id - b.region.id;
    if (order === 'contested') { const difference = lead(a.view) - lead(b.view); if (difference) return difference; }
    return a.view.total === b.view.total ? a.region.id - b.region.id : a.view.total > b.view.total ? -1 : 1;
  });
}

export function airState(view: AirView): string {
  if (view.state === 'filtered') return view.reason === 'outside-theatres' ? 'Outside selected air theatres' : FILTERED_NOTE;
  if (view.state === 'no-air-object') return 'No air data';
  if (view.state === 'gap') return 'Not recorded';
  if (view.state === 'unknown') return view.reason === 'side-not-present' ? 'Side not present at this hour' : 'Unknown';
  if (view.state === 'inactive') return 'No air activity';
  const friendly = view.parts.find(part => part.id === 'friendly');
  if (friendly) {
    // Signed multiplication wraps before division; thresholds are strict.
    // The maximum signed score counts as air superiority.
    if (friendly.score === 9223372036854775807n) return 'Air superiority';
    const denominator = BigInt.asIntN(64, view.total);
    if (!denominator) return 'Air superiority';
    const ratio = BigInt.asIntN(64, friendly.score * BigInt(view.scale)) / denominator;
    return ratio > BigInt(view.goodAbove) ? 'Air superiority' : ratio < BigInt(view.badBelow) ? 'Enemy superiority' : 'Contested';
  }
  return 'Scores recorded';
}
const AIR_MISSING_REASONS: Readonly<Record<string, string>> = {
  'side-not-present': 'Side not present at this hour.',
  'score-not-recorded': 'Scores were not recorded at this hour.',
  'scores-unavailable': 'Scores were not recorded at this hour.',
  'sides-not-recorded': 'Side membership was not recorded.',
  'countries-not-recorded': 'Countries unavailable.',
  'identity-not-recorded': 'Countries unavailable.',
  'theatres-not-recorded': 'Air groups unavailable.',
};
export function airTooltip(region: { name: string }, view: AirView, viewpoint: boolean, countryLabel: CountryLabel, filtered: boolean, factionNames?: ReadonlyMap<string, string | null>): TooltipContent {
  const named: ReturnType<CountryLabel>[] = [];
  const country: CountryLabel = tag => { const label = countryLabel(tag); named.push(label); return label; };
  const parts = [...view.parts].sort((a, b) => a.score === b.score ? 0 : a.score > b.score ? -1 : 1);
  const sideLabel = (side: AirSide) => airSideName(side, tag => country(tag).name, factionNames);
  const friendly = parts.find(part => part.id === 'friendly');
  const viewpointName = view.viewpoint.tag ? country(view.viewpoint.tag).name : friendly?.side ? sideLabel(friendly.side)
    : friendly?.label.replace(/ \(friendly\)$/, '') ?? view.viewpoint.name;
  // Air-map thresholds are supplied by the product.
  const overflows = friendly && (BigInt.asIntN(64, view.total) !== view.total || BigInt.asIntN(64, friendly.score * BigInt(view.scale)) !== friendly.score * BigInt(view.scale));
  const thresholds = `Above ${airPercent(BigInt(view.goodAbove), BigInt(view.scale))}: air superiority; below ${airPercent(BigInt(view.badBelow), BigInt(view.scale))}: enemy superiority; otherwise contested.`;
  const viewpointNote = `${overflows ? 'Game status differs from the displayed shares. ' : ''}${thresholds}`;
  const content: TooltipContent = { title: region.name, rows: [
    ...parts.map(part => ({ label: part.id === 'enemy' ? `Enemies of ${viewpointName}` : part.side ? `${sideLabel(part.side)}${part.id === 'friendly' ? ' (friendly)' : ''}` : part.tag ? `${country(part.tag).name}${part.id === 'friendly' ? ' (friendly)' : ''}` : part.label,
      countryFlag: part.tag ? country(part.tag) : undefined, swatch: part.color, value: `${view.state === 'unknown' ? 'Share unknown' : airPercent(part.score, view.total)} · ${airScore(part.score, view.scale)} score` })),
    ...parts.flatMap(part => part.side && part.side.coBelligerents ? [{ label: `${sideLabel(part.side)} with ${part.side.coBelligerents} co-belligerents`, value: part.tag ? `Viewpoint: ${country(part.tag).name}` : 'Leader not recorded', indent: true }] : []),
    ...(view.unknown.length ? [{ label: 'Unknown sides', value: String(view.unknown.length) }] : []),
    ...view.unknown.map(side => ({ label: sideLabel(side), value: 'Score unknown' })),
    ...view.noAirObject.map(side => ({ label: sideLabel(side), value: 'No air data' })),
    ...(view.excluded ? [{ label: 'Excluded sides', value: String(view.excluded) }] : []),
    ...(viewpoint ? [{ label: 'Viewpoint', value: viewpointName }] : []),
    { label: 'State', value: airState(view) },
    ...(view.wings == null ? [] : [{ label: 'Air wings assigned to the region', value: `${view.wings}` }]),
  ], note: [viewpoint ? viewpointNote : '', filtered ? 'Shares use only the selected sides.' : '',
    view.reason && view.state === 'unknown' ? AIR_MISSING_REASONS[view.reason] ?? 'The required data was not recorded.' : ''].filter(Boolean).join(' ') || undefined };
  return { ...content, ...countryNameDetails(...named) };
}

/** The compact hover keeps shares, missing-side counts and assigned wings. */
export function airSummaryTooltip(full: TooltipContent, view: AirView): TooltipContent {
  const labels = new Set(view.parts.map(part => part.label));
  return { ...full, rows: full.rows.filter(row => row.label === 'Air wings assigned to the region' || labels.has(String(row.label)) || row.swatch != null
    || row.label === 'Excluded sides' || row.label === 'Unknown sides')
      .map(row => row.swatch ? { ...row, value: String(row.value).split(' \u00b7 ')[0] } : row)
      .concat(view.state === 'known' ? [] : [{ label: 'State', value: airState(view) }]) };
}

export function validateAirReply(value: unknown): AirReply {
  const invalid = (): never => { throw new Error('Invalid air-superiority reply'); };
  const text = (item: unknown): item is string => typeof item === 'string';
  const optionalText = (item: unknown) => item === null || text(item);
  const natural = (item: unknown): item is number => int(item) && item >= 0;
  // Fewer than 19 digits are always below i64::MAX, so only 19-digit scores are parsed.
  const raw = (item: unknown) => text(item) && /^(0|[1-9][0-9]{0,18})$/.test(item) && (item.length < 19 || BigInt(item) <= 9223372036854775807n);
  if (!object(value) || !text(value.branchId) || !Array.isArray(value.sides) || !Array.isArray(value.warnings) || !value.warnings.every(text)) return invalid();
  if (value.view === 'moment') {
    const sides = value.sides.map(side => {
      if (!Array.isArray(side)) return side;
      if (side.length !== 2 || !text(side[0]) || !side[0] || !int(side[1])) return invalid();
      const [tag, hour] = side;
      return { key: `c:${tag}`, faction: null, colourFaction: null, tag, name: 'Country name unavailable', leader: tag, coBelligerents: 0,
        reads: [[hour, 'Country name unavailable', tag, tag, 0, [], null]] };
    });
    value = { ...value, sides };
  }
  if (!object(value) || !Array.isArray(value.sides)) return invalid();
  for (const side of value.sides) if (!object(side) || !text(side.key) || !text(side.name) || !optionalText(side.faction)
    || (side.faction != null && !/^[0-9]+$/.test(side.faction as string)) || !optionalText(side.colourFaction)
    || (side.colourFaction != null && !/^[0-9]+$/.test(side.colourFaction as string))
    || !optionalText(side.tag) || !optionalText(side.leader) || !natural(side.coBelligerents)) return invalid();
  for (const side of value.sides) {
    const factions = (items: unknown) => Array.isArray(items) && items.every(item => Array.isArray(item) && item.length === 2 && text(item[0]) && /^[0-9]+$/.test(item[0]) && text(item[1]));
    if (side.factions !== undefined && !factions(side.factions)) return invalid();
    if (side.reads !== undefined) {
      if (!Array.isArray(side.reads)) return invalid();
      let previous = -Infinity;
      for (const read of side.reads) {
        if (!Array.isArray(read) || read.length !== 7 || !int(read[0]) || read[0] <= previous || !text(read[1]) || !optionalText(read[2]) || !optionalText(read[3]) || !natural(read[4]) || !factions(read[5])
          || !optionalText(read[6]) || (read[6] != null && !/^[0-9]+$/.test(read[6]))) return invalid();
        previous = read[0];
      }
    }
  }
  preparedPending(value);
  if (value.approximate !== undefined && !isOrderedSpans(value.approximate)) return invalid();
  if (value.rowsApproximate !== undefined && typeof value.rowsApproximate !== 'boolean') return invalid();
  if (value.sampleHours !== undefined && (!Array.isArray(value.sampleHours) || !Array.isArray(value.days) || value.sampleHours.length !== value.days.length
    || !value.sampleHours.every(hour => hour === null || int(hour)))) return invalid();
  const sides = value.sides;
  const replyView = value.view;
  const sideIndex = (item: unknown): item is number => natural(item) && item < sides.length;
  const unknownSets = value.unknownSets;
  if (unknownSets !== undefined && (value.view !== 'moment' || !Array.isArray(unknownSets)
    || !unknownSets.every(set => Array.isArray(set) && set.length > 0 && set.every(sideIndex) && new Set(set).size === set.length))) return invalid();
  const cell = (item: unknown): boolean => {
    if (item === null || text(item)) return true;
    if (Array.isArray(item) && typeof item[0] === 'string') return item.length === 2 && item.every(raw);
    if (object(item) && 'view' in item) return Array.isArray(item.view) && item.view.length === 2 && item.view.every(raw) && sideIndex(item.side) && optionalText(item.leader);
    const scores = Array.isArray(item) ? item : object(item) ? item.scores : null;
    if (!Array.isArray(scores)) return false;
    const used = new Set<number>();
    for (const score of scores) {
      if (!Array.isArray(score) || score.length !== 2 || !sideIndex(score[0]) || !raw(score[1]) || score[1] === '0' || used.has(score[0])) return false;
      used.add(score[0]);
    }
    if (Array.isArray(item)) return true;
    if (!object(item)) return false;
    if (item.viewpoints !== undefined && (!Array.isArray(item.viewpoints) || !item.viewpoints.every(entry => Array.isArray(entry) && entry.length === 2 && sideIndex(entry[0]) && text(entry[1])))) return false;
    if (item.unknownSet !== undefined) {
      if (replyView !== 'moment' || !natural(item.unknownSet) || !Array.isArray(unknownSets) || item.unknownSet >= unknownSets.length
        || (item.unknown !== undefined && (!Array.isArray(item.unknown) || item.unknown.length))) return false;
      for (const index of unknownSets[item.unknownSet]) { if (used.has(index)) return false; used.add(index); }
    }
    for (const key of ['unknown', 'noAirObject']) if (item[key] !== undefined) {
      if (!Array.isArray(item[key])) return false;
      for (const index of item[key]) { if (!sideIndex(index) || used.has(index)) return false; used.add(index); }
    }
    return (item.filtered === undefined || typeof item.filtered === 'boolean') && (item.excluded === undefined || natural(item.excluded));
  };
  if (value.view === 'choices') {
    if (!Array.isArray(value.regions) || !value.regions.every(region => object(region) && natural(region.id) && text(region.name))
      || !Array.isArray(value.theatres) || !value.theatres.every(theatre => object(theatre) && text(theatre.id) && text(theatre.name))
      || !Array.isArray(value.countries) || !value.countries.every(text)
      || !Array.isArray(value.windows)) return invalid();
    let end: number | undefined;
    for (const window of value.windows) {
      if (!object(window) || !int(window.firstHour) || !int(window.lastHour) || window.firstHour > window.lastHour
        || (end !== undefined && window.firstHour !== end + 1)) return invalid();
      end = window.lastHour;
    }
  } else {
    if (typeof value.available !== 'boolean' || !int(value.firstHour) || !int(value.lastHour) || value.firstHour > value.lastHour
      || !natural(value.scale) || value.scale === 0 || !natural(value.dayHours) || value.dayHours === 0
      || !natural(value.badBelow) || !natural(value.goodAbove) || value.badBelow > value.goodAbove || value.goodAbove > value.scale) return invalid();
    const first = value.firstHour, last = value.lastHour;
    if (!object(value.segments)) return invalid();
    for (const segments of Object.values(value.segments)) {
      if (!Array.isArray(segments)) return invalid();
      let previous = -Infinity;
      for (const segment of segments) {
        if (!object(segment) || !int(segment.firstHour) || !int(segment.lastHour) || segment.firstHour <= previous || segment.firstHour > segment.lastHour || segment.lastHour > last) return invalid();
        previous = segment.lastHour;
      }
    }
    if (!object(value.anchorReads)) return invalid();
    const sideKeys = new Set(sides.map(side => side.key));
    for (const [key, read] of Object.entries(value.anchorReads)) {
      if (!sideKeys.has(key) || !Array.isArray(read) || read.length !== 2 || !text(read[0]) || !int(read[1])
        || read[1] > last || airSegmentAt(value.segments as Record<string, AirSpan[]>, read[0], read[1]) == null) return invalid();
    }
    const spans = (items: unknown): boolean => {
      if (!Array.isArray(items)) return false;
      let previous = first - 1;
      for (const span of items) {
        if (!object(span) || !int(span.firstHour) || !int(span.lastHour) || span.firstHour <= previous || span.firstHour > span.lastHour || span.lastHour > last) return false;
        previous = span.lastHour;
      }
      return true;
    };
    if (value.view === 'moment' && !spans(value.complete)) return invalid();
    if (!Array.isArray(value.theatreReads)) return invalid();
    let previousTheatre = first - 1;
    for (const run of value.theatreReads) {
      if (!Array.isArray(run) || run.length !== 3 || !int(run[0]) || !int(run[1]) || !int(run[2]) || run[0] <= previousTheatre || run[0] > run[1] || run[1] > last || run[2] > run[0]) return invalid();
      previousTheatre = run[1];
    }
    if (!Array.isArray(value.excluded)) return invalid();
    let previousExcluded = first - 1;
    for (const run of value.excluded) {
      if (!Array.isArray(run) || run.length !== 3 || !int(run[0]) || !int(run[1]) || run[0] <= previousExcluded || run[0] > run[1] || run[1] > last || !natural(run[2])) return invalid();
      previousExcluded = run[1];
    }
    if (value.view === 'timeline') {
      if (!Array.isArray(value.days) || value.days.length !== Math.floor((last - first) / value.dayHours) + 1 || !Array.isArray(value.rows)) return invalid();
      for (let day = 0; day < value.days.length; day++) {
        const read = value.days[day], start = first + day * value.dayHours;
        if (read !== null && (!int(read) || read < start || read > Math.min(last, start + value.dayHours - 1))) return invalid();
      }
      for (const row of value.rows) {
        if (!object(row) || !natural(row.region) || !text(row.name) || !Array.isArray(row.cells)) return invalid();
        let count = 0;
        for (const run of row.cells) {
          if (!Array.isArray(run) || run.length !== 2 || !natural(run[0]) || !run[0] || !cell(run[1])) return invalid();
          count += run[0]; if (count > value.days.length) return invalid();
        }
        if (count !== value.days.length) return invalid();
      }
    } else if (value.view === 'moment') {
      if (!spans(value.recorded) || !spans(value.wingsKnown) || !Array.isArray(value.regions)) return invalid();
      const runs = (items: unknown, valid: (item: unknown) => boolean): boolean => {
        if (!Array.isArray(items)) return false;
        let previous = first - 1;
        for (const run of items) {
          if (!Array.isArray(run) || run.length !== 3 || !int(run[0]) || !int(run[1]) || run[0] <= previous || run[0] > run[1] || run[1] > last || !valid(run[2])) return false;
          previous = run[1];
        }
        return true;
      };
      if (!runs(value.fallback, cell)) return invalid();
      for (const region of value.regions) {
        if (!object(region) || !natural(region.id) || !text(region.name) || !spans(region.active)) return invalid();
        for (const [key, valid] of [['runs', cell], ['wings', natural]] as const) {
          if (!runs(region[key], valid)) return invalid();
        }
      }
    } else return invalid();
  }
  if (value.view === 'moment' && Array.isArray(unknownSets)) {
    const reply = value as AirMoment;
    const expand = ([first, last, cell]: AirMoment['fallback'][number]): AirMoment['fallback'][number] => {
      if (!object(cell) || !('unknownSet' in cell) || typeof cell.unknownSet !== 'number') return [first, last, cell];
      const { unknownSet, ...rest } = cell;
      return [first, last, { ...rest, unknown: unknownSets[unknownSet] }];
    };
    return { ...reply, fallback: reply.fallback.map(expand), regions: reply.regions.map(region => ({ ...region, runs: region.runs.map(expand) })) };
  }
  return value as AirReply;
}

/** Windows follow the recording's blocks, including their gaps and unread tail. */
export function momentWindows(windows: readonly AirSpan[], hour: number): number[] {
  if (!windows.length) return [];
  let low = 0, high = windows.length;
  while (low < high) { const middle = Math.floor((low + high) / 2); if (windows[middle].firstHour <= hour) low = middle + 1; else high = middle; }
  const index = Math.max(0, low - 1), window = windows[index];
  const prefetch = window.firstHour + (window.lastHour - window.firstHour + 1) * 0.75;
  return hour >= prefetch && index + 1 < windows.length ? [index, index + 1] : [index];
}
