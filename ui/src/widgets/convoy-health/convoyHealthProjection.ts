import { CHART_MEASURE_COLOURS, CONVOY_EFFICIENCY_STOPS, readableColour } from '../../charts/palette.ts';
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
  return efficiencyReadout(Number(state.value), scale, surface);
}

function efficiencyReadout(value: number, scale: number, surface: string): HeatLaneReadout {
  return {
    kind: 'value',
    form: 'value',
    primary: percentText(value, scale),
    meaning: 'Efficiency',
    ...(value >= scale ? { tone: 'muted' as const } : {}),
    swatch: readableColour(efficiencyColour(value / scale), surface, 1),
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

function tradeRows(reply: ConvoyHealthReply, through: number, surface: string): ConvoyRow[] {
  const resources = new Map(reply.resources.map(resource => [resource.id, resource.name ?? 'Resource name not recorded']));
  const dealsKnown = reply.known[1] ?? [];
  const clientsKnown = reply.known[2] ?? [];
  const routeBad = readableColour(CHART_MEASURE_COLOURS.short, surface);
  const dealName = (deal: ConvoyHealthReply['deals'][number], hour: number) => {
    const resource = deal.resource[spanIndexAt(deal.resource, hour, run => run[0])]?.[2] ?? null;
    return `${countryAt(deal.exporter, hour)} · ${resource === null ? 'Resource not recorded' : resources.get(resource) ?? 'Resource name not recorded'}`;
  };
  const rows: ConvoyRow[] = [];
  for (const deal of reply.deals) {
    const runs = seriesRuns(deal.efficiency, dealsKnown, through, value => ({ kind: 'solid', color: efficiencyColour(value / reply.scale) }), 'Resource deals were not recorded');
    if (!runs.length) continue;
    const value = valueAt(deal.efficiency, through);
    const readout: HeatLaneReadout = value !== null
      ? efficiencyReadout(value, reply.scale, surface)
      : inSpans(dealsKnown, through) ? { kind: 'clear', label: 'No deal' } : { kind: 'missing', reason: 'Resource deals were not recorded' };
    rows.push({
      row: { id: `deal:${deal.id}`, label: dealName(deal, through), runs, readout },
      inspect: hour => {
        const value = valueAt(deal.efficiency, hour);
        const item = value === null ? null : { label: 'Efficiency', value: percentText(value, reply.scale), swatch: efficiencyColour(value / reply.scale) };
        return seriesTooltip(dealName(deal, hour), hour, item, dealsKnown, 'No deal', 'Resource deals were not recorded');
      },
    });
  }
  for (const lease of reply.leases) {
    const runs = seriesRuns(lease.bad, clientsKnown, through, bad => routeCell(bad, routeBad), 'Convoy clients were not recorded');
    if (!runs.length) continue;
    const bad = valueAt(lease.bad, through);
    const readout: HeatLaneReadout = bad === null
      ? inSpans(clientsKnown, through) ? { kind: 'clear', label: 'No route' } : { kind: 'missing', reason: 'Convoy clients were not recorded' }
      : { kind: 'value', form: 'word', primary: bad ? 'Route bad' : 'Route clear', swatch: bad ? routeBad : 'hollow' };
    rows.push({
      row: { id: `lease:${lease.handle}`, label: `Lend-lease from ${countryAt(lease.giver, through)}`, runs, readout },
      inspect: hour => {
        const bad = valueAt(lease.bad, hour);
        const item = bad === null ? null : bad ? { label: 'Route bad', swatch: routeBad } : { label: 'Route clear' };
        return seriesTooltip(`Lend-lease from ${countryAt(lease.giver, hour)}`, hour, item, clientsKnown, 'No route', 'Convoy clients were not recorded');
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
    return {
      owner: `convoy-health:${JSON.stringify(params)}`,
      entries: [{
        name: 'convoys',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'branch-to-hour',
          coverage: view === 'regions' ? { mode: 'population', form: 'top', limit: top } : { mode: 'population', form: 'all' },
          product: 'convoy-health',
          followQuery: null,
          params,
          buildSpanParams: (base, span) => ({ ...base, from: span.firstHour, to: span.lastHour }),
        },
      }],
    };
  },
  projection: convoyHealthProjection,
  projectionOptions: (ctx, extra) => ({ view: convoyView(ctx.options), surface: extra.surface }),
  jumpToDate: true,
};
