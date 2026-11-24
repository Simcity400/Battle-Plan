import { CHART_MEASURE_COLOURS, CONVOY_EFFICIENCY_STOPS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import { dayHistoryNames, dayTileCount, dayTileEntries } from '../../families/heat/dayTiles.ts';
import type { HeatLanesWidgetDeclaration } from '../../families/heat/lanesDeclaration.ts';
import {
  spanIndexAt,
  type HeatLaneCell,
  type HeatLaneReadout,
  type HeatLaneRow,
  type HeatLaneRun,
  type HeatLanesModel,
  type HeatLanesProjection,
} from '../../families/heat/lanesModel.ts';
import type { FamilyTooltipPayload } from '../../families/host/tooltips.ts';
import { readableOn, regionLabel } from '../../families/heat/scale.ts';
import { readRegionChoice, regionParams } from '../../families/regions/regionChoice.ts';
import { countrySubject, regionLanes } from '../../families/regions/regionRows.ts';
import { formatProgress } from '../../families/valueFormat.ts';
import type { ConvoyCellRun, ConvoyCellState, ConvoyCountry, ConvoyHealthReply, ConvoyView, HourSpan } from '../../generated/dashboard';
import { mix } from '../../shell/appearance/color.ts';

export const CONVOY_TOP_DEFAULT = 7;

export interface ConvoyProjectionOptions {
  readonly view: ConvoyView;
  readonly surface: string;
  readonly country: string | null;
  readonly tiles: number;
}

export function convoyView(options: Readonly<Record<string, unknown>>): ConvoyView {
  return options.view === 'trade' ? 'trade' : 'regions';
}

export function efficiencyColour(share: number): string {
  const stops = CONVOY_EFFICIENCY_STOPS;
  const value = Math.max(0, Math.min(1, share));
  for (let i = 1; i < stops.length; i++) {
    const [upper, upperColour] = stops[i - 1]!;
    const [lower, lowerColour] = stops[i]!;
    if (value >= lower) return mix(lowerColour, upperColour, upper === lower ? 1 : (value - lower) / (upper - lower));
  }
  return stops[stops.length - 1]![1];
}

export function percentText(value: number, scale: number): string {
  return formatProgress(value / scale);
}

function stateCell(state: ConvoyCellState, scale: number): HeatLaneCell {
  if (state.kind === 'notRecorded') return { kind: 'missing', reason: state.reason };
  if (state.kind === 'noTraffic') return { kind: 'clear', label: 'No convoy traffic' };
  return { kind: 'solid', color: efficiencyColour(Number(state.value) / scale) };
}

function stateReadout(state: ConvoyCellState | undefined, scale: number, surface: string): HeatLaneReadout | undefined {
  if (!state) return undefined;
  if (state.kind === 'notRecorded') return { kind: 'missing', reason: state.reason };
  if (state.kind === 'noTraffic') return { kind: 'clear', label: 'No convoy traffic' };
  return efficiencyReadout(Number(state.value), scale, surface);
}

function efficiencyReadout(value: number, scale: number, surface: string): HeatLaneReadout {
  return {
    kind: 'value',
    form: 'value',
    primary: percentText(value, scale),
    meaning: 'Efficiency',
    ...(value >= scale ? { tone: 'muted' as const } : {}),
    swatch: readableOn(efficiencyColour(value / scale), surface, 1),
  };
}

type Dated = readonly (readonly [number, number, number])[];

function valueAt(runs: Dated, hour: number): number | null {
  const run = runs[spanIndexAt(runs, hour, item => item[0])];
  return run && hour <= run[1] ? run[2] : null;
}

function inSpans(spans: readonly HourSpan[], hour: number): boolean {
  const span = spans[spanIndexAt(spans, hour, item => item.firstHour)];
  return !!span && hour <= span.lastHour;
}

function countryAt(runs: readonly ConvoyCountry[], hour: number): string {
  const found = runs[spanIndexAt(runs, hour, run => run.firstHour)];
  if (!found) return 'Country not recorded';
  return found.name ?? (found.nameMissing ? 'Name not recorded' : found.tag ?? 'Country not recorded');
}

function stateTooltip(title: string, hour: number, state: ConvoyCellState, scale: number): FamilyTooltipPayload {
  if (state.kind === 'notRecorded') return { title, hour, missingReason: state.reason };
  if (state.kind === 'noTraffic') return { title, hour, items: [{ label: 'No convoy traffic' }] };
  const value = Number(state.value);
  return { title, hour, items: [{ label: 'Efficiency', value: percentText(value, scale), swatch: efficiencyColour(value / scale) }] };
}

interface Piece<T> {
  readonly reply: ConvoyHealthReply;
  readonly end: number;
  readonly item: T | undefined;
}

function pieceAt<T>(pieces: readonly Piece<T>[], hour: number): Piece<T> | null {
  const piece = pieces[spanIndexAt(pieces, hour, item => item.reply.firstHour)];
  return piece && hour <= piece.end ? piece : null;
}

function extended(runs: readonly HeatLaneRun[], lastHour: number, through: number): readonly HeatLaneRun[] {
  const last = runs.at(-1);
  if (!last || through <= lastHour || last.lastHour !== lastHour) return runs;
  return [...runs.slice(0, -1), { ...last, lastHour: through }];
}

function cellRuns(cells: readonly ConvoyCellRun[], through: number, scale: number): HeatLaneRun[] {
  const runs: HeatLaneRun[] = [];
  for (const run of cells) {
    if (run.firstHour > through) break;
    const lastHour = Math.min(run.lastHour, through);
    runs.push({ firstHour: run.firstHour, lastHour, readHour: lastHour, cell: stateCell(run.state, scale) });
  }
  return runs;
}

interface RegionLane {
  readonly runs: readonly HeatLaneRun[];
  readonly cells: readonly ConvoyCellRun[];
}

interface TileRegions {
  readonly byRegion: ReadonlyMap<number, RegionLane>;
  readonly absent: RegionLane;
}

const heldRegions = new WeakMap<ConvoyHealthReply, TileRegions>();

function tileRegions(tile: ConvoyHealthReply): TileRegions {
  const held = heldRegions.get(tile);
  if (held) return held;
  const lane = (cells: readonly ConvoyCellRun[]): RegionLane => ({ runs: cellRuns(cells, tile.lastHour, tile.scale), cells });
  const made = { byRegion: new Map(tile.regions.map(region => [region.region, lane(region.cells)])), absent: lane(tile.absent ?? []) };
  heldRegions.set(tile, made);
  return made;
}

interface ConvoyRow {
  readonly row: HeatLaneRow;
  readonly inspect: (hour: number) => FamilyTooltipPayload | null;
}

function regionRows(readView: BoundedReadView, tiles: readonly ConvoyHealthReply[], now: ConvoyHealthReply, through: number, surface: string): ConvoyRow[] {
  const held = tiles.map(tileRegions);
  return now.regions.map(region => {
    const { label, nameMissing } = regionLabel(readView, region.region, region.name);
    const pieces: Piece<RegionLane>[] = [
      ...tiles.map((tile, k) => ({ reply: tile, end: tile.lastHour, item: held[k]!.byRegion.get(region.region) ?? held[k]!.absent })),
      { reply: now, end: through, item: { runs: extended(cellRuns(region.cells, now.lastHour, now.scale), now.lastHour, through), cells: region.cells } },
    ];
    const readout = stateReadout(region.readout, now.scale, surface);
    return {
      row: {
        id: `r:${region.region}`,
        label,
        ...(nameMissing ? { nameMissing } : {}),
        pinned: region.pinned,
        region: region.region,
        runs: pieces.flatMap(piece => piece.item!.runs),
        ...(readout ? { readout } : {}),
      },
      inspect: hour => {
        const piece = pieceAt(pieces, hour);
        if (!piece) return null;
        const at = Math.min(hour, piece.reply.lastHour);
        const cells = piece.item!.cells;
        const run = cells[spanIndexAt(cells, at, item => item.firstHour)];
        return run && at <= run.lastHour ? stateTooltip(label, hour, run.state, piece.reply.scale) : null;
      },
    };
  });
}

function seriesRuns(
  values: Dated,
  known: readonly HourSpan[],
  through: number,
  cellOf: (value: number) => HeatLaneCell,
  unknownReason: string,
): HeatLaneRun[] {
  const out: HeatLaneRun[] = [];
  const cuts = new Set<number>();
  for (const [first, last] of values) { cuts.add(first); cuts.add(last + 1); }
  for (const span of known) { cuts.add(span.firstHour); cuts.add(span.lastHour + 1); }
  const points = [...cuts].filter(hour => hour <= through).sort((a, b) => a - b);
  for (let i = 0; i < points.length; i++) {
    const start = points[i]!;
    const end = Math.min(through, (points[i + 1] ?? through + 1) - 1);
    const value = valueAt(values, start);
    const cell: HeatLaneCell | null = value !== null
      ? cellOf(value)
      : inSpans(known, start) ? null : { kind: 'missing', reason: unknownReason };
    if (!cell) continue;
    out.push({ firstHour: start, lastHour: end, readHour: end, cell });
  }
  return out;
}

function seriesTooltip(title: string, hour: number, item: { readonly label: string; readonly value?: string; readonly swatch?: string } | null, known: readonly HourSpan[], none: string, unknownReason: string): FamilyTooltipPayload {
  if (item) return { title, hour, items: [item] };
  return inSpans(known, hour) ? { title, hour, items: [{ label: none }] } : { title, hour, missingReason: unknownReason };
}

function routeCell(bad: number, routeBad: string): HeatLaneCell {
  return bad ? { kind: 'solid', color: routeBad, label: 'Route bad' } : { kind: 'clear', label: 'Route clear' };
}

type Deal = ConvoyHealthReply['deals'][number];
type Lease = ConvoyHealthReply['leases'][number];

const DEALS_UNKNOWN = 'Resource deals were not recorded';
const CLIENTS_UNKNOWN = 'Convoy clients were not recorded';

interface TileTrade {
  readonly deals: ReadonlyMap<string, readonly HeatLaneRun[]>;
  readonly leases: ReadonlyMap<string, readonly HeatLaneRun[]>;
}

const heldTrade = new WeakMap<ConvoyHealthReply, TileTrade>();

function dealRuns(deal: Deal, reply: ConvoyHealthReply, through: number): HeatLaneRun[] {
  return seriesRuns(deal.efficiency, reply.known[1] ?? [], through, value => ({ kind: 'solid', color: efficiencyColour(value / reply.scale) }), DEALS_UNKNOWN);
}

function leaseRuns(lease: Lease, reply: ConvoyHealthReply, through: number, routeBad: string): HeatLaneRun[] {
  return seriesRuns(lease.bad, reply.known[2] ?? [], through, bad => routeCell(bad, routeBad), CLIENTS_UNKNOWN);
}

function tileTrade(tile: ConvoyHealthReply, routeBad: string): TileTrade {
  const held = heldTrade.get(tile);
  if (held) return held;
  const made = {
    deals: new Map(tile.deals.map(deal => [deal.id, dealRuns(deal, tile, tile.lastHour)])),
    leases: new Map(tile.leases.map(lease => [lease.handle, leaseRuns(lease, tile, tile.lastHour, routeBad)])),
  };
  heldTrade.set(tile, made);
  return made;
}

function latest<T>(pieces: readonly Piece<T>[], has: (item: T) => boolean): Piece<T> | null {
  for (let k = pieces.length - 1; k >= 0; k--) {
    const piece = pieces[k]!;
    if (piece.item && has(piece.item)) return piece;
  }
  return null;
}

function dealName(deal: Deal, reply: ConvoyHealthReply, hour: number): string {
  const resource = deal.resource[spanIndexAt(deal.resource, hour, run => run[0])]?.[2] ?? null;
  const named = resource === null ? undefined : reply.resources.find(item => item.id === resource);
  return `${countryAt(deal.exporter, hour)} · ${resource === null ? 'Resource not recorded' : named?.name ?? 'Resource name not recorded'}`;
}

function tradeRows(tiles: readonly ConvoyHealthReply[], now: ConvoyHealthReply, through: number, surface: string): ConvoyRow[] {
  const routeBad = readableOn(CHART_MEASURE_COLOURS.short, surface);
  const held = tiles.map(tile => tileTrade(tile, routeBad));
  const tileDeals = tiles.map(tile => new Map(tile.deals.map(deal => [deal.id, deal])));
  const tileLeases = tiles.map(tile => new Map(tile.leases.map(lease => [lease.handle, lease])));
  const asked = now.lastHour;
  const rows: ConvoyRow[] = [];
  for (const deal of now.deals) {
    const pieces: Piece<Deal>[] = [...tiles.map((tile, k) => ({ reply: tile, end: tile.lastHour, item: tileDeals[k]!.get(deal.id) })), { reply: now, end: through, item: deal }];
    const runs = [...tiles.flatMap((_, k) => held[k]!.deals.get(deal.id) ?? []), ...extended(dealRuns(deal, now, asked), asked, through)];
    if (!runs.length) continue;
    const named = latest(pieces, item => item.exporter.length > 0 || item.resource.length > 0) ?? pieces.at(-1)!;
    const label = dealName(named.item!, named.reply, Math.min(asked, named.reply.lastHour));
    const value = valueAt(deal.efficiency, asked);
    const readout: HeatLaneReadout = value !== null
      ? efficiencyReadout(value, now.scale, surface)
      : inSpans(now.known[1] ?? [], asked) ? { kind: 'clear', label: 'No deal' } : { kind: 'missing', reason: DEALS_UNKNOWN };
    rows.push({
      row: { id: `deal:${deal.id}`, label, runs, readout },
      inspect: hour => {
        const piece = pieceAt(pieces, hour);
        if (!piece) return null;
        const at = Math.min(hour, piece.reply.lastHour);
        const item = piece.item;
        const value = item ? valueAt(item.efficiency, at) : null;
        const shown = value === null ? null : { label: 'Efficiency', value: percentText(value, piece.reply.scale), swatch: efficiencyColour(value / piece.reply.scale) };
        return seriesTooltip(item ? dealName(item, piece.reply, at) : label, hour, shown, piece.reply.known[1] ?? [], 'No deal', DEALS_UNKNOWN);
      },
    });
  }
  for (const lease of now.leases) {
    const pieces: Piece<Lease>[] = [...tiles.map((tile, k) => ({ reply: tile, end: tile.lastHour, item: tileLeases[k]!.get(lease.handle) })), { reply: now, end: through, item: lease }];
    const runs = [...tiles.flatMap((_, k) => held[k]!.leases.get(lease.handle) ?? []), ...extended(leaseRuns(lease, now, asked, routeBad), asked, through)];
    if (!runs.length) continue;
    const named = latest(pieces, item => item.giver.length > 0) ?? pieces.at(-1)!;
    const giver = countryAt(named.item!.giver, Math.min(asked, named.reply.lastHour));
    const bad = valueAt(lease.bad, asked);
    const readout: HeatLaneReadout = bad === null
      ? inSpans(now.known[2] ?? [], asked) ? { kind: 'clear', label: 'No route' } : { kind: 'missing', reason: CLIENTS_UNKNOWN }
      : { kind: 'value', form: 'word', primary: bad ? 'Route bad' : 'Route clear', swatch: bad ? routeBad : 'hollow' };
    rows.push({
      row: { id: `lease:${lease.handle}`, label: `Lend-lease from ${giver}`, runs, readout },
      inspect: hour => {
        const piece = pieceAt(pieces, hour);
        if (!piece) return null;
        const at = Math.min(hour, piece.reply.lastHour);
        const item = piece.item;
        const bad = item ? valueAt(item.bad, at) : null;
        const shown = bad === null ? null : bad ? { label: 'Route bad', swatch: routeBad } : { label: 'Route clear' };
        return seriesTooltip(`Lend-lease from ${item ? countryAt(item.giver, at) : giver}`, hour, shown, piece.reply.known[2] ?? [], 'No route', CLIENTS_UNKNOWN);
      },
    });
  }
  return rows;
}

function emptyModel(hour: number, extra: Partial<HeatLanesModel>): HeatLanesModel {
  return { hour, firstHour: hour, lastHour: hour, dayHours: 24, playhead: null, rows: [], ...extra };
}

export function projectConvoyHealth(readView: BoundedReadView, options: ConvoyProjectionOptions): HeatLanesModel {
  const hour: number = readView.hour;
  const refused = readView.refused('now') ?? dayHistoryNames(options.tiles).map(name => readView.refused(name)).find(Boolean);
  if (refused) return emptyModel(hour, { refused });
  const now = readView.panel<ConvoyHealthReply>('now');
  if (!now || !options.surface || now.lastHour > hour || now.view !== options.view) return emptyModel(hour, {});
  const tiles: ConvoyHealthReply[] = [];
  for (const name of dayHistoryNames(options.tiles)) {
    const tile = readView.panel<ConvoyHealthReply>(name);
    if (tile && tile.view === now.view && tile.lastHour < now.firstHour) tiles.push(tile);
  }
  const through = Math.min(hour, now.validThrough);
  const built = now.view === 'regions' ? regionRows(readView, tiles, now, through, options.surface) : tradeRows(tiles, now, through, options.surface);
  const rows = built.map(item => item.row);
  const subject = countrySubject(readView, options.country);
  const gaps = new Map<string, { firstHour: number; lastHour: number; reason: string }>();
  for (const reply of [...tiles, now]) for (const gap of reply.gaps) gaps.set(`${gap.firstHour}:${gap.lastHour}`, { firstHour: gap.firstHour, lastHour: gap.lastHour, reason: 'The recording holds no hour here' });
  return {
    hour: through,
    firstHour: tiles[0]?.firstHour ?? now.firstHour,
    lastHour: through,
    dayHours: 24,
    playhead: through,
    gaps: [...gaps.values()],
    rows,
    ...(now.view === 'regions' ? { regions: regionLanes(subject) } : {}),
    inspect: (rowIndex, at) => built[rowIndex]?.inspect(at) ?? null,
    ...(rows.length ? {} : now.rankMissing ? { empty: now.rankMissing, emptyKind: 'unavailable' as const } : { empty: `${now.view === 'regions' ? 'No convoys' : 'No convoyed trade'} for ${subject ?? 'this country'} at this date.` }),
    warnings: [...new Set([...tiles, now].flatMap(reply => reply.warnings).concat(rows.length && now.rankMissing ? [now.rankMissing] : []))],
  };
}

function nextConvoyChange(readView: BoundedReadView): number {
  const now = readView.panel<ConvoyHealthReply>('now');
  return now && readView.hour < now.validThrough ? readView.hour + 1 : Infinity;
}

export const convoyHealthProjection: HeatLanesProjection<ConvoyProjectionOptions> = {
  kind: 'convoy-health',
  family: 'heat-lanes',
  project: projectConvoyHealth,
  nextChange: nextConvoyChange,
};

export const convoyHealthDeclaration: HeatLanesWidgetDeclaration<ConvoyProjectionOptions> = {
  family: 'heat-lanes',
  kind: 'convoy-health',
  label: 'Convoy health',
  subject: 'convoys',
  emptyCampaign: 'Load a campaign to see convoys.',
  icon: 'anchor',
  countries: { mode: 'single' },
  unavailable: ctx => (ctx.countryTag ? null : 'Choose a country in widget settings.'),
  sources: (ctx): SourcePlan => {
    const view = convoyView(ctx.options);
    if (!ctx.countryTag) return { owner: `convoy-health:${ctx.branch.id}:none`, entries: [], unavailable: 'Choose a country in widget settings.' };
    const top = typeof ctx.options.rowCount === 'number' ? ctx.options.rowCount : CONVOY_TOP_DEFAULT;
    const regions: Record<string, unknown> = view === 'regions' ? regionParams(readRegionChoice(ctx.options.regions)) : {};
    const params = { branchId: ctx.branch.id, view, country: countryFromKey(ctx.countryTag), from: null, to: null };
    const shown = view === 'regions' ? { ...params, ...regions, top } : params;
    const { pinnedRegions: _pinned, hiddenRegions: _hidden, ...rankMissing } = regions;
    return {
      owner: `convoy-health:${JSON.stringify(shown)}`,
      entries: dayTileEntries(ctx, {
        product: 'convoy-health',
        now: {
          params: shown,
          coverage: view === 'regions' ? { mode: 'population', form: 'top', limit: top } : { mode: 'population', form: 'all' },
          span: (base, first, last, spanFirst) => ({ ...base, from: first, to: last, ...(view === 'trade' ? { listFrom: spanFirst } : {}) }),
          validThrough: data => (data as ConvoyHealthReply | undefined)?.validThrough ?? null,
        },
        tile: {
          params: view === 'regions' ? { ...params, ...rankMissing, every: true } : params,
          coverage: { mode: 'population', form: 'all' },
          span: (base, first, last) => ({ ...base, from: first, to: last }),
        },
      }),
    };
  },
  projection: convoyHealthProjection,
  projectionOptions: (ctx, extra) => ({ view: convoyView(ctx.options), surface: extra.surface, country: ctx.countryTag, tiles: dayTileCount(ctx.branch) }),
  jumpToDate: true,
};
