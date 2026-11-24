import { CHART_MEASURE_COLOURS, readableColour, authoredHex, reservedGrey } from '../../charts/palette.ts';
import { airChoiceKey } from '../../dashboard/airOptions.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import { dayTileCount, dayTileEntries } from '../../families/heat/dayTiles.ts';
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
import { countryColourAt, regionLabel, type DatedColour } from '../../families/heat/scale.ts';
import type { FamilyTooltipPayload } from '../../families/host/tooltips.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import { readRegionChoice, regionParams, type RegionChoice } from '../../families/regions/regionChoice.ts';
import { countrySubject, regionLanes } from '../../families/regions/regionRows.ts';
import { formatDateOnly, formatShare } from '../../families/valueFormat.ts';
import type { AirDayCell, AirReadout, AirSide, AirTimelineReply, AirTimelineRow } from '../../generated/dashboard';
import { LANE_READOUT_MAX_WIDTH } from '../../families/heat/lanesLayout.ts';
import { mix, surfaceMix } from '../../shell/appearance/color.ts';

export const AIR_TOP_DEFAULT = 7;

export interface AirTimelineProjectionOptions {
  readonly viewpointCountry: string | null;
  readonly tiles: number;
  readonly surface: string;
}

interface AirMode {
  readonly mode: 'sides' | 'viewpoint';
  readonly viewpoint: string;
  readonly sides: readonly string[];
  readonly regions: RegionChoice;
  readonly theatres: readonly string[];
}

export function airTimelineMode(ctx: Pick<FamilyPlanContext, 'options' | 'track' | 'countryTag'>): AirMode {
  const options = ctx.options;
  const campaign = ctx.track.source.metadata.campaignKey ?? '';
  const mode = options.mode === 'viewpoint' ? 'viewpoint' as const : 'sides' as const;
  const chosen = options.viewpoint ? String(options.viewpoint) : '';
  const viewpoint = mode !== 'viewpoint'
    ? ''
    : chosen
      ? contentKey(chosen).startsWith('country:') ? contentKey(chosen) : airChoiceKey(campaign, chosen) ?? ''
      : ctx.countryTag ? `country:${ctx.countryTag}` : '';
  const regions = readRegionChoice(options.regions);
  return {
    mode,
    viewpoint,
    sides: contentSet(typeof options.sides === 'string' ? options.sides : '').flatMap(value => airChoiceKey(campaign, value) ?? []),
    regions,
    theatres: contentSet(typeof options.theatres === 'string' ? options.theatres : '').flatMap(value => airChoiceKey(campaign, value) ?? []),
  };
}

export function airTimelineParams(ctx: FamilyPlanContext, mode: AirMode): Record<string, unknown> {
  const top = typeof ctx.options.regionCount === 'number' ? ctx.options.regionCount : AIR_TOP_DEFAULT;
  return {
    branchId: ctx.branch.id,
    mode: mode.mode,
    ...(mode.viewpoint
      ? mode.viewpoint.startsWith('country:')
        ? { viewpoint: countryFromKey(mode.viewpoint.slice(8)), viewpointCountry: true }
        : { viewpoint: mode.viewpoint }
      : {}),
    ...(mode.sides.length ? { sides: mode.sides } : {}),
    ...(mode.theatres.length ? { theatres: mode.theatres } : {}),
    ...regionParams(mode.regions),
    top,
  };
}

const REASONS: Readonly<Record<string, string>> = {
  'theatres-not-recorded': 'Air theatres were not recorded',
  'side-not-present': 'This side was not present',
};

function sideColour(side: AirSide | undefined, surface: string, fallback: string): string {
  return side?.color ? readableColour(authoredHex(side.color), surface) : fallback;
}

function sideName(side: AirSide | undefined): string {
  return side?.name ?? (side?.nameMissing ? 'Name not recorded' : 'Side not recorded');
}

function depth(track: string, color: string, strength: number): string {
  return mix(track, color, 0.28 + 0.72 * Math.max(0, Math.min(1, strength)));
}

interface Look {
  readonly track: string;
  readonly contested: string;
  readonly enemy: string;
  readonly unknownSide: string;
  readonly viewer: ((hour: number) => DatedColour) | null;
  readonly noViewer: string;
}

interface Made {
  readonly cell: HeatLaneCell;
  readonly tip: Omit<FamilyTooltipPayload, 'title' | 'hour'>;
}

function viewerColour(side: number | null, reply: AirTimelineReply, look: Look, surface: string, hour: number): DatedColour {
  if (look.viewer) return look.viewer(hour);
  return { colour: sideColour(side === null ? undefined : reply.sides[side], surface, look.noViewer), through: Infinity };
}

