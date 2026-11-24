import { CHART_MEASURE_COLOURS, readableColour, authoredHex, reservedGrey } from '../../charts/palette.ts';
import { airChoiceKey } from '../../dashboard/airOptions.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import type { HeatLanesWidgetDeclaration } from '../../families/heat/lanesDeclaration.ts';
import type {
  HeatLaneCell,
  HeatLaneReadout,
  HeatLaneRow,
  HeatLaneRun,
  HeatLanesModel,
  HeatLanesProjection,
} from '../../families/heat/lanesModel.ts';
import type { FamilyTooltipPayload } from '../../families/host/tooltips.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import { regionIds } from '../../families/regionTokens.ts';
import type { AirDayCell, AirReadout, AirSide, AirTimelineReply, AirTimelineRow } from '../../generated/dashboard';
import { LANE_READOUT_MAX_WIDTH } from '../../families/heat/lanesLayout.ts';
import { mix, surfaceMix } from '../../shell/appearance/color.ts';

export const AIR_TOP_DEFAULT = 7;

export interface AirTimelineProjectionOptions {
  readonly viewpoint: boolean;
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
  readonly viewpointColour: string;
}

function cellOf(cell: AirDayCell | null, reply: AirTimelineReply, look: Look, surface: string): { cell: HeatLaneCell; tip: Omit<FamilyTooltipPayload, 'title' | 'hour'> } {
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
    const color = share > 0.5 ? depth(look.track, look.viewpointColour, share)
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
      const swatch = readout.share_bp > 5000 ? look.viewpointColour : readout.share_bp < 5000 ? look.enemy : look.contested;
      return { kind: 'value', form: 'value', primary: pct, secondary: 'Air superiority', swatch };
    }
  }
}

function lane(row: AirTimelineRow, reply: AirTimelineReply, name: string, through: number, look: Look, surface: string): HeatLaneRun[] {
  const runs: HeatLaneRun[] = [];
  let day = 0;
  let previousKey = '';
  for (const [repeat, raw] of row.cells) {
    for (let i = 0; i < repeat; i++, day++) {
      const first = reply.firstHour + day * reply.dayHours;
      if (first > through) return runs;
      const last = Math.min(first + reply.dayHours - 1, through);
      const read = reply.days[day] ?? null;
      const made = cellOf(raw, reply, look, surface);
      const key = JSON.stringify(made.cell);
      const previous = runs[runs.length - 1];
      if (previous && key === previousKey) {
        runs[runs.length - 1] = { ...previous, lastHour: last, readHour: read ?? previous.readHour, tooltip: { ...previous.tooltip!, hour: read ?? previous.tooltip!.hour } };
        continue;
      }
      previousKey = key;
      runs.push({ firstHour: first, lastHour: last, readHour: read, cell: made.cell, tooltip: { title: name, hour: read ?? first, ...made.tip } });
    }
  }
  return runs;
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
  const track = surfaceMix(options.surface, 0.04);
  const style = reply.viewpointCountryStyle;
  const viewpointSide = reply.viewpoint === null ? undefined : reply.sides[reply.viewpoint];
  const look: Look = {
    track,
    contested: reservedGrey(options.surface, reply.sides.flatMap(side => (side.color ? [readableColour(authoredHex(side.color), options.surface)] : []))),
    enemy: readableColour(CHART_MEASURE_COLOURS.short, options.surface),
    unknownSide: surfaceMix(options.surface, 0.45),
    viewpointColour: style?.color ? readableColour(authoredHex(style.color), options.surface) : sideColour(viewpointSide, options.surface, readableColour(CHART_MEASURE_COLOURS.bopRight, options.surface)),
  };
  const rows: HeatLaneRow[] = reply.rows.map(row => {
    const label = row.name ?? 'Region name not recorded';
    const nameMissing = row.name === null ? row.nameMissing ?? 'Region name not recorded' : undefined;
    const readout = readoutOf(row.readout, reply, look, options.surface);
    return {
      id: String(row.region),
      label,
      ...(nameMissing ? { nameMissing } : {}),
      pinned: row.pinned,
      runs: lane(row, reply, label, through, look, options.surface),
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
  projectionOptions: (ctx, extra) => ({ viewpoint: airTimelineMode(ctx).mode === 'viewpoint', surface: extra.surface }),
  jumpToDate: true,
};
