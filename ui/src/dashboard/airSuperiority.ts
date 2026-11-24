import { countryNameDetails } from '../charts/tooltip.ts';
import type { TooltipContent } from '../charts/tooltip';
import type { CountryLabel } from './countryFlagsAt';
import { formatAmount, rescale } from '../charts/valueAxis.ts';
import { liveKey } from '../charts/live.ts';
import type { LiveValues } from '../charts/live';
import { int, object } from './validation.ts';
import { countryLineColours } from './countryLineColours.ts';
import type { AirCell, AirChoicesReply, AirRegion, AirSide, AirSuperiorityReply } from '../generated/dashboard';

export type AirState = 'known' | 'unknown' | 'inactive' | 'filtered' | 'no-air-object' | 'gap';
export interface AirPart { id: string; label: string; color: string; share: number; score: number; side?: AirSide; tag?: string }
export interface AirViewpoint { name: string; tag?: string; color: string }
export interface AirView {
  state: AirState; parts: readonly AirPart[]; total: number; unknown: readonly AirSide[]; noAirObject: readonly AirSide[]; excluded: number;
  reason?: string; wings: number | null; scale: number; badBelow: number; goodAbove: number; viewpoint: AirViewpoint;
}
export interface AirLooks {
  name(side: AirSide): string; color(side: AirSide): string; viewpoint: AirViewpoint; enemy: string;
  scale: number; badBelow: number; goodAbove: number; mode: 'sides' | 'viewpoint';
}
export interface AirRegionView { region: { id: number; name: string }; view: AirView; active: boolean }
export const FILTERED_NOTE = 'No selected side has a score (others may)';

export function airPercent(score: number, total: number): string { return total > 0 ? `${Math.round(score * 100 / total)}%` : '—'; }
export const airScore = (score: number, scale: number): string => formatAmount(rescale(BigInt(Math.trunc(score)), scale, 2), 2);

export function sideOrder(sides: readonly AirSide[]): Map<string, number> {
  const factions = [...new Set(sides.flatMap(side => side.faction == null ? [] : [side.faction]))].sort((a, b) => {
    const first = BigInt(a), second = BigInt(b); return first < second ? -1 : first > second ? 1 : 0;
  });
  const ranks = new Map(factions.map((id, index) => [id, index]));
  const order = new Map<string, number>();
  for (const side of sides) if (!order.has(side.key)) order.set(side.key, side.faction == null ? factions.length + order.size : ranks.get(side.faction)!);
  return order;
}

export function airSideName(side: AirSide, country: (tag: string) => string, factions: (id: string) => string): string {
  if (side.factions.length) return side.factions.map(factions).join(' / ');
  const tag = side.tag ?? side.leader;
  const name = tag ? country(tag) : 'Coalition (name not recorded)';
  return side.key.startsWith('w:') && side.coBelligerents > 0 ? `${name} and allies` : name;
}

export function sideColourKey(side: AirSide): string {
  if (side.anchor) return `anchor:${side.anchor.tag}@${side.anchor.hour}`;
  const tag = side.tag ?? side.leader;
  return tag ? `tag:${tag}` : `side:${side.key}`;
}

export function resolveSideColours(
  sides: readonly AirSide[],
  anchorColor: (tag: string, hour: number) => [number, number, number] | null,
  tagColor: (tag: string) => [number, number, number] | null,
  viewpointTag: string | null,
  surface: string,
  fixed: boolean,
): Map<string, string> {
  const entries = new Map<string, { color: [number, number, number] }>();
  for (const side of sides) {
    if (side.colourFaction) continue;
    const id = sideColourKey(side);
    if (entries.has(id)) continue;
    const anchor = side.anchor ? anchorColor(side.anchor.tag, side.anchor.hour) : null;
    const tag = side.tag ?? side.leader;
    const color = anchor ?? (tag ? tagColor(tag) : null);
    if (color) entries.set(id, { color });
  }
  if (viewpointTag) {
    const color = tagColor(viewpointTag);
    if (color) entries.set(`country:${viewpointTag}`, { color });
  }
  return countryLineColours([...entries.keys()].sort(), entries, surface, fixed);
}