function cellOf(cell: AirDayCell | null, reply: AirTimelineReply, look: Look, surface: string, viewer: string): Made {
  if (!cell) return { cell: { kind: 'missing', reason: 'Not recorded on this day' }, tip: { missingReason: 'Not recorded on this day' } };
  if (cell.kind === 'reason') {
    if (cell.reason === 'outside-theatres') return { cell: { kind: 'clear' }, tip: { items: [{ label: 'Outside the chosen air theatres' }] } };
    const reason = REASONS[cell.reason] ?? cell.reason;
    return { cell: { kind: 'missing', reason }, tip: { missingReason: reason } };
  }
  if (cell.kind === 'view') {
    if (cell.share_bp === null) return { cell: { kind: 'clear' }, tip: { items: [{ label: 'No activity' }] } };
    const share = cell.share_bp / 10_000;
    const color = share > 0.5 ? depth(look.track, viewer, share)
      : share < 0.5 ? depth(look.track, look.enemy, 1 - share)
        : look.contested;
    return {
      cell: { kind: 'solid', color },
      tip: { items: [{ label: 'Air superiority', value: formatShare(share), swatch: color }] },
    };
  }
  const lead = cell.lead;
  if (lead.kind === 'unknown') {
    const reason = 'A side’s air score was not recorded';
    return { cell: { kind: 'missing', reason }, tip: { missingReason: reason } };
  }
  if (lead.kind === 'noActivity') return { cell: { kind: 'clear' }, tip: { items: [{ label: 'No activity' }] } };
  let total = 0;
  for (const part of cell.parts) total += Math.max(0, Number(part.friendly));
  const items = [];
  for (const part of cell.parts) {
    const score = Math.max(0, Number(part.friendly));
    if (score <= 0) continue;
    const side = reply.sides[part.side];
    items.push({ label: sideName(side), value: formatShare(score / total), swatch: sideColour(side, surface, look.unknownSide) });
  }
  if (lead.kind === 'contested') return { cell: { kind: 'solid', color: look.contested }, tip: { items: [{ label: 'Contested' }, ...items] } };
  const color = depth(look.track, sideColour(reply.sides[lead.side], surface, look.unknownSide), lead.share_bp / 10_000);
  return { cell: { kind: 'solid', color }, tip: { items } };
}

function readoutOf(readout: AirReadout | undefined, reply: AirTimelineReply, look: Look, surface: string): HeatLaneReadout | undefined {
  if (!readout) return undefined;
  switch (readout.kind) {
    case 'notRecorded':
      return { kind: 'missing', reason: REASONS[readout.reason] ?? readout.reason };
    case 'noActivity':
      return { kind: 'clear', label: 'No activity' };
    case 'contested':
      return { kind: 'value', form: 'value', primary: '', secondary: 'Contested', swatch: look.contested, audit: 'Contested' };
    case 'leading': {
      const side = reply.sides[readout.side];
      const pct = formatShare(readout.share_bp / 10_000);
      return { kind: 'value', form: 'value', primary: pct, secondary: sideName(side), swatch: sideColour(side, surface, look.unknownSide) };
    }
    case 'viewpoint': {
      const pct = formatShare(readout.share_bp / 10_000);
      const viewer = viewerColour(readout.side, reply, look, surface, readout.as_of_hour).colour;
      const swatch = readout.share_bp > 5000 ? viewer : readout.share_bp < 5000 ? look.enemy : look.contested;
      return { kind: 'value', form: 'value', primary: pct, secondary: 'Air superiority', swatch };
    }
  }
}

interface AirLane {
  readonly runs: readonly HeatLaneRun[];
  readonly viewers: readonly string[];
  readonly starts: readonly number[];
}

function lane(cells: AirTimelineRow['cells'], reply: AirTimelineReply, through: number, look: Look, surface: string, cellsOf: Map<string, HeatLaneCell>): AirLane {
  const runs: HeatLaneRun[] = [];
  const viewers: string[] = [];
  const starts: number[] = [];
  const dayStart = (day: number) => reply.firstHour + day * reply.dayHours;
  const observed = (day: number) => reply.days[day] ?? dayStart(day);
  const lastDayThrough = (hour: number) => {
    const day = Math.floor((hour - reply.firstHour) / reply.dayHours);
    return observed(day) > hour ? day - 1 : day;
  };
  let day = 0;
  for (const [repeat, raw] of cells) {
    starts.push(day);
    const end = day + repeat - 1;
    const key = JSON.stringify(raw);
    for (let from = day; from <= end;) {
      const firstHour = dayStart(from);
      if (firstHour > through) return { runs, viewers, starts };
      const dated = raw?.kind === 'view' ? viewerColour(raw.side, reply, look, surface, observed(from)) : null;
      const until = dated && dated.through !== Infinity ? Math.max(from, Math.min(end, lastDayThrough(dated.through))) : end;
      const viewer = dated?.colour ?? look.noViewer;
      const memo = `${key}|${viewer}`;
      let cell = cellsOf.get(memo);
      if (!cell) cellsOf.set(memo, cell = cellOf(raw, reply, look, surface, viewer).cell);
      viewers.push(viewer);
      runs.push({ firstHour, lastHour: Math.min(dayStart(until) + reply.dayHours - 1, through), readHour: reply.days[until] ?? null, cell });
      from = until + 1;
    }
    day = end + 1;
  }
  return { runs, viewers, starts };
}

