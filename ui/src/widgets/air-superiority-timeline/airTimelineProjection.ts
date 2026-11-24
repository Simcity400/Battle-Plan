import { CHART_MEASURE_COLOURS, readableColour, authoredHex, reservedGrey } from '../../charts/palette.ts';
import { airChoiceKey } from '../../dashboard/airOptions.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
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
import { countryColourAt, type DatedColour } from '../../families/heat/scale.ts';
import type { FamilyTooltipPayload } from '../../families/host/tooltips.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import { regionIds } from '../../families/regionTokens.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { AirDayCell, AirReadout, AirSide, AirTimelineReply, AirTimelineRow } from '../../generated/dashboard';
import { LANE_READOUT_MAX_WIDTH } from '../../families/heat/lanesLayout.ts';
import { mix } from '../../shell/appearance/color.ts';
import { OPTIONS } from './definition.ts';

export const AIR_TOP_DEFAULT = 7;
const LANE_FILL = '#f5f5f5';

export interface AirTimelineProjectionOptions {
  readonly viewpointCountry: string | null;
  readonly surface: string;
}

interface AirMode {
  readonly mode: 'sides' | 'viewpoint';
  readonly viewpoint: string;
  readonly sides: readonly string[];
  readonly regions: readonly number[];
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
  const regions = regionIds(options.regions);
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
    ...(mode.regions.length ? { pinnedRegions: mode.regions } : {}),
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
    const friendly = Math.max(0, Number(cell.friendly));
    const enemy = Math.max(0, Number(cell.enemy));
    const total = friendly + enemy;
    if (total <= 0) return { cell: { kind: 'clear' }, tip: { items: [{ label: 'No activity' }] } };
    const share = friendly / total;
    const color = share > 0.5 ? depth(look.track, viewer, share)
      : share < 0.5 ? depth(look.track, look.enemy, 1 - share)
        : look.contested;
    return {
      cell: { kind: 'solid', color },
      tip: { items: [{ label: 'Air superiority', value: `${Math.round(share * 100)}%`, swatch: color }] },
    };
  }
  if (cell.unknown.length) {
    const reason = 'A side’s air score was not recorded';
    return { cell: { kind: 'missing', reason }, tip: { missingReason: reason } };
  }
  let total = 0;
  let best: { side: number; score: number } | null = null;
  const items = [];
  for (const part of cell.parts) {
    const score = Math.max(0, Number(part.friendly));
    total += score;
    if (!best || score > best.score) best = { side: part.side, score };
  }
  if (total <= 0) return { cell: { kind: 'clear' }, tip: { items: [{ label: 'No activity' }] } };
  for (const part of cell.parts) {
    const score = Math.max(0, Number(part.friendly));
    if (score <= 0) continue;
    const side = reply.sides[part.side];
    items.push({ label: sideName(side), value: `${Math.round((score / total) * 100)}%`, swatch: sideColour(side, surface, look.unknownSide) });
  }
  const share = best!.score / total;
  if (share <= 0.5) return { cell: { kind: 'solid', color: look.contested }, tip: { items: [{ label: 'Contested' }, ...items] } };
  const color = depth(look.track, sideColour(reply.sides[best!.side], surface, look.unknownSide), share);
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
      const pct = `${Math.round(readout.share_bp / 100)}%`;
      return { kind: 'value', form: 'value', primary: pct, secondary: sideName(side), swatch: sideColour(side, surface, look.unknownSide) };
    }
    case 'viewpoint': {
      const pct = `${Math.round(readout.share_bp / 100)}%`;
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

function lane(row: AirTimelineRow, reply: AirTimelineReply, through: number, look: Look, surface: string): AirLane {
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
  for (const [repeat, raw] of row.cells) {
    starts.push(day);
    const end = day + repeat - 1;
    for (let from = day; from <= end;) {
      const firstHour = dayStart(from);
      if (firstHour > through) return { runs, viewers, starts };
      const dated = raw?.kind === 'view' ? viewerColour(raw.side, reply, look, surface, observed(from)) : null;
      const until = dated && dated.through !== Infinity ? Math.max(from, Math.min(end, lastDayThrough(dated.through))) : end;
      const viewer = dated?.colour ?? look.noViewer;
      viewers.push(viewer);
      runs.push({
        firstHour,
        lastHour: Math.min(dayStart(until) + reply.dayHours - 1, through),
        readHour: reply.days[until] ?? null,
        cell: cellOf(raw, reply, look, surface, viewer).cell,
      });
      from = until + 1;
    }
    day = end + 1;
  }
  return { runs, viewers, starts };
}

function inspectDay(reply: AirTimelineReply, row: AirTimelineRow, lane: AirLane, label: string, hour: number, look: Look, surface: string): FamilyTooltipPayload | null {
  const day = Math.floor((hour - reply.firstHour) / reply.dayHours);
  const at = spanIndexAt(lane.starts, day, start => start);
  if (day < 0 || at < 0) return null;
  const raw = row.cells[at]![1];
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

export function projectAirTimeline(readView: BoundedReadView, options: AirTimelineProjectionOptions): HeatLanesModel {
  const hour: number = readView.hour;
  const refused = readView.refused('timeline');
  if (refused) return emptyModel(hour, { refused });
  const reply = readView.panel<AirTimelineReply>('timeline');
  if (!reply || !options.surface || reply.lastHour > hour) return emptyModel(hour, {});
  const through = reply.lastHour;
  const surface = options.surface;
  const noViewer = readableColour(CHART_MEASURE_COLOURS.bopRight, surface);
  const viewpointCountry = options.viewpointCountry;
  const look: Look = {
    track: mix(surface, LANE_FILL, 0.04),
    contested: reservedGrey(surface, reply.sides.flatMap(side => (side.color ? [readableColour(authoredHex(side.color), surface)] : []))),
    enemy: readableColour(CHART_MEASURE_COLOURS.short, surface),
    unknownSide: mix(surface, LANE_FILL, 0.45),
    viewer: viewpointCountry ? at => countryColourAt(readView, viewpointCountry, at, surface, noViewer) : null,
    noViewer,
  };
  const lanes = reply.rows.map(row => lane(row, reply, through, look, surface));
  const rows: HeatLaneRow[] = reply.rows.map((row, index) => {
    const label = row.name ?? 'Region name not recorded';
    const nameMissing = row.name === null ? row.nameMissing ?? 'Region name not recorded' : undefined;
    const readout = readoutOf(row.readout, reply, look, surface);
    return {
      id: String(row.region),
      label,
      ...(nameMissing ? { nameMissing } : {}),
      pinned: row.pinned,
      runs: lanes[index]!.runs,
      ...(readout ? { readout } : {}),
    };
  });
  return {
    hour: through,
    firstHour: reply.firstHour,
    lastHour: through,
    dayHours: reply.dayHours,
    playhead: through,
    gaps: reply.gaps.map(gap => ({ firstHour: gap.firstHour, lastHour: gap.lastHour, reason: 'The recording holds no hour here' })),
    rows,
    readoutWidth: LANE_READOUT_MAX_WIDTH,
    inspect: (rowIndex, at) => {
      const row = reply.rows[rowIndex];
      return row ? inspectDay(reply, row, lanes[rowIndex]!, rows[rowIndex]!.label, at, look, surface) : null;
    },
    ...(rows.length ? {} : { empty: 'No air activity at this date.' }),
    warnings: reply.warnings,
  };
}

export const airTimelineProjection: HeatLanesProjection<AirTimelineProjectionOptions> = {
  kind: 'air-superiority-timeline',
  family: 'heat-lanes',
  project: projectAirTimeline,
};

export const airTimelineDeclaration: HeatLanesWidgetDeclaration<AirTimelineProjectionOptions> = {
  family: 'heat-lanes',
  kind: 'air-superiority-timeline',
  label: 'Air superiority timeline',
  subject: 'air superiority',
  emptyCampaign: 'Load a campaign to see its air superiority.',
  icon: 'plane',
  optionFields: OPTIONS,
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
    return {
      owner: `air-superiority-timeline:${JSON.stringify(params)}`,
      entries: [{
        name: 'timeline',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'followed',
          coverage: { mode: 'population', form: 'top', limit: Number(params.top) },
          product: 'air-superiority-timeline',
          followQuery: null,
          params,
          dynamicKeys: ['lastHour'],
          buildExactParams: (base, hour) => ({ ...base, firstHour: ctx.branch.firstHour, lastHour: hour }),
        },
      }],
    };
  },
  projection: airTimelineProjection,
  projectionOptions: (ctx, extra) => {
    const viewpoint = airTimelineMode(ctx).viewpoint;
    return { viewpointCountry: viewpoint.startsWith('country:') ? viewpoint.slice(8) : null, surface: extra.surface };
  },
  jumpToDate: true,
};
