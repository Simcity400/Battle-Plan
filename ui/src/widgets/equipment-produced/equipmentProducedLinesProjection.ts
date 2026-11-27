import { tailColour } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { artUrlOf, flagChoiceArt } from '../../families/art/provider.ts';
import { auditRow, type FamilyAuditRow } from '../../families/auditRow.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourceCoverage,
  type SourceEntry,
  type SourcePlan,
} from '../../families/contract.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import type { HolderRun, HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { identityName } from '../../families/table/identityCell.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import {
  seriesLookAt,
  type ChartGap,
  type ChartHoverDetail,
  type TimeChartSeriesModel,
} from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  countrySeriesLooks,
  memberAt,
  missingSpansOf,
  withinMapOf,
  chartTopOf,
} from '../../families/time-chart/seriesHelpers.ts';
import {
  chartAxis,
  chartWindow,
  lineChartSettings,
  type LineChartSettings,
} from '../../families/time-chart/settings.ts';
import { formatCompactScalar, formatDateOnly, formatScalar, notRecordedLabel } from '../../families/valueFormat.ts';
import type {
  EquipmentProducedRaceReply,
  EquipmentProducedRaceRow,
  EquipmentProducedTileFamily,
  ProducedCount,
  ProducedMeasure,
  ProducedTypesReply,
} from '../../generated/dashboard';
import {
  buildExactParams,
  captionFor,
  provenInterval,
  resolvedEquipment,
  UNKNOWN_WORDS,
  validateEquipmentProducedRace,
  validateEquipmentProducedTypes,
  windowUnavailable,
  type ProducedRaceExactBase,
} from './equipmentProducedRaceProjection.ts';

const COLOURED = 8;

export interface EquipmentProducedLinesOptions {
  readonly tags: readonly string[];
  readonly top: ReturnType<typeof chartTopOf>;
  readonly count: ProducedCount;
  readonly measure: ProducedMeasure;
  readonly equipmentLabel: string;
  readonly noDataMessage: string | null;
  readonly fromHour: number | null;
  readonly holders: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly endLabels: boolean;
  readonly settings: LineChartSettings;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
}


function rowKeyOf(row: Pick<EquipmentProducedRaceRow, 'country'>): string {
  return countryKey(row.country);
}

function scaleOf(count: ProducedCount): number {
  return count === 'ic' ? 100_000 : 1;
}

function formatCompactAmount(value: number, count: ProducedCount): string {
  return formatCompactScalar(value, count === 'ic' ? { decimals: 0, maxDecimals: 1 } : { decimals: 0, maxDecimals: 0 }).text;
}

function formatExactAmount(raw: number, count: ProducedCount): string {
  return count === 'ic' ? formatScalar(BigInt(raw), { decimals: 5, maxDecimals: 5 }) : formatScalar(raw, { decimals: 0 });
}

function buildHoverDetail(
  readView: BoundedReadView,
  orderedKeys: readonly string[],
  named: readonly string[],
  options: EquipmentProducedLinesOptions,
  looks: ReadonlyMap<string, ReturnType<typeof countrySeriesLooks> extends ReadonlyMap<string, infer T> ? T : never>,
): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<EquipmentProducedRaceReply>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover || !hover.data) return null;
  const { hour, data: reply } = hover;
  if (reply.missing !== null) {
    return {
      hour,
      replacesSeries: true,
      items: [{ label: options.equipmentLabel, missingReason: reply.missing || notRecordedLabel(), tone: 'muted' }],
    };
  }
  const eligible = orderedKeys.filter(key => memberAt(options.membership, key, hour));
  if (eligible.length === 0) return null;

  const counted = new Map(reply.rows.map(row => [countryKey(row.counted), row]));
  const items: TooltipListItem[] = [];
  const itemsBySeries = new Map<string, TooltipListItem[]>();
  for (const key of eligible) {
    const look = looks.get(key);
    const atHour = look ? seriesLookAt(look, hour) : null;
    const label = atHour ? atHour.name : identityName(readView, 'country', key, hour);
    const who = readView.who('country', key, hour);
    const flagUrl = artUrlOf(flagChoiceArt(readView, who.flagChoice));
    const row = counted.get(key);
    const identity = {
      label,
      ...(atHour ? { swatch: atHour.color } : {}),
      ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
    };
    const total: TooltipListItem = {
      ...identity,
      ...(row?.value != null ? { value: formatExactAmount(row.value, options.count) } : {
        missingReason: !row ? 'This country is not present at this date.' : row.unknown ? UNKNOWN_WORDS[row.unknown] : notRecordedLabel(),
        tone: 'muted',
      }),
    };
    items.push({ ...total, divider: items.length > 0 });
    const detail = row?.breakdown && named.includes(key) ? row.breakdown : undefined;
    const line = [total];
    if (detail && 'models' in detail) {
      for (const model of detail.models) line.push({ label: model.name, value: formatExactAmount(model.value, options.count) });
      if (detail.more > 0) line.push({ label: `And ${detail.more} more`, value: formatExactAmount(detail.remainder, options.count) });
    } else if (detail) {
      line.push({ label: 'Model amounts', missingReason: 'reason' in detail ? UNKNOWN_WORDS[detail.reason] : detail.unavailable, tone: 'muted' });
    }
    itemsBySeries.set(key, line);
  }
  return { hour, items, itemsBySeries, replacesSeries: true };
}