interface TileLanes {
  readonly sig: string;
  readonly byRegion: Map<number, AirLane>;
  readonly absent: AirLane;
  readonly cells: Map<number, AirTimelineRow['cells']>;
}

const heldTiles = new WeakMap<AirTimelineReply, TileLanes>();

function tileLanes(tile: AirTimelineReply, sig: string, look: Look, surface: string): TileLanes {
  const held = heldTiles.get(tile);
  if (held && held.sig === sig) return held;
  const byRegion = new Map<number, AirLane>();
  const cells = new Map<number, AirTimelineRow['cells']>();
  const cellsOf = new Map<string, HeatLaneCell>();
  for (const row of tile.rows) {
    byRegion.set(row.region, lane(row.cells, tile, tile.lastHour, look, surface, cellsOf));
    cells.set(row.region, row.cells);
  }
  const made = { sig, byRegion, absent: lane(tile.absent ?? [], tile, tile.lastHour, look, surface, cellsOf), cells };
  heldTiles.set(tile, made);
  return made;
}

interface Piece {
  readonly reply: AirTimelineReply;
  readonly lane: AirLane;
  readonly cells: AirTimelineRow['cells'];
}

function inspectDay(piece: Piece, label: string, hour: number, look: Look, surface: string): FamilyTooltipPayload | null {
  const { reply, lane } = piece;
  const day = Math.floor((hour - reply.firstHour) / reply.dayHours);
  const at = spanIndexAt(lane.starts, day, start => start);
  if (day < 0 || at < 0) return null;
  const raw = piece.cells[at]![1];
  const read = reply.days[day] ?? null;
  const viewer = lane.viewers[spanIndexAt(lane.runs, hour, run => run.firstHour)] ?? look.noViewer;
  const made = cellOf(raw, reply, look, surface, viewer);
  return read === null
    ? { title: label, subtitle: formatDateOnly(reply.firstHour + day * reply.dayHours), ...made.tip }
    : { title: label, hour: read, ...made.tip };
}

function emptyModel(hour: number, extra: Partial<HeatLanesModel>): HeatLanesModel {
  return { hour, firstHour: hour, lastHour: hour, dayHours: 24, playhead: null, rows: [], ...extra };
}

function datedSignature(look: Look, first: number, last: number): string {
  if (!look.viewer) return '';
  const out: string[] = [];
  for (let at = first; at <= last;) {
    const dated = look.viewer(at);
    out.push(dated.colour);
    if (dated.through >= last) break;
    at = dated.through + 1;
  }
  return out.join(',');
}

