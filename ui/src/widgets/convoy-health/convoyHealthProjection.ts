import { CONVOY_EFFICIENCY_STOPS, readableColour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
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
import type { FamilyTooltipPayload } from '../../families/host/tooltips.ts';
import { regionLabel } from '../../families/heat/scale.ts';
import { regionIds } from '../../families/regionTokens.ts';
import type { ConvoyCellState, ConvoyCountry, ConvoyHealthReply, ConvoyView, HourSpan } from '../../generated/dashboard';
import { mix } from '../../shell/appearance/color.ts';
import { OPTIONS } from './definition.ts';

export const CONVOY_TOP_DEFAULT = 7;

export interface ConvoyProjectionOptions {
  readonly view: ConvoyView;
  readonly surface: string;
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
  return `${Math.floor((value * 100) / scale)}%`;
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
  const value = Number(state.value);
  return { kind: 'value', form: 'value', primary: percentText(value, scale), secondary: 'Efficiency', swatch: readableColour(efficiencyColour(value / scale), surface, 1) };
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

interface ConvoyRow {
  readonly row: HeatLaneRow;
  readonly inspect: (hour: number) => FamilyTooltipPayload | null;
}

function regionRows(readView: BoundedReadView, reply: ConvoyHealthReply, through: number, surface: string): ConvoyRow[] {
  return reply.regions.map(region => {
    const { label, nameMissing } = regionLabel(readView, region.region, region.name);
    const runs: HeatLaneRun[] = [];
    for (const run of region.cells) {
      if (run.firstHour > through) break;
      const lastHour = Math.min(run.lastHour, through);
      runs.push({ firstHour: run.firstHour, lastHour, readHour: lastHour, cell: stateCell(run.state, reply.scale) });
    }
    const readout = stateReadout(region.readout, reply.scale, surface);
    return {
      row: {
        id: `r:${region.region}`,
        label,
        ...(nameMissing ? { nameMissing } : {}),
        pinned: region.pinned,
        runs,
        ...(readout ? { readout } : {}),
      },
      inspect: hour => {
        const run = region.cells[spanIndexAt(region.cells, hour, item => item.firstHour)];
        return run && hour <= run.lastHour ? stateTooltip(label, hour, run.state, reply.scale) : null;
      },
    };
  });
}

function seriesRuns(
  values: Dated,
  known: readonly HourSpan[],
  through: number,
  scale: number,
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
      ? { kind: 'solid', color: efficiencyColour(value / scale) }
      : inSpans(known, start) ? null : { kind: 'missing', reason: unknownReason };
    if (!cell) continue;
    out.push({ firstHour: start, lastHour: end, readHour: end, cell });
  }
  return out;
}

function seriesTooltip(title: string, hour: number, value: number | null, known: readonly HourSpan[], scale: number, none: string, unknownReason: string): FamilyTooltipPayload {
  if (value !== null) return { title, hour, items: [{ label: 'Efficiency', value: percentText(value, scale), swatch: efficiencyColour(value / scale) }] };
  return inSpans(known, hour) ? { title, hour, items: [{ label: none }] } : { title, hour, missingReason: unknownReason };
}

function tradeRows(reply: ConvoyHealthReply, through: number, surface: string): ConvoyRow[] {
  const resources = new Map(reply.resources.map(resource => [resource.id, resource.name ?? 'Resource name not recorded']));
  const dealsKnown = reply.known[1] ?? [];
  const clientsKnown = reply.known[2] ?? [];
  const dealName = (deal: ConvoyHealthReply['deals'][number], hour: number) => {
    const resource = deal.resource[spanIndexAt(deal.resource, hour, run => run[0])]?.[2] ?? null;
    return `${countryAt(deal.exporter, hour)} · ${resource === null ? 'Resource not recorded' : resources.get(resource) ?? 'Resource name not recorded'}`;
  };
  const rows: ConvoyRow[] = [];
  for (const deal of reply.deals) {
    const runs = seriesRuns(deal.efficiency, dealsKnown, through, reply.scale, 'Resource deals were not recorded');
    if (!runs.length) continue;
    const value = valueAt(deal.efficiency, through);
    const readout: HeatLaneReadout = value !== null
      ? { kind: 'value', form: 'value', primary: percentText(value, reply.scale), secondary: 'Efficiency', swatch: readableColour(efficiencyColour(value / reply.scale), surface, 1) }
      : inSpans(dealsKnown, through) ? { kind: 'clear', label: 'No deal' } : { kind: 'missing', reason: 'Resource deals were not recorded' };
    rows.push({
      row: { id: `deal:${deal.id}`, label: dealName(deal, through), runs, readout },
      inspect: hour => seriesTooltip(dealName(deal, hour), hour, valueAt(deal.efficiency, hour), dealsKnown, reply.scale, 'No deal', 'Resource deals were not recorded'),
    });
  }
  for (const lease of reply.leases) {
    const healthy = lease.bad.map(([first, last, bad]) => [first, last, bad ? 0 : reply.scale] as const);
    const runs = seriesRuns(healthy, clientsKnown, through, reply.scale, 'Convoy clients were not recorded');
    if (!runs.length) continue;
    const bad = valueAt(lease.bad, through);
    const readout: HeatLaneReadout = bad === null
      ? inSpans(clientsKnown, through) ? { kind: 'clear', label: 'No route' } : { kind: 'missing', reason: 'Convoy clients were not recorded' }
      : { kind: 'value', form: 'word', primary: bad ? 'Route bad' : 'Route clear', swatch: readableColour(efficiencyColour(bad ? 0 : 1), surface, 1) };
    rows.push({
      row: { id: `lease:${lease.handle}`, label: `Lend-lease from ${countryAt(lease.giver, through)}`, runs, readout },
      inspect: hour => seriesTooltip(`Lend-lease from ${countryAt(lease.giver, hour)}`, hour, valueAt(healthy, hour), clientsKnown, reply.scale, 'No route', 'Convoy clients were not recorded'),
    });
  }
  return rows;
}

function emptyModel(hour: number, extra: Partial<HeatLanesModel>): HeatLanesModel {
  return { hour, firstHour: hour, lastHour: hour, dayHours: 24, playhead: null, rows: [], ...extra };
}

export function projectConvoyHealth(readView: BoundedReadView, options: ConvoyProjectionOptions): HeatLanesModel {
  const hour: number = readView.hour;
  const refused = readView.refused('convoys');
  if (refused) return emptyModel(hour, { refused });
  const reply = readView.panel<ConvoyHealthReply>('convoys');
  if (!reply || !options.surface || reply.lastHour > hour || reply.view !== options.view) return emptyModel(hour, {});
  const through = reply.lastHour;
  const built = reply.view === 'regions' ? regionRows(readView, reply, through, options.surface) : tradeRows(reply, through, options.surface);
  const rows = built.map(item => item.row);
  return {
    hour: through,
    firstHour: reply.firstHour,
    lastHour: through,
    dayHours: 24,
    playhead: through,
    gaps: reply.gaps.map(gap => ({ firstHour: gap.firstHour, lastHour: gap.lastHour, reason: 'The recording holds no hour here' })),
    rows,
    inspect: (rowIndex, at) => built[rowIndex]?.inspect(at) ?? null,
    ...(rows.length ? {} : { empty: reply.view === 'regions' ? 'No convoys for this country at this date.' : 'No convoyed trade for this country at this date.' }),
    warnings: reply.warnings,
  };
}

export const convoyHealthProjection: HeatLanesProjection<ConvoyProjectionOptions> = {
  kind: 'convoy-health',
  family: 'heat-lanes',
  project: projectConvoyHealth,
};

export const convoyHealthDeclaration: HeatLanesWidgetDeclaration<ConvoyProjectionOptions> = {
  family: 'heat-lanes',
  kind: 'convoy-health',
  label: 'Convoy health',
  subject: 'convoys',
  emptyCampaign: 'Load a campaign to see convoys.',
  icon: 'anchor',
  optionFields: OPTIONS,
  countries: { mode: 'single' },
  unavailable: ctx => (ctx.countryTag ? null : 'Choose a country in widget settings.'),
  sources: (ctx): SourcePlan => {
    const view = convoyView(ctx.options);
    if (!ctx.countryTag) return { owner: `convoy-health:${ctx.branch.id}:none`, entries: [], unavailable: 'Choose a country in widget settings.' };
    const pinned = view === 'regions' ? regionIds(ctx.options.regions) : [];
    const top = typeof ctx.options.rowCount === 'number' ? ctx.options.rowCount : CONVOY_TOP_DEFAULT;
    const params = {
      branchId: ctx.branch.id,
      view,
      country: countryFromKey(ctx.countryTag),
      from: null,
      to: null,
      ...(view === 'regions' ? { top, ...(pinned.length ? { pinnedRegions: pinned } : {}) } : {}),
    };
    const firstHour = ctx.branch.firstHour;
    return {
      owner: `convoy-health:${JSON.stringify(params)}`,
      entries: [{
        name: 'convoys',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'followed',
          coverage: view === 'regions' ? { mode: 'population', form: 'top', limit: top } : { mode: 'population', form: 'all' },
          product: 'convoy-health',
          followQuery: null,
          params,
          dynamicKeys: ['to'],
          buildExactParams: (base, hour) => ({ ...base, from: firstHour, to: hour }),
        },
      }],
    };
  },
  projection: convoyHealthProjection,
  projectionOptions: (ctx, extra) => ({ view: convoyView(ctx.options), surface: extra.surface }),
  jumpToDate: true,
};
