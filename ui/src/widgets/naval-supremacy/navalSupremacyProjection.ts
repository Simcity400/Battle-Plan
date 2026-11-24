import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BarsModel, BarsProjection, BarsSegment } from '../../families/bars/model.ts';
import { buildShareRowsModel, type ShareRowEntry } from '../../families/bars/shareRows.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import { dayHistoryNames, dayTileCount, dayTileEntries } from '../../families/heat/dayTiles.ts';
import type { HeatLanesWidgetDeclaration } from '../../families/heat/lanesDeclaration.ts';
import { findRunAtHour, spanIndexAt, type HeatLaneCell, type HeatLaneReadout, type HeatLaneRow, type HeatLaneRun, type HeatLanesModel, type HeatLanesProjection } from '../../families/heat/lanesModel.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import { contestedGrey, countryColourAt, readableOn, regionLabel, type DatedColour } from '../../families/heat/scale.ts';
import { formatDateOnly, formatShare } from '../../families/valueFormat.ts';
import { readRegionChoice, regionParams } from '../../families/regions/regionChoice.ts';
import { ADD_REGION_LABEL, mostActiveHeading, PICKED_REGIONS_HEADING } from '../../families/regions/regionRows.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import type { MissionShips, NavalSupremacyReply, SupremacyRegion, SupremacyRun } from '../../generated/dashboard';

export const SUPREMACY_TOP_DEFAULT = 7;

export interface SupremacyOptions {
  readonly viewer: string | null;
  readonly surface: string;
  readonly tiles: number;
}

interface Look {
  readonly friendly: (hour: number) => DatedColour;
  readonly enemy: string;
  readonly contested: string;
  readonly friendlyLabel: string;
  readonly enemyLabel: string;
  readonly viewer: string;
}

function lookOf(readView: BoundedReadView, reply: NavalSupremacyReply, options: SupremacyOptions): Look {
  const { surface, viewer } = options;
  const noColour = readableOn(CHART_MEASURE_COLOURS.bopRight, surface);
  const friendly = viewer
    ? (hour: number) => countryColourAt(readView, viewer, hour, surface, noColour)
    : () => ({ colour: noColour, through: Infinity });
  return {
    friendly,
    enemy: readableOn(CHART_MEASURE_COLOURS.short, surface),
    contested: contestedGrey(surface, [friendly(reply.lastHour).colour]),
    friendlyLabel: 'Friendly control',
    enemyLabel: 'Enemy control',
    viewer: reply.viewerName ?? 'Viewer',
  };
}

function shareOf(text: string | null, scale: number): number | null {
  if (text === null) return null;
  const value = Number(text) / scale;
  return Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : null;
}

export function shipsText(own: MissionShips, enemy: MissionShips): string {
  if (own.kind === 'counted' && enemy.kind === 'counted') return `${own.ships} v ${enemy.ships} ships`;
  if (own.kind === 'counted') return `${own.ships} v n/a ships`;
  if (enemy.kind === 'counted') return `n/a v ${enemy.ships} ships`;
  return 'Ships not recorded';
}

function shipItems(region: SupremacyRegion, look: Look) {
  const item = (label: string, ships: MissionShips) => ships.kind === 'counted'
    ? { label, value: `${ships.ships}` }
    : { label, missingReason: ships.reason };
  return [item(`Mission-registered ships, ${look.viewer} and friends`, region.ownShips), item(`Mission-registered ships, enemies of ${look.viewer}`, region.enemyShips)];
}

function emptyBars(hour: number): BarsModel {
  return buildShareRowsModel({ hour });
}

