import { tailColour } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import { artUrlOf, flagChoiceArt } from '../../families/art/provider.ts';
import { auditRow, type FamilyAuditRow } from '../../families/auditRow.ts';
import { forceSelection, validateNavalLossesAtReply, type ForceSelection } from '../../families/bars/forceAtRace.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourceEntry,
  type SourcePlan,
} from '../../families/contract.ts';
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
  partialSpansOf,
  withinMapOf,
  chartTopOf,
  rowsBySelectedKey,
} from '../../families/time-chart/seriesHelpers.ts';
import {
  chartAxis,
  chartWindow,
  lineChartSettings,
  type LineChartSettings,
} from '../../families/time-chart/settings.ts';
import { formatCompactScalar, formatScalar, notRecordedLabel } from '../../families/valueFormat.ts';
import type {
  ForceAtRow,
  NavalAttribution,
  NavalKind,
  NavalLossesAtReply,
  NavalLossesTileFamily,
  NavalPeriod,
} from '../../generated/dashboard';
import { NAVAL_NOTE, navalLossesAtProof } from './navalLossesAt.ts';
import { navalAttribution, navalKind, navalMeasure, navalPeriod } from './navalLossesData.ts';

const COLOURED = 8;

export interface NavalLossesLinesOptions {
  readonly tags: readonly string[];
  readonly top: ReturnType<typeof chartTopOf>;
  readonly kind: NavalKind;
  readonly attribution: NavalAttribution;
  readonly period: NavalPeriod;
  readonly measureTitle: string;
  readonly holders: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly endLabels: boolean;
  readonly settings: LineChartSettings;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
}

interface NavalLossesHeadBase {
  readonly branchId: string;
  readonly selection: ForceSelection['params'];
  readonly attribution: NavalAttribution;
  readonly kind: NavalKind;
  readonly period: NavalPeriod;
}

function buildHeadParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> {
  const cfg = base as unknown as NavalLossesHeadBase;
  return { branchId: cfg.branchId, hour, ...cfg.selection, attribution: cfg.attribution, kind: cfg.kind, period: cfg.period };
}


function rowKeyOf(row: Pick<ForceAtRow, 'slot' | 'country'>): string {
  return row.slot != null ? countryKey(row.slot) : countryKey(row.country);
}

function formatCompactCount(value: number): string {
  return formatCompactScalar(value, { decimals: 0, maxDecimals: 0 }).text;
}

function formatExactCount(value: number): string {
  return formatScalar(value, { decimals: 0 });
}

function buildHoverDetail(
  readView: BoundedReadView,
  orderedKeys: readonly string[],
  options: NavalLossesLinesOptions,
  looks: ReadonlyMap<string, ReturnType<typeof countrySeriesLooks> extends ReadonlyMap<string, infer T> ? T : never>,
): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<NavalLossesAtReply>('hover');
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
      items: [{ label: options.measureTitle, missingReason: reply.missing || notRecordedLabel(), tone: 'muted' }],
    };
  }
  const eligible = orderedKeys.filter(key => memberAt(options.membership, key, hour));
  if (eligible.length === 0) return null;

  const rowFor = rowsBySelectedKey(reply.rows, rowKeyOf);
  const items: TooltipListItem[] = [];
  for (const key of eligible) {
    const look = looks.get(key);
    const atHour = look ? seriesLookAt(look, hour) : null;
    const label = atHour ? atHour.name : identityName(readView, 'country', key, hour);
    const who = readView.who('country', key, hour);
    const flagUrl = artUrlOf(flagChoiceArt(readView, who.flagChoice));
    const row = rowFor(key);
    if (!row) {
      items.push({
        label,
        missingReason: 'This country is not present at this date.',
        tone: 'muted',
        ...(atHour ? { swatch: atHour.color } : {}),
        ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
        divider: items.length > 0,
      });
      continue;
    }
    if (row.total === null) {
      items.push({
        label,
        missingReason: row.missing ?? notRecordedLabel(),
        tone: 'muted',
        ...(atHour ? { swatch: atHour.color } : {}),
        ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
        divider: items.length > 0,
      });
    } else {
      items.push({
        label,
        value: formatExactCount(row.total),
        ...(row.partial ? { partialReason: row.partial } : {}),
        strong: true,
        ...(atHour ? { swatch: atHour.color } : {}),
        ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
        divider: items.length > 0,
      });
    }
    if (reply.parts.length > 1 && (row.total !== null || row.values.some(value => value !== null))) {
      reply.parts.forEach((part, index) => {
        const value = row.values[index] ?? null;
        const partialReason = row.partsPartial?.[index];
        items.push({
          label: part.name ?? part.key,
          ...(value === null
            ? { missingReason: row.partsMissing ?? notRecordedLabel(), tone: 'muted' as const }
            : { value: formatExactCount(value), ...(partialReason ? { partialReason } : {}) }),
          indent: true,
        });
      });
    }
  }
  return items.length > 0 ? { hour, replacesSeries: true, items } : null;
}

