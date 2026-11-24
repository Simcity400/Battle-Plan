import { CHART_MEASURE_COLOURS, readableColour, reservedGrey } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BarsModel, BarsProjection, BarsSegment } from '../../families/bars/model.ts';
import { buildShareRowsModel, type ShareRowEntry } from '../../families/bars/shareRows.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import type { HeatLanesWidgetDeclaration } from '../../families/heat/lanesDeclaration.ts';
import { findRunAtHour, spanIndexAt, type HeatLaneCell, type HeatLaneReadout, type HeatLaneRow, type HeatLaneRun, type HeatLanesModel, type HeatLanesProjection } from '../../families/heat/lanesModel.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import { countryColourAt, regionLabel, type DatedColour } from '../../families/heat/scale.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import { readRegionChoice, regionParams } from '../../families/regions/regionChoice.ts';
import { ADD_REGION_LABEL, mostActiveHeading, PICKED_REGIONS_HEADING } from '../../families/regions/regionRows.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import type { MissionShips, NavalSupremacyReply, SupremacyRegion, SupremacyRun } from '../../generated/dashboard';

export const SUPREMACY_TOP_DEFAULT = 7;

export interface SupremacyOptions {
  readonly viewer: string | null;
  readonly surface: string;
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
  const noColour = readableColour(CHART_MEASURE_COLOURS.bopRight, surface);
  const friendly = viewer
    ? (hour: number) => countryColourAt(readView, viewer, hour, surface, noColour)
    : () => ({ colour: noColour, through: Infinity });
  return {
    friendly,
    enemy: readableColour(CHART_MEASURE_COLOURS.short, surface),
    contested: reservedGrey(surface, [friendly(reply.lastHour).colour]),
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
  const refused = readView.refused('supremacy');
  if (refused) return buildShareRowsModel({ hour, empty: refused, emptyKind: 'unavailable' });
  const reply = readView.panel<NavalSupremacyReply>('supremacy');
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
    const share = shareOf(readout.share, reply.scale);
    if (share === null) {
      return { ...base, segments: [], missing: notRecorded('The supremacy share was not recorded'), tooltip: { title: named.label, hour: readout.as_of_hour, missingReason: 'The supremacy share was not recorded' } };
    }
    const pct = Math.round(share * 100);
    const friendly = look.friendly(readout.as_of_hour).colour;
    const segments: BarsSegment[] = [
      { key: 'friendly', label: `${look.viewer} and friends`, share, value: knownValue(share), color: friendly, inlineLabel: `Friendly ${pct}%` },
      { key: 'enemy', label: `Enemies of ${look.viewer}`, share: 1 - share, value: knownValue(1 - share), color: look.enemy, inlineLabel: `Enemy ${100 - pct}%` },
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
          { label: `${look.viewer} and friends`, value: `${pct}%`, swatch: friendly },
          { label: `Enemies of ${look.viewer}`, value: `${100 - pct}%`, swatch: look.enemy },
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
    ...(reply.regions.length ? {} : { empty: `No naval activity for ${look.viewer} at this date.` }),
    warnings: reply.warnings,
  });
}

function idle(friendly: string | null, enemy: string | null): boolean {
  return friendly !== null && enemy !== null && Number(friendly) <= 0 && Number(enemy) <= 0;
}

function runCell(run: SupremacyRun, look: Look, scale: number, friendly: string): { cell: HeatLaneCell; items: { label: string; value?: string; swatch?: string }[] } {
  if (run.control === null) return { cell: { kind: 'missing', reason: 'Naval region control was not recorded' }, items: [] };
  if (idle(run.friendly, run.enemy)) return { cell: { kind: 'clear', label: 'No activity' }, items: [{ label: 'No activity' }] };
  const share = shareOf(run.share, scale);
  const pct = share === null ? null : Math.round(share * 100);
  const color = run.control > 0 ? friendly : run.control < 0 ? look.enemy : look.contested;
  const label = run.control > 0 ? look.friendlyLabel : run.control < 0 ? look.enemyLabel : 'Contested';
  return {
    cell: { kind: 'solid', color },
    items: [{ label, swatch: color, ...(pct === null ? {} : { value: `${pct}% of the score` }) }],
  };
}

function emptyLanes(hour: number, extra: Partial<HeatLanesModel>): HeatLanesModel {
  return { hour, firstHour: hour, lastHour: hour, dayHours: 24, playhead: null, rows: [], ...extra };
}