export const equipmentProducedLinesProjection: TimeChartProjection<EquipmentProducedLinesOptions> = registerFamilyProjection({
  kind: 'equipment-produced',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Equipment produced',
  project(readView: BoundedReadView, options: EquipmentProducedLinesOptions) {
    const hour = readView.hour;
    const view = chartWindow(options.settings, hour, options.extent[0]);
    const yAxis = {
      format: 'compact' as const,
      decimals: options.count === 'ic' ? 1 : 0,
      ...chartAxis(options.settings),
    };
    const gaps = options.gaps.length > 0 ? options.gaps : undefined;

    if (options.noDataMessage) {
      return {
        kind: 'equipment-produced',
        measureTitle: options.equipmentLabel,
        mode: 'lines' as const,
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps,
        empty: { kind: 'empty' as const, message: options.noDataMessage },
        auditRows: [],
      };
    }

    const headRefused = readView.refused('head') ?? (options.top !== null ? readView.refused('main') : undefined);
    if (headRefused) {
      return {
        kind: 'equipment-produced',
        measureTitle: options.equipmentLabel,
        mode: 'lines' as const,
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps,
        empty: { kind: 'unavailable' as const, message: headRefused },
        auditRows: [],
      };
    }

    const headPanel = readView.panel<EquipmentProducedRaceReply>('head');
    const mainTiles = readView.series('main');
    if (!headPanel) {
      return {
        kind: 'equipment-produced',
        measureTitle: options.equipmentLabel,
        mode: 'lines' as const,
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps,
        auditRows: [],
      };
    }

    if (headPanel.missing !== null) {
      return {
        kind: 'equipment-produced',
        measureTitle: options.equipmentLabel,
        mode: 'lines' as const,
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps,
        empty: { kind: 'unavailable' as const, message: headPanel.missing },
        warnings: headPanel.warnings.length > 0 ? headPanel.warnings : undefined,
        auditRows: [auditRow('__head__', headPanel.missing)],
      };
    }

    const orderedKeys: string[] = [];
    const seen = new Set<string>();
    const addKey = (key: string) => {
      if (!seen.has(key)) {
        seen.add(key);
        orderedKeys.push(key);
      }
    };

    const headRows = new Map<string, EquipmentProducedRaceRow>();
    for (const row of headPanel.rows) {
      headRows.set(rowKeyOf(row), row);
      headRows.set(row.tag, row);
    }
    for (const row of headPanel.rows) headRows.set(countryKey(row.counted), row);

    if (options.top !== null) {
      for (const row of headPanel.rows) addKey(rowKeyOf(row));
    } else {
      for (const key of options.tags) addKey(key);
    }

    const scale = scaleOf(options.count);
    const seriesList: TimeChartSeriesModel[] = [];
    const auditRows: FamilyAuditRow[] = [];
    const looks = countrySeriesLooks(orderedKeys, readView, {
      holders: options.holders,
      from: options.extent[0],
      surface: options.surface,
      withFlag: true,
      tailColor: tailColour(options.surface),
    });

    for (let i = 0; i < orderedKeys.length; i++) {
      const key = orderedKeys[i]!;
      const row = headRows.get(key);
      const tile = mainTiles?.get(key) ?? null;
      const look = looks.get(key)!;
      const isTail = i >= COLOURED;
      const color = isTail ? tailColour(options.surface) : look.color;

      const rawValue = row?.value ?? null;
      const headValue = rawValue !== null ? rawValue / scale : null;
      const absentReason = headValue !== null
        ? null
        : row?.unknown
          ? UNKNOWN_WORDS[row.unknown]
          : !row
            ? 'No deliveries recorded at this date.'
            : notRecordedLabel();

      seriesList.push({
        id: key,
        key,
        name: look.name,
        color,
        role: isTail ? 'tail' : 'primary',
        step: true,
        bounded: tile,
        headValue,
        headHour: hour,
        flag: look.flag,
        identities: look.identities,
        missingSpans: missingSpansOf(tile),
        absentReason,
        endLabel: !isTail && options.endLabels && headValue !== null
          ? {
              flag: look.flag,
              name: look.name,
              fullName: look.name,
              value: formatCompactAmount(headValue, options.count),
            }
          : null,
      });

      auditRows.push(auditRow(
        key,
        rawValue !== null ? formatExactAmount(rawValue, options.count) : (absentReason ?? notRecordedLabel()),
      ));
    }

    const summaryText = options.fromHour !== null && options.measure === 'total'
      ? `${options.count === 'ic' ? `IC · ${options.equipmentLabel}` : options.equipmentLabel} · since ${formatDateOnly(options.fromHour)}`
      : captionFor(options.count, options.measure, options.equipmentLabel, headPanel.observationHour);

    const named = orderedKeys.slice(0, COLOURED);
    const hoverDetail = buildHoverDetail(readView, orderedKeys, named, options, looks);
    const empty = seriesList.length === 0
      ? {
          kind: (headPanel.readHour !== null ? 'empty' : 'unavailable') as 'empty' | 'unavailable',
          message: headPanel.readHour !== null
            ? (options.measure === 'total'
              ? `No ${options.equipmentLabel.toLowerCase()} delivered yet.`
              : `No ${options.equipmentLabel.toLowerCase()} delivered in this ${options.measure}.`)
            : windowUnavailable(options.measure),
        }
      : undefined;

    return {
      kind: 'equipment-produced',
      measureTitle: options.equipmentLabel,
      mode: 'lines' as const,
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis,
      series: seriesList,
      drawnKeys: { main: orderedKeys, named },
      empty,
      summaryText,
      warnings: headPanel.warnings.length > 0 ? headPanel.warnings : undefined,
      gaps,
      hoverDetail,
      auditRows,
    };
  },
}, 'lines');

