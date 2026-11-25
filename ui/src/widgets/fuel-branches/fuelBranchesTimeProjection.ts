import { colour } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { heldParts, keyId, type HeldPart, type SeriesKey } from '../../dashboard/keyedSeriesData.ts';
import { FutureDataError, type BoundedReadView, type FamilyPlanContext, type SourcePlan } from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import type { ChartGap, ChartHoverDetail, ChartLegendEntry, TimeChartModel, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import {
  auditReading,
  branchGapsOf,
  headReading,
  headValueOf,
  missingSpansOf,
  type HeadReading,
  type KnownHeadReading,
} from '../../families/time-chart/seriesHelpers.ts';
import { chartAxis, chartWindow, DEFAULT_LINE_SETTINGS, lineChartSettings, type LineChartSettings } from '../../families/time-chart/settings.ts';
import { formatRational, formatScalar } from '../../families/valueFormat.ts';
import type { FuelBranchesReply } from '../../generated/dashboard';
import { branchName, FUEL_BRANCHES, FUEL_CONSUMERS, validateFuelBranches } from './fuelBranchesData.ts';

export const FUEL_BRANCHES_TIME_VIEW = 'time';

const SHARE_FAMILY = { kind: FUEL_CONSUMERS, measure: 'receivedShare' } as const;
const SHARE_PLACES = 4;

export interface FuelPartsMissing {
  readonly firstHour: number;
  readonly lastHour: number;
  readonly reason: string;
}

export interface FuelPartsAux {
  readonly partBits: number;
  readonly parts: readonly HeldPart[];
  readonly missing: readonly FuelPartsMissing[];
}

export interface FuelBranchesTimeOptions {
  readonly subject: string | null;
  readonly parts: FuelPartsAux | null;
  readonly endLabels: boolean;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly settings?: LineChartSettings;
}

interface HoverState {
  readonly country?: string | number;
  readonly part?: string;
  readonly hour: number;
  readonly run: { readonly valueRaw: string } | null;
  readonly missing: string | null;
}

const isObj = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const isInt = (value: unknown): value is number => Number.isSafeInteger(value);
const isDigits = (value: unknown): value is string => typeof value === 'string' && /^\d+$/.test(value);

export function validateFuelParts(value: unknown): FuelPartsAux {
  const owner = isObj(value) && Array.isArray(value.owners) && value.owners.length === 1 ? value.owners[0] : undefined;
  if (
    !isObj(value)
    || !Number.isInteger(value.partBits)
    || (value.partBits as number) < 1
    || (value.partBits as number) > 127
    || !isObj(owner)
    || !Array.isArray(owner.parts)
    || !owner.parts.every(part =>
      isObj(part) && isDigits(part.part) && isInt(part.firstHour) && isInt(part.lastHour) && part.firstHour <= part.lastHour)
    || !Array.isArray(owner.missing)
    || !owner.missing.every(span => isObj(span) && isInt(span.firstHour) && isInt(span.lastHour) && typeof span.reason === 'string')
  ) {
    throw new Error('The fuel consumer parts reply is malformed.');
  }
  const rawParts = (owner.parts as Record<string, unknown>[]).map(part => ({
    part: part.part as string,
    id: typeof part.id === 'string' ? part.id : null,
    firstHour: part.firstHour as number,
    lastHour: part.lastHour as number,
  }));
  const parts = heldParts(rawParts).sort((a, b) => Number(a.part) - Number(b.part));
  return {
    partBits: value.partBits as number,
    parts,
    missing: owner.missing as FuelPartsMissing[],
  };
}

function formatFuelShare(reading: KnownHeadReading): string {
  if (reading.raw !== undefined && reading.denominator !== undefined) {
    return formatRational(reading.raw, reading.denominator, { maxDecimals: 2, percent: true });
  }
  return formatScalar(reading.value, { decimals: 0, maxDecimals: 2, percent: true });
}

function formatRawShare(valueRaw: string, decimals: number): string {
  return formatRational(BigInt(valueRaw), 10n ** BigInt(decimals), { maxDecimals: 2, percent: true });
}

function branchLabel(reply: FuelBranchesReply | undefined, index: number): string {
  return reply === undefined ? 'Loading…' : branchName(reply.consumerNames, index);
}

function fuelHoverDetail(
  readView: BoundedReadView,
  subject: string,
  branches: readonly { readonly part: string; readonly name: string; readonly color: string }[],
): ChartHoverDetail | undefined {
  let hover;
  try {
    hover = readView.hover<{ readonly states?: readonly HoverState[]; readonly decimals?: number }>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return undefined;
    throw error;
  }
  if (!hover?.data) return undefined;
  const at = hover.hour;
  const states = Array.isArray(hover.data.states) ? hover.data.states : [];
  const decimals = typeof hover.data.decimals === 'number' ? hover.data.decimals : SHARE_PLACES;
  const items: TooltipListItem[] = branches.map(branch => {
    const state = states.find(item =>
      item.hour === at
      && item.part === branch.part
      && item.country !== undefined
      && countryKey(item.country) === subject);
    return state?.run
      ? { label: branch.name, swatch: branch.color, value: formatRawShare(state.run.valueRaw, decimals), strong: true }
      : { label: branch.name, swatch: branch.color, missingReason: state?.missing ?? 'No share', tone: 'muted' };
  });
  return { hour: at, items, replacesSeries: true };
}

function project(readView: BoundedReadView, options: FuelBranchesTimeOptions): TimeChartModel {
  const hour = readView.hour;
  const settings = options.settings ?? DEFAULT_LINE_SETTINGS;
  const view = chartWindow(settings, hour, options.extent[0]);
  const yAxis = { format: 'percent' as const, ...chartAxis(settings) };
  const base = {
    kind: 'fuel-branches',
    measureTitle: 'Fuel received',
    mode: 'lines' as const,
    hour,
    extent: options.extent,
    ...(view ? { view } : {}),
    yAxis,
    series: [] as readonly TimeChartSeriesModel[],
  };

  if (!options.subject || !options.parts) return { ...base, empty: null };

  const namesRefused = readView.refused('names');
  const namesReply = namesRefused ? undefined : readView.panel<FuelBranchesReply>('names');
  const warnings = [
    ...(namesRefused ? [namesRefused] : []),
    ...(namesReply?.warnings ?? []),
  ];

  const visibleParts = options.parts.parts.filter(part => part.firstHour <= hour);
  const missing = options.parts.missing.filter(span => span.firstHour <= hour);

  if (!visibleParts.length) {
    return {
      ...base,
      warnings,
      empty: missing.length
        ? { message: `No recorded fuel consumers here: ${missing[0]!.reason}`, kind: 'unavailable' }
        : { message: 'No fuel requested in this time.', kind: 'empty' },
    };
  }

  const shares = readView.series('shares');
  const branches = visibleParts.map(part => {
    const index = Number(part.part);
    const id = keyId({ tag: options.subject!, part: part.part });
    const name = branchLabel(namesReply, index);
    const color = colour(options.surface, index);
    return { part: part.part, index, id, name, color };
  });

  const readings: [string, HeadReading | null][] = [];
  const series: TimeChartSeriesModel[] = branches.map(branch => {
    const drawn = shares?.get(branch.id) ?? null;
    const reading = headReading(
      readView,
      'shares',
      { tag: options.subject!, part: branch.part, partBits: options.parts!.partBits },
      SHARE_PLACES,
    );
    readings.push([branch.part, reading]);
    return {
      id: branch.id,
      key: branch.part,
      name: branch.name,
      color: branch.color,
      role: 'primary',
      step: true,
      bounded: drawn,
      headValue: headValueOf(reading),
      headHour: hour,
      missingSpans: missingSpansOf(drawn),
      endLabel: options.endLabels && reading && 'value' in reading
        ? { colorBar: branch.color, name: branch.name, fullName: branch.name, value: formatFuelShare(reading) }
        : null,
    };
  });

  const legend: ChartLegendEntry[] = branches.map(branch => ({
    id: branch.part,
    label: branch.name,
    color: branch.color,
  }));

  const gaps: ChartGap[] = [
    ...options.gaps,
    ...missing.map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour, reason: span.reason })),
  ];
  const detail = fuelHoverDetail(readView, options.subject, branches);

  return {
    ...base,
    series,
    legend,
    ...(gaps.length ? { gaps } : {}),
    ...(detail ? { hoverDetail: detail } : {}),
    warnings,
    auditRows: readings.map(([row, reading]) => auditReading(row, reading, formatFuelShare)),
  };
}