export function projectSupremacyTimeline(readView: BoundedReadView, options: SupremacyOptions): HeatLanesModel {
  const hour: number = readView.hour;
  const refused = readView.refused('supremacy');
  if (refused) return emptyLanes(hour, { refused });
  const reply = readView.panel<NavalSupremacyReply>('supremacy');
  if (!reply || !options.surface || reply.lastHour > hour) return emptyLanes(hour, {});
  const look = lookOf(readView, reply, options);
  const through = reply.lastHour;
  const dayStart = (day: number) => reply.firstDayHour + day * reply.dayHours;
  const lastDay = Math.floor((through - reply.firstDayHour) / reply.dayHours);
  const unknownRuns: HeatLaneRun[] = reply.unknown.map(span => {
    const firstHour = Math.max(reply.firstHour, dayStart(span.firstDay));
    const lastHour = Math.min(through, dayStart(span.lastDay + 1) - 1);
    return { firstHour, lastHour, readHour: lastHour, cell: { kind: 'missing', reason: span.reason } };
  });
  const rows: HeatLaneRow[] = reply.regions.map(region => {
    const named = regionLabel(readView, region.id, region.name);
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
    const readout = region.readout;
    let shown: HeatLaneReadout | undefined;
    if (readout?.kind === 'notRecorded') shown = { kind: 'missing', reason: readout.reason };
    else if (readout && idle(readout.friendly, readout.enemy)) shown = { kind: 'clear', label: 'No activity' };
    else if (readout) {
      const share = shareOf(readout.share, reply.scale);
      const color = readout.control > 0 ? look.friendly(readout.as_of_hour).colour : readout.control < 0 ? look.enemy : look.contested;
      shown = readout.control === 0
        ? { kind: 'value', form: 'value', primary: '', secondary: 'Contested', swatch: color, audit: 'Contested' }
        : { kind: 'value', form: 'value', primary: share === null ? '' : `${Math.round((readout.control > 0 ? share : 1 - share) * 100)}%`, secondary: readout.control > 0 ? look.friendlyLabel : look.enemyLabel, swatch: color };
    }
    return { id: region.id, label: named.label, ...(named.nameMissing ? { nameMissing: named.nameMissing } : {}), pinned: region.pinned, region: Number(region.id), runs, ...(shown ? { readout: shown } : {}) };
  });
  return {
    hour: through,
    firstHour: reply.firstHour,
    lastHour: through,
    dayHours: reply.dayHours,
    playhead: through,
    gaps: reply.gaps.map(gap => ({ firstHour: gap.firstHour, lastHour: gap.lastHour, reason: 'The recording holds no hour here' })),
    rows,
    regions: { pickedHeading: PICKED_REGIONS_HEADING, automaticHeading: mostActiveHeading(look.viewer), addLabel: ADD_REGION_LABEL },
    readoutWidth: 204,
    inspect: (rowIndex, at) => {
      const region = reply.regions[rowIndex];
      if (!region) return null;
      const day = Math.floor((at - reply.firstDayHour) / reply.dayHours);
      const title = rows[rowIndex]!.label;
      const readout = region.readout;
      const dated = day === lastDay && readout?.kind === 'control' ? { hour: readout.as_of_hour } : { subtitle: formatDateOnly(dayStart(day)) };
      const unknown = reply.unknown[spanIndexAt(reply.unknown, day, span => span.firstDay)];
      if (unknown && day <= unknown.lastDay) return { title, ...dated, missingReason: unknown.reason };
      const run = region.runs[spanIndexAt(region.runs, day, item => item.firstDay)];
      if (!run || day > run.lastDay) return { title, ...dated, missingReason: 'Naval region control was not recorded' };
      const painted = findRunAtHour(rows[rowIndex]!.runs, at, through)?.cell;
      const made = runCell(run, look, reply.scale, painted?.kind === 'solid' ? painted.color : look.contested);
      return made.cell.kind === 'missing' ? { title, ...dated, missingReason: made.cell.reason } : { title, ...dated, items: made.items };
    },
    ...(rows.length ? {} : { empty: `No naval activity for ${look.viewer} at this date.` }),
    warnings: reply.warnings,
  };
}

function supremacySources(ctx: FamilyPlanContext, shares: boolean): SourcePlan {
  if (!ctx.countryTag) return { owner: `naval-supremacy:${ctx.branch.id}:none`, entries: [], unavailable: 'Choose a country in the widget’s header.' };
  const top = typeof ctx.options.rowCount === 'number' ? ctx.options.rowCount : SUPREMACY_TOP_DEFAULT;
  const params = { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag), top, ...regionParams(readRegionChoice(ctx.options.regions)) };
  return {
    owner: `naval-supremacy:${shares ? 'shares' : 'timeline'}:${JSON.stringify(params)}`,
    entries: [{
      name: 'supremacy',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'branch-to-hour',
        coverage: { mode: 'population', form: 'top', limit: top },
        product: 'naval-supremacy',
        followQuery: null,
        params,
      },
    }],
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
  projectionOptions: (ctx, extra) => ({ viewer: ctx.countryTag, surface: extra?.surface ?? '' }),
};

export const supremacyTimelineDeclaration: HeatLanesWidgetDeclaration<SupremacyOptions> = {
  ...COMMON,
  family: 'heat-lanes',
  sources: ctx => supremacySources(ctx, false),
  projection: supremacyTimelineProjection,
  projectionOptions: (ctx, extra) => ({ viewer: ctx.countryTag, surface: extra.surface }),
};