export const equipmentProducedLinesDeclaration: TimeChartWidgetDeclaration<EquipmentProducedLinesOptions, ProducedTypesReply> = {
  family: 'time-chart',
  kind: 'equipment-produced',
  label: 'Equipment produced',
  subject: 'equipment produced',
  emptyCampaign: 'Load a campaign to see the equipment it produced.',
  icon: 'layers',
  countries: {
    mode: 'multi',
    content: 'history',
    allowSlots: true,
    everyOnTop: true,
  },
  aux: {
    product: 'equipment-produced-types',
    label: 'the equipment types',
    params: (track, options) => ({ branchId: track.branch.id, refits: options.refits === true }),
    validate: validateEquipmentProducedTypes,
  },
  sources(ctx: FamilyPlanContext<ProducedTypesReply>): SourcePlan {
    const { count, measure, refits, shownTypes, noDataMessage } = resolvedEquipment(ctx);
    const top = chartTopOf(ctx.countries);
    const all = ctx.countries.all;
    const group = top === null && !all ? groupSelection(ctx.widget.countries) : null;
    const tags = top === null ? ctx.countries.tags : [];
    const fromHour = measure === 'total' && ctx.widget.time.mode === 'range' && ctx.staticSpan ? ctx.staticSpan[0] : undefined;
    const waitingForDefault = count === 'units' && !shownTypes.length && !noDataMessage;
    const emptySelection = top === null && !all && !group && tags.length === 0;
    const skip = waitingForDefault || noDataMessage !== null || emptySelection;
    const owner = `equipment-produced:lines:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${top === null ? tags.join(',') : `top${top}`}:${count}:${measure}:${refits ? 1 : 0}:${shownTypes.join(',')}:${fromHour ?? ''}`;
    if (skip) {
      return { owner, entries: [], allowEmptyFollow: true };
    }
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    const within = top === null ? withinMapOf(ctx.countries.membership) : undefined;
    const tileFamily: EquipmentProducedTileFamily = {
      kind: 'equipment-produced-lines',
      count,
      measure,
      types: [...shownTypes],
      refits,
      ...(fromHour !== undefined ? { fromHour } : {}),
    };
    const headCoverage: SourceCoverage = top !== null
      ? top === 'all' ? { mode: 'population', form: 'all' } : { mode: 'population', form: 'top', limit: top }
      : group
        ? { mode: 'population', form: 'all' }
        : { mode: 'keyed', keys: tags };
    const headBase: ProducedRaceExactBase = {
      branchId: ctx.branch.id,
      top: ctx.countries.top,
      all,
      group,
      tags: group ? [] : tags,
      count,
      measure,
      types: shownTypes,
      refits,
      ...(fromHour !== undefined ? { fromHour } : {}),
    };
    const hoverParams: Record<string, unknown> = {
      branchId: ctx.branch.id,
      count,
      measure,
      types: [...shownTypes],
      refits,
      ...(fromHour !== undefined ? { fromHour } : {}),
      ...(top === null ? { countries: tags.map(countryFromKey) } : {}),
    };
    const tileEntry: SourceEntry = {
      name: 'main',
      ...common,
      read: {
        shape: 'pixel-binned',
        coverage: top !== null
          ? top === 'all' ? { mode: 'population', form: 'all' } : { mode: 'population', form: 'top', limit: top }
          : { mode: 'keyed', keys: tags },
        family: tileFamily as unknown as Readonly<Record<string, unknown>>,
        tags,
        ...(within ? { within } : {}),
        ...(top !== null ? { top: { n: top } } : {}),
        headQuery: null,
      },
    };
    const headEntry: SourceEntry = {
      name: 'head',
      ...common,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage: headCoverage,
        product: 'equipment-produced-race',
        exactProduct: 'equipment-produced-race',
        followQuery: null,
        params: headBase as unknown as Readonly<Record<string, unknown>>,
        buildExactParams,
        dynamicKeys: ['hour'],
        provenInterval,
        validate: validateEquipmentProducedRace,
      },
    };
    const hoverEntry: SourceEntry = {
      name: 'hover',
      ...common,
      read: {
        shape: 'exact-hover',
        product: 'equipment-produced-race',
        params: hoverParams,
        countryParamsOf: { models: 'named' },
        ...(top !== null ? { countriesOf: 'main' } : {}),
        ...(top === null && ctx.countries.membership ? { membership: ctx.countries.membership } : {}),
        validate: validateEquipmentProducedRace,
      },
    };
    return { owner, exactEntry: headEntry, entries: [tileEntry, headEntry, hoverEntry] };
  },
  projection: equipmentProducedLinesProjection,
  projectionOptions(ctx, { extent, surface }) {
    const { count, measure, equipmentLabel, noDataMessage } = resolvedEquipment(ctx);
    const top = chartTopOf(ctx.countries);
    const fromHour = measure === 'total' && ctx.widget.time.mode === 'range' && ctx.staticSpan ? ctx.staticSpan[0] : null;
    return {
      tags: top === null ? ctx.countries.tags : [],
      top,
      count,
      measure,
      equipmentLabel,
      noDataMessage,
      fromHour,
      holders: ctx.countries.holders,
      membership: top === null ? ctx.countries.membership : null,
      endLabels: ctx.options.endLabels !== false,
      settings: lineChartSettings(ctx.options, ctx.following),
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
    };
  },
  unavailable: ctx => ctx.countries.reason,
  warnings: (ctx, followRefused) => [
    ...(ctx.aux?.warnings ?? []),
    ...(followRefused ? [followRefused] : []),
  ],
};