export function projectSupremacyShares(readView: BoundedReadView, options: SupremacyOptions): BarsModel {
  const hour: number = readView.hour;
  const refused = readView.refused('now');
  if (refused) return buildShareRowsModel({ hour, empty: refused, emptyKind: 'unavailable' });
  const reply = readView.panel<NavalSupremacyReply>('now');
  if (!reply || !options.surface || reply.lastHour > hour) return emptyBars(hour);
  const look = lookOf(readView, reply, options);
  const entry = (region: SupremacyRegion): ShareRowEntry => {
    const named = regionLabel(readView, region.id, region.name);
    const base = { key: region.id, subject: region.id, region: Number(region.id), label: named.label, ...(named.nameMissing ? { labelMissing: true } : {}), countText: shipsText(region.ownShips, region.enemyShips) };
    const readout = region.readout;
    if (!readout || readout.kind === 'notRecorded') {
      const reason = readout?.reason ?? 'Naval region control was not recorded';
      return { ...base, segments: [], missing: notRecorded(reason), tooltip: { title: named.label, hour, missingReason: reason, items: shipItems(region, look) } };
    }
    if (idle(readout.friendly, readout.enemy)) {
      return { ...base, segments: [], tooltip: { title: named.label, hour: readout.as_of_hour, items: [{ label: 'No activity' }, ...shipItems(region, look)] } };
    }
    const share = shareOf(readout.share, reply.scale);
    if (share === null) {
      return { ...base, segments: [], missing: notRecorded('The supremacy share was not recorded'), tooltip: { title: named.label, hour: readout.as_of_hour, missingReason: 'The supremacy share was not recorded' } };
    }
    const friendlyPct = formatShare(share);
    const enemyPct = formatShare(1 - share);
    const friendly = look.friendly(readout.as_of_hour).colour;
    const segments: BarsSegment[] = [
      { key: 'friendly', label: `${look.viewer} and friends`, share, value: knownValue(share), color: friendly, inlineLabel: `Friendly ${friendlyPct}` },
      { key: 'enemy', label: `Enemies of ${look.viewer}`, share: 1 - share, value: knownValue(1 - share), color: look.enemy, inlineLabel: `Enemy ${enemyPct}` },
    ];
    const control = readout.control > 0 ? look.friendlyLabel : readout.control < 0 ? look.enemyLabel : 'Contested';
    return {
      ...base,
      segments,
      tooltip: {
        title: named.label,
        hour: readout.as_of_hour,
        items: [
          { label: 'Control', value: control },
          { label: `${look.viewer} and friends`, value: friendlyPct, swatch: friendly },
          { label: `Enemies of ${look.viewer}`, value: enemyPct, swatch: look.enemy },
          ...shipItems(region, look),
        ],
      },
    };
  };
  const picked = reply.regions.filter(region => region.pinned).map(entry);
  const automatic = reply.regions.filter(region => !region.pinned).map(entry);
  return buildShareRowsModel({
    hour,
    picked,
    automatic,
    pickedHeading: PICKED_REGIONS_HEADING,
    automaticHeading: mostActiveHeading(look.viewer),
    addRegionLabel: ADD_REGION_LABEL,
    ...(reply.regions.length ? {} : reply.rankMissing ? { empty: reply.rankMissing, emptyKind: 'unavailable' as const } : { empty: `No naval activity for ${look.viewer} at this date.` }),
    warnings: reply.regions.length && reply.rankMissing ? [...reply.warnings, reply.rankMissing] : reply.warnings,
  });
}

function idle(friendly: string | null, enemy: string | null): boolean {
  return friendly !== null && enemy !== null && Number(friendly) <= 0 && Number(enemy) <= 0;
}

function runCell(run: SupremacyRun, look: Look, scale: number, friendly: string): { cell: HeatLaneCell; items: { label: string; value?: string; swatch?: string }[] } {
  if (run.control === null) return { cell: { kind: 'missing', reason: 'Naval region control was not recorded' }, items: [] };
  if (idle(run.friendly, run.enemy)) return { cell: { kind: 'clear', label: 'No activity' }, items: [{ label: 'No activity' }] };
  const share = shareOf(run.share, scale);
  const pct = share === null ? null : formatShare(share);
  const color = run.control > 0 ? friendly : run.control < 0 ? look.enemy : look.contested;
  const label = run.control > 0 ? look.friendlyLabel : run.control < 0 ? look.enemyLabel : 'Contested';
  return {
    cell: { kind: 'solid', color },
    items: [{ label, swatch: color, ...(pct === null ? {} : { value: `${pct} of the score` }) }],
  };
}