export function sideColor(
  side: AirSide,
  factionColours: ReadonlyMap<string, string>,
  fallbackColours: ReadonlyMap<string, string>,
  muted: string,
): string {
  return side.colourFaction ? factionColours.get(side.colourFaction) ?? muted : fallbackColours.get(sideColourKey(side)) ?? muted;
}

const NOT_FOLLOWED = 'Not followed';

export function cellView(cell: AirCell | null, sides: readonly AirSide[], values: LiveValues<number>, looks: AirLooks, order: ReadonlyMap<string, number>,
  wings: number | null, recorded = true): AirView {
  const base: AirView = { state: 'inactive', parts: [], total: 0, unknown: [], noAirObject: [], excluded: 0, wings, scale: looks.scale, badBelow: looks.badBelow,
    goodAbove: looks.goodAbove, viewpoint: looks.viewpoint };
  if (!recorded) return { ...base, state: 'gap' };
  if (cell === null) return base;
  if (cell.kind === 'reason') {
    const state: AirState = cell.reason === 'no-air-object' ? 'no-air-object' : cell.reason === 'outside-theatres' ? 'filtered' : 'unknown';
    return { ...base, state, reason: cell.reason };
  }
  if (cell.kind === 'view') {
    const side = cell.side === null ? undefined : sides[cell.side];
    const viewpoint = side ? { name: looks.name(side), color: looks.color(side), tag: cell.leader ?? undefined } : looks.viewpoint;
    const friendly = cell.live === null ? 0 : values.get(liveKey(cell.live, 'friendly'));
    const enemy = cell.live === null ? 0 : values.get(liveKey(cell.live, 'enemy'));
    if (friendly === undefined || enemy === undefined) return { ...base, viewpoint, state: 'unknown', reason: cell.live ? values.missing(liveKey(cell.live, 'friendly')) : NOT_FOLLOWED };
    const total = friendly + enemy;
    return { ...base, viewpoint, state: total ? 'known' : 'inactive', total, parts: [
      { id: 'friendly', label: `${viewpoint.name} (friendly)`, color: viewpoint.color, score: friendly, share: total ? friendly / total : 0, tag: viewpoint.tag, side },
      { id: 'enemy', label: `Enemies of ${viewpoint.name}`, color: looks.enemy, score: enemy, share: total ? enemy / total : 0 },
    ] };
  }
  const leaders = new Map(cell.viewpoints);
  let unread: string | undefined;
  const scored = cell.parts.flatMap(part => {
    const score = values.get(liveKey(part.live, 'friendly'));
    if (score === undefined) { unread ??= values.missing(liveKey(part.live, 'friendly')); return []; }
    return score > 0 ? [{ part, score }] : [];
  });
  const total = scored.reduce((sum, { score }) => sum + score, 0);
  const parts = scored.map(({ part, score }) => {
    const side = sides[part.side];
    return { id: side.key, label: looks.name(side), color: looks.color(side), score, share: total ? score / total : 0, side, tag: leaders.get(part.side) ?? side.leader ?? side.tag ?? part.tag };
  }).sort((a, b) => (order.get(a.id) ?? 0) - (order.get(b.id) ?? 0));
  const unknown = cell.unknown.map(index => sides[index]);
  const noAirObject = cell.noAirObject.map(index => sides[index]);
  const missingSide = looks.mode === 'viewpoint' ? unknown[0] ?? noAirObject[0] : undefined;
  const viewpoint = missingSide ? { name: looks.name(missingSide), color: looks.color(missingSide), tag: missingSide.leader ?? missingSide.tag ?? undefined } : base.viewpoint;
  return { ...base, total, parts, unknown, noAirObject, viewpoint, excluded: cell.excluded, ...(unread ? { reason: unread } : {}),
    state: unknown.length || unread ? 'unknown' : total ? 'known' : looks.mode === 'viewpoint' && noAirObject.length ? 'no-air-object' : cell.filtered ? 'filtered' : 'inactive' };
}