export const fuelBranchesTimeProjection: TimeChartProjection<FuelBranchesTimeOptions> = registerFamilyProjection({
  kind: 'fuel-branches',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Fuel received',
  nextChange(readView, options) {
    const parts = options.parts?.parts;
    if (!parts?.length) return Infinity;
    let next = Infinity;
    for (const part of parts) {
      if (part.firstHour > readView.hour && part.firstHour < next) next = part.firstHour;
    }
    return next;
  },
  project,
}, FUEL_BRANCHES_TIME_VIEW);

export function fuelBranchesTimeSources(ctx: FamilyPlanContext<FuelPartsAux>): SourcePlan {
  const subject = ctx.countrySubject;
  const parts = ctx.aux;
  const owner = `fuel-branches:time:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${subject ?? ''}`;
  if (!subject || !parts) return { owner, entries: [] };
  const keys: SeriesKey[] = parts.parts.map(part => ({ tag: subject, part: part.part }));
  if (!keys.length) return { owner: `${owner}:empty`, entries: [], allowEmptyFollow: true };
  const country = countryFromKey(subject);
  const reads = keys.map(key => ({ country, part: key.part }));
  const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
  return {
    owner: `${owner}:${keys.map(keyId).join(',')}`,
    entries: [
      {
        name: 'names',
        ...common,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'population', form: 'all' },
          product: FUEL_BRANCHES,
          followQuery: { panel: FUEL_BRANCHES, branchId: ctx.branch.id, country },
          params: { branchId: ctx.branch.id, country },
          validate: validateFuelBranches,
        },
      },
      {
        name: 'shares',
        ...common,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'keyed', keys: keys.map(keyId) },
          family: SHARE_FAMILY,
          tags: [subject],
          keys,
          partBits: parts.partBits,
          headQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family: SHARE_FAMILY, reads: [{ country }] } },
          params: { branchId: ctx.branch.id, family: SHARE_FAMILY, reads },
          exactProduct: 'series-state',
        },
      },
      {
        name: 'hover',
        ...common,
        read: {
          shape: 'exact-hover',
          product: 'series-state',
          params: { branchId: ctx.branch.id, family: SHARE_FAMILY, reads },
        },
      },
    ],
  };
}

export const fuelBranchesTimeDeclaration: TimeChartWidgetDeclaration<FuelBranchesTimeOptions, FuelPartsAux> = {
  family: 'time-chart',
  kind: 'fuel-branches',
  label: 'Fuel by branch over time',
  subject: 'fuel by branch',
  emptyCampaign: 'Load a campaign to see fuel.',
  icon: 'droplet',
  countries: { mode: 'single', choose: 'Choose a country to see its fuel by branch.' },
  aux: {
    product: 'series-parts',
    params: (track, _options, _country, subject) => (
      subject
        ? {
            branchId: track.branch.id,
            family: SHARE_FAMILY,
            countries: [countryFromKey(subject)],
            from: track.branch.firstHour,
            to: track.branch.lastHour,
          }
        : null
    ),
    validate: validateFuelParts,
  },
  sources: fuelBranchesTimeSources,
  projection: fuelBranchesTimeProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      subject: ctx.countrySubject,
      parts: ctx.aux ?? null,
      endLabels: ctx.options.endLabels !== false,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      settings: lineChartSettings(ctx.options, ctx.following),
    };
  },
};