function emptyLanes(hour: number, extra: Partial<HeatLanesModel>): HeatLanesModel {
  return { hour, firstHour: hour, lastHour: hour, dayHours: 24, playhead: null, rows: [], ...extra };
}

function unknownRunsOf(reply: NavalSupremacyReply, through: number): HeatLaneRun[] {
  const dayStart = (day: number) => reply.firstDayHour + day * reply.dayHours;
  return reply.unknown.map(span => {
    const firstHour = Math.max(reply.firstHour, dayStart(span.firstDay));
    const lastHour = Math.min(through, dayStart(span.lastDay + 1) - 1);
    return { firstHour, lastHour, readHour: lastHour, cell: { kind: 'missing', reason: span.reason } };
  });
}

function regionRuns(reply: NavalSupremacyReply, region: SupremacyRegion, look: Look, through: number, unknownRuns: readonly HeatLaneRun[]): HeatLaneRun[] {
  const dayStart = (day: number) => reply.firstDayHour + day * reply.dayHours;
  const runs: HeatLaneRun[] = [];
  for (const run of region.runs) {
    for (let day = run.firstDay; day <= run.lastDay;) {
      const firstHour = Math.max(reply.firstHour, dayStart(day));
      if (firstHour > through) break;
      const friendly = look.friendly(firstHour);
      const until = Math.max(day, Math.min(run.lastDay, Math.floor((friendly.through - reply.firstDayHour) / reply.dayHours)));
      const lastHour = Math.min(through, dayStart(until + 1) - 1);
      runs.push({ firstHour, lastHour, readHour: lastHour, cell: runCell(run, look, reply.scale, friendly.colour).cell });
      day = until + 1;
    }
  }
  runs.push(...unknownRuns);
  runs.sort((a, b) => a.firstHour - b.firstHour);
  return runs;
}

interface TileLanes {
  readonly sig: string;
  readonly byRegion: ReadonlyMap<string, readonly HeatLaneRun[]>;
  readonly unknown: readonly HeatLaneRun[];
}

const heldTiles = new WeakMap<NavalSupremacyReply, TileLanes>();

function tileLanes(tile: NavalSupremacyReply, sig: string, look: Look): TileLanes {
  const held = heldTiles.get(tile);
  if (held && held.sig === sig) return held;
  const unknown = unknownRunsOf(tile, tile.lastHour);
  const made = { sig, unknown, byRegion: new Map(tile.regions.map(region => [region.id, regionRuns(tile, region, look, tile.lastHour, unknown)])) };
  heldTiles.set(tile, made);
  return made;
}

function datedSignature(look: Look, first: number, last: number): string {
  const out: string[] = [];
  for (let at = first; at <= last;) {
    const dated = look.friendly(at);
    out.push(dated.colour);
    if (dated.through >= last) break;
    at = dated.through + 1;
  }
  return out.join(',');
}

function inspectDay(reply: NavalSupremacyReply, region: SupremacyRegion | undefined, title: string, at: number, look: Look, painted: HeatLaneCell | undefined, readoutDay: boolean) {
  const dayStart = (day: number) => reply.firstDayHour + day * reply.dayHours;
  const day = Math.floor((at - reply.firstDayHour) / reply.dayHours);
  const readout = region?.readout;
  const dated = readoutDay && readout?.kind === 'control' ? { hour: readout.as_of_hour } : { subtitle: formatDateOnly(dayStart(day)) };
  const unknown = reply.unknown[spanIndexAt(reply.unknown, day, span => span.firstDay)];
  if (unknown && day <= unknown.lastDay) return { title, ...dated, missingReason: unknown.reason };
  const run = region?.runs[spanIndexAt(region.runs, day, item => item.firstDay)];
  if (!run || day > run.lastDay) return { title, ...dated, missingReason: 'Naval region control was not recorded' };
  const made = runCell(run, look, reply.scale, painted?.kind === 'solid' ? painted.color : look.contested);
  return made.cell.kind === 'missing' ? { title, ...dated, missingReason: made.cell.reason } : { title, ...dated, items: made.items };
}