export const navalLossesLinesProjection: TimeChartProjection<NavalLossesLinesOptions> = registerFamilyProjection({
  kind: 'naval-losses',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Naval losses',
  project(readView: BoundedReadView, options: NavalLossesLinesOptions) {
    const hour = readView.hour;
    const yAxis = {
      format: 'compact' as const,
      decimals: 0,
      ...chartAxis(options.settings),
    };
    const view = chartWindow(options.settings, hour, options.extent[0]);
    const top = options.top !== null;

    const refused = readView.refused('head') ?? (top ? readView.refused('main') : undefined);
    if (refused) {
      return {
        kind: 'naval-losses',
        measureTitle: options.measureTitle,
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        summaryText: NAVAL_NOTE,
        empty: { message: refused, kind: 'unavailable' },
        auditRows: [],
      };
    }

    const headPanel = readView.panel<NavalLossesAtReply>('head');
    const head = headPanel ? validateNavalLossesAtReply(headPanel) : undefined;
    const tiles = readView.series('main');

    if (head && head.missing !== null) {
      return {
        kind: 'naval-losses',
        measureTitle: options.measureTitle,
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        summaryText: NAVAL_NOTE,
        warnings: head.warnings,
        empty: { message: head.missing, kind: 'unavailable' },
        auditRows: [auditRow('__head__', head.missing)],
      };
    }

    const rowFor = rowsBySelectedKey(head?.rows ?? [], rowKeyOf);
    const orderedKeys: string[] = [];
    const seen = new Set<string>();
    const addKey = (key: string) => {
      if (!seen.has(key)) {
        seen.add(key);
        orderedKeys.push(key);
      }
    };
    if (top) {
      if (head) {
        for (const row of head.rows) addKey(rowKeyOf(row));
      } else {
        for (const key of tiles?.keys() ?? []) addKey(key);
      }
    } else {
      for (const key of options.tags) addKey(key);
    }

    const looks = countrySeriesLooks(orderedKeys, readView, {
      holders: options.holders,
      from: options.extent[0],
      surface: options.surface,
      withFlag: true,
      tailColor: tailColour(options.surface),
    });

    const hoverDetail = buildHoverDetail(readView, orderedKeys, options, looks);
    const auditRows: FamilyAuditRow[] = [];
    const series: TimeChartSeriesModel[] = orderedKeys.map((key, rank) => {
      const isTail = rank >= COLOURED;
      const look = looks.get(key)!;
      const row = rowFor(key);
      const present = memberAt(options.membership, key, hour);
      const bounded = tiles?.get(key) ?? null;
      const headValue = present && row && row.total !== null ? row.total : null;
      const partial = partialSpansOf(bounded);
      const headPartial = present && row?.partial;
      const partialSpans = headPartial && !partial?.some(span => span.firstHour <= hour && span.lastHour >= hour)
        ? [...(partial ?? []), { firstHour: hour, lastHour: hour, reason: headPartial }]
        : partial;
      const absentReason = present && row && row.total === null
        ? (row.missing ?? notRecordedLabel())
        : present && head && !row
          ? 'This country is not present at this date.'
          : null;

      if (present && row) {
        const count = row.total === null ? (row.missing ?? notRecordedLabel()) : formatCompactCount(row.total);
        auditRows.push(auditRow(key, row.partial ? `${count}, partial: ${row.partial}` : count));
      } else if (present && head) {
        auditRows.push(auditRow(key, 'This country is not present at this date.'));
      }

      return {
        id: key,
        key,
        name: look.name,
        color: look.color,
        role: isTail ? 'tail' : 'primary',
        step: true,
        bounded,
        headValue,
        headHour: hour,
        absentReason,
        flag: look.flag,
        identities: look.identities,
        missingSpans: missingSpansOf(bounded),
        partialSpans,
        endLabel: !isTail && options.endLabels && headValue !== null
          ? { flag: look.flag, name: look.name, fullName: look.name, value: `${formatCompactCount(headValue)}${row?.partial ? ' (partial)' : ''}` }
          : null,
      };
    });

    if (series.length === 0 && head) {
      return {
        kind: 'naval-losses',
        measureTitle: options.measureTitle,
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        hoverDetail,
        summaryText: NAVAL_NOTE,
        warnings: head.warnings,
        empty: {
          message: options.period === 'month' ? 'No known naval losses this month.' : 'No known naval losses yet.',
          kind: 'empty',
        },
        auditRows,
      };
    }

    return {
      kind: 'naval-losses',
      measureTitle: options.measureTitle,
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis,
      series,
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      hoverDetail,
      summaryText: NAVAL_NOTE,
      ...(head?.warnings.length ? { warnings: head.warnings } : {}),
      auditRows,
    };
  },
}, 'lines');