export function momentViews(reply: AirSuperiorityReply, values: LiveValues<number>, looks: AirLooks, choices?: readonly { id: number; name: string }[]): AirRegionView[] {
  const order = sideOrder(reply.sides);
  const wingsOf = (region: AirRegion) => region.wings === 'none' ? 0 : region.wings === 'unknown' ? null : values.get(liveKey(String(region.id), 'wings')) ?? null;
  const rows: AirRegionView[] = reply.regions.map(region => {
    const view = cellView(region.cell, reply.sides, values, looks, order, wingsOf(region));
    return { region: { id: region.id, name: regionLabel(region) }, view, active: region.active || view.total > 0 || (view.wings ?? 0) > 0 };
  });
  if (choices) {
    const present = new Set(reply.regions.map(region => region.id));
    for (const region of choices) if (!present.has(region.id)) rows.push({ region, active: false, view: cellView(reply.fallback, reply.sides, values, looks, order, 0) });
  }
  return rows;
}

export const regionLabel = (region: Pick<AirRegion, 'id' | 'name'>) => region.name ?? `Region ${region.id}`;

export function orderedRegions(rows: readonly AirRegionView[], order: 'name' | 'active' | 'contested'): AirRegionView[] {
  const lead = (view: AirView) => {
    if (view.state !== 'known' || view.total <= 0) return Infinity;
    let first = 0, second = 0;
    for (const part of view.parts) { if (part.share > first) { second = first; first = part.share; } else if (part.share > second) second = part.share; }
    return first - second;
  };
  return rows.filter(row => row.view.reason !== 'outside-theatres' && (order === 'name' || row.active || row.view.total > 0 || (row.view.wings ?? 0) > 0)).sort((a, b) => {
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
    if (friendly.score >= 9223372036854775807) return 'Air superiority';
    const denominator = BigInt.asIntN(64, BigInt(Math.trunc(view.total)));
    if (!denominator) return 'Air superiority';
    const ratio = BigInt.asIntN(64, BigInt(Math.trunc(friendly.score)) * BigInt(view.scale)) / denominator;
    return ratio > BigInt(view.goodAbove) ? 'Air superiority' : ratio < BigInt(view.badBelow) ? 'Enemy superiority' : 'Contested';
  }
  return 'Scores recorded';
}

const AIR_MISSING_REASONS: Readonly<Record<string, string>> = {
  'side-not-present': 'Side not present at this hour.',
  'score-not-recorded': 'Scores were not recorded at this hour.',
  'scores-unavailable': 'Scores were not recorded at this hour.',
  'sides-not-recorded': 'Side membership was not recorded.',
  'identity-not-recorded': 'Countries unavailable.',
  'theatres-not-recorded': 'Air groups unavailable.',
};

export function airTooltip(region: { name: string }, view: AirView, viewpoint: boolean, countryLabel: CountryLabel, filtered: boolean,
  factions: (id: string) => string): TooltipContent {
  const named: ReturnType<CountryLabel>[] = [];
  const country: CountryLabel = tag => { const label = countryLabel(tag); named.push(label); return label; };
  const parts = [...view.parts].sort((a, b) => b.score - a.score);
  const sideLabel = (side: AirSide) => airSideName(side, tag => country(tag).name, factions);
  const friendly = parts.find(part => part.id === 'friendly');
  const viewpointName = view.viewpoint.tag ? country(view.viewpoint.tag).name : friendly?.side ? sideLabel(friendly.side)
    : friendly?.label.replace(/ \(friendly\)$/, '') ?? view.viewpoint.name;
  const thresholds = `Above ${airPercent(view.goodAbove, view.scale)}: air superiority; below ${airPercent(view.badBelow, view.scale)}: enemy superiority; otherwise contested.`;
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
  ], note: [viewpoint ? thresholds : '', filtered ? 'Shares use only the selected sides.' : '',
    view.reason && view.state === 'unknown' ? AIR_MISSING_REASONS[view.reason] ?? view.reason : ''].filter(Boolean).join(' ') || undefined };
  return { ...content, ...countryNameDetails(...named) };
}