export function projectSupremacyTimeline(readView: BoundedReadView, options: SupremacyOptions): HeatLanesModel {
  const hour: number = readView.hour;
  const refused = refusedOf(readView, options.tiles);
  if (refused) return emptyLanes(hour, { refused });
  const now = readView.panel<NavalSupremacyReply>('now');
  if (!now || !options.surface || now.lastHour > hour) return emptyLanes(hour, {});
  const tiles: NavalSupremacyReply[] = [];
  for (const name of dayHistoryNames(options.tiles)) {
    const tile = readView.panel<NavalSupremacyReply>(name);
    if (tile && tile.lastHour < now.firstHour) tiles.push(tile);
  }
  const look = lookOf(readView, now, options);
  const through = Math.min(hour, now.validThrough);
  const held = tiles.map(tile => tileLanes(tile, `${options.surface}|${look.contested}|${datedSignature(look, tile.firstHour, tile.lastHour)}`, look));
  const nowUnknown = unknownRunsOf(now, through);
  const lastDay = Math.floor((through - now.firstDayHour) / now.dayHours);
  const rows: HeatLaneRow[] = now.regions.map(region => {
    const named = regionLabel(readView, region.id, region.name);
    const runs = [...held.flatMap(lanes => lanes.byRegion.get(region.id) ?? lanes.unknown), ...regionRuns(now, region, look, through, nowUnknown)];
    const readout = region.readout;
    let shown: HeatLaneReadout | undefined;
    if (readout?.kind === 'notRecorded') shown = { kind: 'missing', reason: readout.reason };
    else if (readout && idle(readout.friendly, readout.enemy)) shown = { kind: 'clear', label: 'No activity' };
    else if (readout) {
      const share = shareOf(readout.share, now.scale);
      const color = readout.control > 0 ? look.friendly(readout.as_of_hour).colour : readout.control < 0 ? look.enemy : look.contested;
      shown = readout.control === 0
        ? { kind: 'value', form: 'value', primary: '', secondary: 'Contested', swatch: color, audit: 'Contested' }
        : { kind: 'value', form: 'value', primary: share === null ? '' : formatShare(readout.control > 0 ? share : 1 - share), secondary: readout.control > 0 ? look.friendlyLabel : look.enemyLabel, swatch: color };
    }
    return { id: region.id, label: named.label, ...(named.nameMissing ? { nameMissing: named.nameMissing } : {}), pinned: region.pinned, region: Number(region.id), runs, ...(shown ? { readout: shown } : {}) };
  });
  const pieces = [...tiles, now];
  const gaps = new Map<string, { firstHour: number; lastHour: number; reason: string }>();
  for (const reply of pieces) for (const gap of reply.gaps) gaps.set(`${gap.firstHour}:${gap.lastHour}`, { firstHour: gap.firstHour, lastHour: gap.lastHour, reason: 'The recording holds no hour here' });
  return {
    hour: through,
    firstHour: tiles[0]?.firstHour ?? now.firstHour,
    lastHour: through,
    dayHours: now.dayHours,
    playhead: through,
    gaps: [...gaps.values()],
    rows,
    regions: { pickedHeading: PICKED_REGIONS_HEADING, automaticHeading: mostActiveHeading(look.viewer), addLabel: ADD_REGION_LABEL },
    readoutWidth: 204,
    inspect: (rowIndex, at) => {
      const row = rows[rowIndex];
      if (!row) return null;
      const reply = pieces[spanIndexAt(pieces, at, item => item.firstHour)];
      if (!reply || at > (reply === now ? through : reply.lastHour)) return null;
      const region = reply === now ? now.regions[rowIndex] : reply.regions.find(item => item.id === row.id);
      const readoutDay = reply === now && Math.floor((at - now.firstDayHour) / now.dayHours) === lastDay;
      return inspectDay(reply, region, row.label, at, look, findRunAtHour(row.runs, at, through)?.cell, readoutDay);
    },
    ...(rows.length ? {} : now.rankMissing ? { empty: now.rankMissing, emptyKind: 'unavailable' as const } : { empty: `No naval activity for ${look.viewer} at this date.` }),
    warnings: [...new Set(pieces.flatMap(reply => reply.warnings).concat(rows.length && now.rankMissing ? [now.rankMissing] : []))],
  };
}