export const navalLossesLinesDeclaration: TimeChartWidgetDeclaration<NavalLossesLinesOptions> = {
  family: 'time-chart',
  kind: 'naval-losses',
  label: 'Naval losses',
  subject: 'naval losses',
  emptyCampaign: 'Load a campaign to see its naval losses.',
  icon: 'shield',
  countries: {
    mode: 'multi',
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const kind = navalKind(ctx.options);
    const attribution = navalAttribution(ctx.options);
    const period = navalPeriod(ctx.options);
    const top = chartTopOf(ctx.countries);
    const tags = top === null ? ctx.countries.tags : [];
    const selection = forceSelection(ctx);
    const owner = `naval-losses:lines:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${top === null ? tags.join(',') : `top${top}`}:${kind}:${attribution}:${period}`;
    if (selection.none || (top === null && tags.length === 0)) {
      return { owner, entries: [], allowEmptyFollow: true };
    }
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    const within = top === null ? withinMapOf(ctx.countries.membership) : undefined;
    const tileFamily: NavalLossesTileFamily = {
      kind: 'naval-losses-lines',
      countKind: kind,
      attribution,
      period,
    };
    const headBase: NavalLossesHeadBase = {
      branchId: ctx.branch.id,
      selection: selection.params,
      attribution,
      kind,
      period,
    };
    const hoverParams: Record<string, unknown> = {
      branchId: ctx.branch.id,
      attribution,
      kind,
      period,
      ...(top === null ? { countries: tags.map(countryFromKey) } : {}),
    };
    const entries: SourceEntry[] = [
      {
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
      },
      {
        name: 'head',
        ...common,
        read: {
          shape: 'point',
          channel: 'followed',
          coverage: selection.coverage,
          product: 'naval-losses-at',
          exactProduct: 'naval-losses-at',
          followQuery: null,
          params: headBase as unknown as Readonly<Record<string, unknown>>,
          buildExactParams: buildHeadParams,
          provenInterval: navalLossesAtProof,
          dynamicKeys: ['hour', 'countries'],
          validate: validateNavalLossesAtReply,
        },
      },
      {
        name: 'hover',
        ...common,
        read: {
          shape: 'exact-hover',
          product: 'naval-losses-at',
          params: hoverParams,
          ...(top !== null ? { countriesOf: 'main' } : {}),
          ...(top === null && ctx.countries.membership ? { membership: ctx.countries.membership } : {}),
          validate: validateNavalLossesAtReply,
        },
      },
    ];
    return { owner, exactEntry: entries[1]!, entries };
  },
  projection: navalLossesLinesProjection,
  projectionOptions(ctx, { extent, surface }) {
    const kind = navalKind(ctx.options);
    const attribution = navalAttribution(ctx.options);
    const period = navalPeriod(ctx.options);
    const top = chartTopOf(ctx.countries);
    return {
      tags: top === null ? ctx.countries.tags : [],
      top,
      kind,
      attribution,
      period,
      measureTitle: navalMeasure({ kind, attribution }),
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
};