export function airSummaryTooltip(full: TooltipContent, view: AirView): TooltipContent {
  const labels = new Set(view.parts.map(part => part.label));
  return { ...full, rows: full.rows.filter(row => row.label === 'Air wings assigned to the region' || labels.has(String(row.label)) || row.swatch != null
    || row.label === 'Excluded sides' || row.label === 'Unknown sides')
      .map(row => row.swatch ? { ...row, value: String(row.value).split(' · ')[0] } : row)
      .concat(view.state === 'known' ? [] : [{ label: 'State', value: airState(view) }]) };
}

const text = (item: unknown): item is string => typeof item === 'string';
const optionalText = (item: unknown) => item === null || text(item);
const natural = (item: unknown): item is number => int(item) && item >= 0;

function validSide(side: unknown): boolean {
  return object(side) && text(side.key) && optionalText(side.faction) && optionalText(side.colourFaction) && Array.isArray(side.factions) && side.factions.every(text)
    && optionalText(side.tag) && optionalText(side.leader) && natural(side.coBelligerents)
    && (side.anchor === null || object(side.anchor) && text(side.anchor.tag) && int(side.anchor.hour));
}

function validCell(cell: unknown, sides: number): boolean {
  const index = (item: unknown) => natural(item) && item < sides;
  if (!object(cell)) return false;
  if (cell.kind === 'reason') return text(cell.reason);
  if (cell.kind === 'view') return (cell.side === null || index(cell.side)) && optionalText(cell.leader) && optionalText(cell.live);
  return cell.kind === 'sides' && Array.isArray(cell.parts) && cell.parts.every(part => object(part) && index(part.side) && text(part.tag) && text(part.live))
    && Array.isArray(cell.viewpoints) && cell.viewpoints.every(entry => Array.isArray(entry) && entry.length === 2 && index(entry[0]) && text(entry[1]))
    && Array.isArray(cell.unknown) && cell.unknown.every(index) && Array.isArray(cell.noAirObject) && cell.noAirObject.every(index)
    && typeof cell.filtered === 'boolean' && natural(cell.excluded);
}

export function validateAirSuperiority(value: unknown): AirSuperiorityReply {
  const invalid = (): never => { throw new Error('Invalid air-superiority reply'); };
  if (!object(value) || !optionalText(value.missing) || !Array.isArray(value.sides) || !value.sides.every(validSide) || !Array.isArray(value.regions)
    || !Array.isArray(value.warnings) || !value.warnings.every(text) || !natural(value.scale) || value.scale === 0 || !natural(value.badBelow) || !natural(value.goodAbove)
    || value.badBelow > value.goodAbove || value.goodAbove > value.scale) return invalid();
  const sides = value.sides.length;
  if (!(value.viewpoint === null || natural(value.viewpoint) && value.viewpoint < sides) || !validCell(value.fallback, sides)) return invalid();
  for (const region of value.regions) {
    if (!object(region) || !natural(region.id) || !optionalText(region.name) || !optionalText(region.nameMissing) || typeof region.active !== 'boolean'
      || !['live', 'none', 'unknown'].includes(region.wings as string) || !validCell(region.cell, sides)) return invalid();
  }
  return value as AirSuperiorityReply;
}

export function validateAirChoices(value: unknown): AirChoicesReply {
  const choice = (item: unknown) => object(item) && text(item.id) && text(item.name);
  if (!object(value) || !Array.isArray(value.regions) || !value.regions.every(choice) || !Array.isArray(value.theatres) || !value.theatres.every(choice)
    || !Array.isArray(value.sides) || !value.sides.every(validSide) || !optionalText(value.theatresMissing) || !Array.isArray(value.countries)
    || !value.countries.every(text) || !Array.isArray(value.warnings) || !value.warnings.every(text)) throw new Error('Invalid air-superiority-choices reply');
  return value as AirChoicesReply;
}

export const momentRows = (reply: AirSuperiorityReply) => JSON.stringify([reply.regions.map(region => [region.id, region.name, region.nameMissing]),
  reply.sides, reply.viewpoint, reply.fallback.kind, reply.missing, reply.warnings, reply.scale, reply.badBelow, reply.goodAbove]);