function refusedOf(readView: BoundedReadView, tiles: number): string | undefined {
  return readView.refused('now') ?? dayHistoryNames(tiles).map(name => readView.refused(name)).find(Boolean);
}

function supremacySources(ctx: FamilyPlanContext, shares: boolean): SourcePlan {
  if (!ctx.countryTag) return { owner: `naval-supremacy:${ctx.branch.id}:none`, entries: [], unavailable: 'Choose a country in the widget’s header.' };
  const top = typeof ctx.options.rowCount === 'number' ? ctx.options.rowCount : SUPREMACY_TOP_DEFAULT;
  const regions: Record<string, unknown> = regionParams(readRegionChoice(ctx.options.regions));
  const base = { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) };
  const params = { ...base, top, ...regions };
  const { pinnedRegions: _pinned, hiddenRegions: _hidden, ...rankMissing } = regions;
  const entries = dayTileEntries(ctx, {
    product: 'naval-supremacy',
    now: {
      params,
      coverage: { mode: 'population', form: 'top', limit: top },
      span: (params, first, last) => ({ ...params, firstHour: first, lastHour: last }),
      validThrough: data => (data as NavalSupremacyReply | undefined)?.validThrough ?? null,
    },
    tile: {
      params: { ...base, ...rankMissing },
      coverage: { mode: 'population', form: 'all' },
      span: (params, first, last) => ({ ...params, firstHour: first, lastHour: last }),
    },
  });
  return {
    owner: `naval-supremacy:${shares ? 'shares' : 'timeline'}:${JSON.stringify(params)}`,
    entries: shares ? entries.filter(entry => entry.name === 'now') : entries,
  };
}

const COMMON = {
  kind: 'naval-supremacy',
  label: 'Naval supremacy',
  subject: 'naval supremacy',
  emptyCampaign: 'Load a campaign to see naval supremacy.',
  icon: 'anchor' as const,
  countries: { mode: 'single' } as const,
  unavailable: (ctx: FamilyPlanContext) => (ctx.countryTag ? null : 'Choose a country in the widget’s header.'),
  jumpToDate: true,
};

export const supremacySharesProjection: BarsProjection<SupremacyOptions> = { kind: 'naval-supremacy', family: 'bars', project: projectSupremacyShares };
export const supremacyTimelineProjection: HeatLanesProjection<SupremacyOptions> = { kind: 'naval-supremacy', family: 'heat-lanes', project: projectSupremacyTimeline };

export const supremacySharesDeclaration: BarsWidgetDeclaration<SupremacyOptions> = {
  ...COMMON,
  family: 'bars',
  sources: ctx => supremacySources(ctx, true),
  projection: supremacySharesProjection,
  projectionOptions: (ctx, extra) => ({ viewer: ctx.countryTag, surface: extra?.surface ?? '', tiles: 0 }),
};

export const supremacyTimelineDeclaration: HeatLanesWidgetDeclaration<SupremacyOptions> = {
  ...COMMON,
  family: 'heat-lanes',
  sources: ctx => supremacySources(ctx, false),
  projection: supremacyTimelineProjection,
  projectionOptions: (ctx, extra) => ({ viewer: ctx.countryTag, surface: extra.surface, tiles: dayTileCount(ctx.branch) }),
};