export function projectAirTimeline(readView: BoundedReadView, options: AirTimelineProjectionOptions): HeatLanesModel {
  const hour: number = readView.hour;
  const refused = readView.refused('now') ?? Array.from({ length: options.tiles }, (_, k) => readView.refused(`tile:${k}`)).find(Boolean);
  if (refused) return emptyModel(hour, { refused });
  const now = readView.panel<AirTimelineReply>('now');
  if (!now || !options.surface || now.lastHour > hour) return emptyModel(hour, {});
  const tiles: AirTimelineReply[] = [];
  for (let k = 0; k < options.tiles; k++) {
    const tile = readView.panel<AirTimelineReply>(`tile:${k}`);
    if (tile && tile.lastHour < now.firstHour) tiles.push(tile);
  }
  const through = now.lastHour;
  const surface = options.surface;
  const noViewer = readableColour(CHART_MEASURE_COLOURS.bopRight, surface);
  const viewpointCountry = options.viewpointCountry;
  const sideColours = new Set<string>();
  for (const reply of [...tiles, now]) for (const side of reply.sides) if (side.color) sideColours.add(readableColour(authoredHex(side.color), surface));
  const look: Look = {
    track: surfaceMix(surface, 0.04),
    contested: reservedGrey(surface, [...sideColours]),
    enemy: readableColour(CHART_MEASURE_COLOURS.short, surface),
    unknownSide: surfaceMix(surface, 0.45),
    viewer: viewpointCountry ? at => countryColourAt(readView, viewpointCountry, at, surface, noViewer) : null,
    noViewer,
  };
  const subject = countrySubject(readView, options.viewpointCountry);
  const held = tiles.map(tile => tileLanes(tile, `${surface}|${look.contested}|${datedSignature(look, tile.firstHour, tile.lastHour)}`, look, surface));
  const cellsOf = new Map<string, HeatLaneCell>();
  const pieces: Piece[][] = now.rows.map(row => [
    ...tiles.map((tile, k) => {
      const lanes = held[k]!;
      const own = lanes.byRegion.get(row.region);
      return { reply: tile, lane: own ?? lanes.absent, cells: own ? lanes.cells.get(row.region)! : tile.absent ?? [] };
    }),
    { reply: now, lane: lane(row.cells, now, through, look, surface, cellsOf), cells: row.cells },
  ]);
  const rows: HeatLaneRow[] = now.rows.map((row, index) => {
    const { label, nameMissing } = regionLabel(readView, row.region, row.name);
    const readout = readoutOf(row.readout, now, look, surface);
    const own = pieces[index]!;
    return {
      id: String(row.region),
      label,
      ...(nameMissing ? { nameMissing } : {}),
      pinned: row.pinned,
      region: row.region,
      runs: own.length === 1 ? own[0]!.lane.runs : own.flatMap(piece => piece.lane.runs),
      ...(readout ? { readout } : {}),
    };
  });
  const gaps = new Map<string, { firstHour: number; lastHour: number; reason: string }>();
  for (const reply of [...tiles, now]) for (const gap of reply.gaps) gaps.set(`${gap.firstHour}:${gap.lastHour}`, { firstHour: gap.firstHour, lastHour: gap.lastHour, reason: 'The recording holds no hour here' });
  return {
    hour: through,
    firstHour: tiles[0]?.firstHour ?? now.firstHour,
    lastHour: through,
    dayHours: now.dayHours,
    playhead: through,
    gaps: [...gaps.values()],
    rows,
    regions: regionLanes(subject),
    readoutWidth: LANE_READOUT_MAX_WIDTH,
    inspect: (rowIndex, at) => {
      const own = pieces[rowIndex];
      const piece = own?.[spanIndexAt(own, at, item => item.reply.firstHour)];
      return piece && at <= piece.reply.lastHour ? inspectDay(piece, rows[rowIndex]!.label, at, look, surface) : null;
    },
    ...(rows.length ? {} : { empty: `No air activity${subject ? ` for ${subject}` : ''} at this date.` }),
    warnings: [...new Set([...tiles, now].flatMap(reply => reply.warnings))],
  };
}

export const airTimelineProjection: HeatLanesProjection<AirTimelineProjectionOptions> = {
  kind: 'air-superiority-timeline',
  family: 'heat-lanes',
  project: projectAirTimeline,
};

function airEntries(ctx: FamilyPlanContext, params: Record<string, unknown>): SourcePlan['entries'] {
  const { top, pinnedRegions: _pinned, ...unranked } = params;
  return dayTileEntries(ctx, {
    product: 'air-superiority-timeline',
    now: {
      params,
      coverage: { mode: 'population', form: 'top', limit: Number(top) },
      span: (base, first, last, rankFrom) => ({ ...base, firstHour: first, lastHour: last, rankFrom }),
      validThrough: () => null,
    },
    tile: {
      params: { ...unranked, every: true },
      coverage: { mode: 'population', form: 'all' },
      span: (base, first, last) => ({ ...base, firstHour: first, lastHour: last }),
    },
  });
}

export const airTimelineDeclaration: HeatLanesWidgetDeclaration<AirTimelineProjectionOptions> = {
  family: 'heat-lanes',
  kind: 'air-superiority-timeline',
  label: 'Air superiority timeline',
  subject: 'air superiority',
  emptyCampaign: 'Load a campaign to see its air superiority.',
  icon: 'plane',
  countries: { mode: 'single' },
  unavailable: ctx => {
    const mode = airTimelineMode(ctx);
    return mode.mode === 'viewpoint' && !mode.viewpoint ? 'Choose a dashboard country or a viewpoint in the widget settings.' : null;
  },
  sources: (ctx): SourcePlan => {
    const mode = airTimelineMode(ctx);
    const params = airTimelineParams(ctx, mode);
    if (mode.mode === 'viewpoint' && !mode.viewpoint) {
      return { owner: `air-superiority-timeline:${ctx.branch.id}:none`, entries: [], unavailable: 'Choose a dashboard country or a viewpoint in the widget settings.' };
    }
    return { owner: `air-superiority-timeline:${JSON.stringify(params)}`, entries: airEntries(ctx, params) };
  },
  projection: airTimelineProjection,
  projectionOptions: (ctx, extra) => {
    const viewpoint = airTimelineMode(ctx).viewpoint;
    return { viewpointCountry: viewpoint.startsWith('country:') ? viewpoint.slice(8) : null, tiles: dayTileCount(ctx.branch), surface: extra.surface };
  },
  jumpToDate: true,
};
