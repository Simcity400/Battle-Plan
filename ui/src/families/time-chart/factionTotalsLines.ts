import { CATEGORY_LIMIT, CHART_MEASURE_COLOURS, colour } from '../../charts/palette.ts';
import type { ChartSummary } from '../../charts/chartSummary';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { CELL, cellState, cellValue, rowReason } from '../../dashboard/follow/message.ts';
import { validateFactionLines, type FactionLines } from '../../dashboard/territoryReplies.ts';
import { auditRow, type FamilyAuditRow } from '../auditRow.ts';
import {
  factionSelectionTop,
  groupTitleForSelection,
  isAggregatedSelection,
  validateFactionTotalsAt,
  type FactionTotalsKind,
} from '../bars/factionTotalsRace.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourceCoverage,
  type SourceEntry,
  type SourcePlan,
} from '../contract.ts';
import { groupSelection } from '../groupSelection.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { resolveName } from '../table/identityCell.ts';
import { formatScalar } from '../valueFormat.ts';
import type { FactionTotalRow, FactionTotalsAtReply } from '../../generated/dashboard';
import type {
  ChartGap,
  ChartHoverDetail,
  ChartInlinePoint,
  ChartMissingSpan,
  TimeChartModel,
  TimeChartSeriesModel,
} from './model.ts';
import { branchGapsOf, headReading, missingSpansOf, type HeadReading } from './seriesHelpers.ts';
import { chartAxis, chartWindow, lineChartSettings, type LineChartSettings } from './settings.ts';

const COLOURED = 8;
const TAIL_COLOUR = CHART_MEASURE_COLOURS.tailLine;

export interface FactionSpanSummary {
  readonly id: string;
  readonly firstHour: number;
  readonly spans: readonly (readonly [number, number])[];
}

export interface FactionTotalsLinesOptions {
  readonly kind: string;
  readonly seriesKind: FactionTotalsKind;
  readonly measure: string;
  readonly measureLabel: string;
  readonly note?: string;
  readonly aggregated: boolean;
  readonly directTiles: boolean;
  readonly top: number | null;
  readonly factions: readonly FactionSpanSummary[];
  readonly groupLabel: string;
  readonly endLabels: boolean;
  readonly settings: LineChartSettings;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
}

function stableColourIndex(key: string): number {
  let hash = 2166136261;
  for (let i = 0; i < key.length; i++) {
    hash ^= key.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0) % CATEGORY_LIMIT;
}

function membershipOf(ctx: FamilyPlanContext): 'hour' | 'ever' {
  const selection = ctx.widget.countries;
  if ((selection.kind === 'factions' || selection.kind === 'group') && selection.membership === 'ever') {
    return 'ever';
  }
  return 'hour';
}

function factionSpansFor(ctx: FamilyPlanContext): readonly FactionSpanSummary[] {
  const selection = ctx.widget.countries;
  const byId = new Map<string, [number, number][]>();
  for (const run of ctx.countries.factions ?? []) {
    const list = byId.get(run.id) ?? [];
    list.push([run.firstHour, run.lastHour]);
    byId.set(run.id, list);
  }
  const merge = (raw: readonly [number, number][]): readonly (readonly [number, number])[] => {
    const sorted = [...raw].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    const out: [number, number][] = [];
    for (const [first, last] of sorted) {
      const prev = out.at(-1);
      if (prev && first <= prev[1] + 1) prev[1] = Math.max(prev[1], last);
      else out.push([first, last]);
    }
    return out;
  };
  if (selection.kind === 'group' && selection.aggregate && selection.group.type === 'faction') {
    const id = selection.group.id;
    const merged = merge(byId.get(id) ?? []);
    return [{ id, firstHour: merged[0]?.[0] ?? ctx.branch.firstHour, spans: merged }];
  }
  return [...byId.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([id, raw]) => {
      const merged = merge(raw);
      return { id, firstHour: merged[0]?.[0] ?? ctx.branch.firstHour, spans: merged };
    });
}

function usesDirectFactionTiles(ctx: FamilyPlanContext): boolean {
  if (membershipOf(ctx) !== 'hour') return false;
  const selection = ctx.widget.countries;
  if (selection.kind === 'factions') return true;
  return selection.kind === 'group' && selection.aggregate && selection.group.type === 'faction';
}

function activeAt(spans: readonly (readonly [number, number])[], hour: number): boolean {
  if (!spans.length) return true;
  for (const [first, last] of spans) {
    if (first <= hour && hour <= last) return true;
  }
  return false;
}

function clipMissingSpans(
  missing: readonly ChartMissingSpan[] | undefined,
  spans: readonly (readonly [number, number])[],
): readonly ChartMissingSpan[] | undefined {
  if (!missing || missing.length === 0 || spans.length === 0) return missing;
  const clipped: ChartMissingSpan[] = [];
  for (const gap of missing) {
    for (const [first, last] of spans) {
      const start = Math.max(gap.firstHour, first);
      const end = Math.min(gap.lastHour, last);
      if (start <= end) clipped.push({ firstHour: start, lastHour: end, reason: gap.reason });
    }
  }
  return clipped.length > 0 ? clipped : undefined;
}

function summaryPoints(summary: ChartSummary, bounds: boolean, maxHour: number): ChartInlinePoint[] {
  const points: ChartInlinePoint[] = [];
  let prevLast: number | null = null;
  for (const bucket of summary.buckets) {
    if (!bucket || bucket.firstHour > maxHour) {
      if (prevLast !== null) {
        points.push({ hour: prevLast + 1, value: null });
        prevLast = null;
      }
      continue;
    }
    if (prevLast !== null && (bucket.firstBreak || bucket.firstHour > prevLast + 1)) {
      points.push({ hour: prevLast + 1, value: null });
    }
    const lastHour = Math.min(bucket.lastHour, maxHour);
    if (bucket.firstHour === lastHour) {
      points.push({ hour: bucket.firstHour, value: bucket.lastValue });
    } else {
      points.push({
        hour: bucket.firstHour,
        value: bucket.firstValue,
        min: bucket.minValue,
        max: bucket.maxValue,
        approximate: bounds,
      });
      points.push({
        hour: lastHour,
        value: bucket.lastValue,
        min: bucket.minValue,
        max: bucket.maxValue,
        approximate: bounds,
      });
    }
    prevLast = lastHour;
  }
  return points;
}

function latestPointValue(points: readonly ChartInlinePoint[]): number | null {
  for (let i = points.length - 1; i >= 0; i--) {
    const v = points[i]!.value;
    if (v !== null && Number.isFinite(v)) return v;
  }
  return null;
}

function factionPresentation(
  readView: BoundedReadView,
  id: string,
  aggregated: boolean,
  groupLabel: string,
  surface: string,
  atHour: number,
): { readonly name: string; readonly color: string } {
  if (aggregated) {
    return { name: groupLabel, color: colour(surface, 0) };
  }
  const resolved = resolveName(readView, 'faction', id, { atHour });
  const name = resolved.value.kind === 'known' || resolved.loading ? resolved.name : `Faction ${id}`;
  const color = resolved.who.colour?.status === 'ready'
    ? resolved.who.colour.value
    : colour(surface, stableColourIndex(id));
  return { name, color };
}

function memberNames(readView: BoundedReadView, row: FactionTotalRow, atHour: number): string {
  const limit = 8;
  const keys = row.memberCountries.length ? row.memberCountries.map(countryKey) : row.members;
  const shown = keys.slice(0, limit).map((key, index) => {
    const resolved = resolveName(readView, 'country', key, { atHour });
    return resolved.value.kind === 'known' || resolved.loading ? resolved.name : (row.members[index] ?? key);
  });
  return keys.length > limit ? `${shown.join(', ')} +${keys.length - limit} more` : shown.join(', ');
}

function buildHoverDetail(
  readView: BoundedReadView,
  options: FactionTotalsLinesOptions,
): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<FactionTotalsAtReply>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover) return null;
  const reply = hover.data;
  if (reply.missing !== null) {
    return {
      hour: hover.hour,
      replacesSeries: true,
      items: [{ label: options.measureLabel, missingReason: reply.missing, tone: 'muted' }],
    };
  }
  if (!reply.rows.length) {
    return {
      hour: hover.hour,
      replacesSeries: true,
      items: [{ label: options.aggregated ? 'No members at this date' : 'No factions at this date', tone: 'muted' }],
    };
  }
  const items: TooltipListItem[] = [];
  for (const row of reply.rows) {
    const pres = factionPresentation(readView, row.id, options.aggregated, options.groupLabel, options.surface, hover.hour);
    items.push(
      row.total === null
        ? { label: pres.name, swatch: pres.color, missingReason: row.missing ?? 'Not recorded at this hour', tone: 'muted', divider: items.length > 0 }
        : { label: pres.name, swatch: pres.color, value: formatScalar(row.total, 'integer'), strong: true, divider: items.length > 0 },
    );
    if (row.members.length > 0) {
      items.push({ label: 'Members', value: memberNames(readView, row, hover.hour), indent: true });
    }
  }
  return { hour: hover.hour, replacesSeries: true, items };
}

function factionHeadReading(readView: BoundedReadView, id: string, rowIndex: number): HeadReading | null {
  const fromHelper = headReading(readView, 'main', { tag: id }, 0);
  if (fromHelper !== null) return fromHelper;
  const answer = readView.answer('main');
  if (!answer) return null;
  if (answer.kind === 'notRecorded') return { missing: answer.reason };
  if (rowIndex < 0 || rowIndex >= answer.rows) return null;
  const at = rowIndex * answer.columns;
  if (cellState(answer, at) !== CELL.value) {
    return { missing: rowReason(answer, rowIndex) ?? 'Not recorded at this hour' };
  }
  const raw = cellValue(answer, at);
  return { value: Number(raw), raw, denominator: 1n };
}

export function factionTotalsLinesSources(
  ownerPrefix: string,
  ctx: FamilyPlanContext,
  kind: FactionTotalsKind,
  measure: string,
): SourcePlan {
  const aggregated = isAggregatedSelection(ctx.widget.countries);
  const group = aggregated ? groupSelection(ctx.widget.countries) : null;
  const top = factionSelectionTop(ctx.widget.countries);
  const membership = membershipOf(ctx);
  const members = aggregated && !group ? ctx.countries.tags.map(countryFromKey) : undefined;
  const emptyAgg = aggregated && !group && (!members || !members.length);
  const atParams: Record<string, unknown> = {
    branchId: ctx.branch.id,
    kind,
    measure,
    ...(top != null ? { top } : {}),
    ...(group ? { group } : {}),
    ...(members ? { members } : {}),
  };
  const coverage: SourceCoverage = group
    ? { mode: 'population', form: 'all' }
    : aggregated
      ? { mode: 'keyed', keys: ctx.countries.tags }
      : top != null
        ? { mode: 'population', form: 'top', limit: top }
        : { mode: 'population', form: 'all' };
  const ownerScope = group
    ? `group:${group.type}:${group.id ?? group.tag ?? ''}:${group.membership ?? 'hour'}`
    : aggregated
      ? 'members'
      : `factions:${membership}:${top ?? 'all'}`;
  const owner = `${ownerPrefix}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${kind}:${measure}:${ownerScope}`;
  if (emptyAgg) return { owner, entries: [], allowEmptyFollow: true };

  const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
  const hoverEntry: SourceEntry = {
    name: 'hover',
    ...common,
    read: {
      shape: 'exact-hover',
      product: 'faction-totals-at',
      params: atParams,
      validate: validateFactionTotalsAt,
    },
  };

  if (usesDirectFactionTiles(ctx)) {
    const ids = factionSpansFor(ctx).map(item => item.id);
    if (!ids.length) return { owner, entries: [], allowEmptyFollow: true };
    const family = { kind: 'faction-totals', measure };
    const reads = ids.map(id => ({ country: id }));
    return {
      owner,
      entries: [
        {
          name: 'main',
          ...common,
          read: {
            shape: 'pixel-binned',
            coverage: { mode: 'keyed', keys: ids },
            family,
            tags: ids,
            headQuery: { product: 'series-state', coverage: 'row', params: { branchId: ctx.branch.id, family, reads } },
            exactProduct: 'series-state',
          },
        },
        hoverEntry,
      ],
    };
  }

  const groupMembers = aggregated
    ? ctx.countries.tags.map(tag => ({
        tag,
        spans: ctx.countries.membership?.[tag]?.map(([first, last]) => [first, last] as [number, number]) ?? null,
      }))
    : null;
  const unread = aggregated
    ? ctx.countries.unobserved.map(([first, last]) => [first, last] as [number, number])
    : null;
  return {
    owner,
    entries: [
      {
        name: 'lines',
        ...common,
        read: {
          shape: 'branch-to-hour',
          coverage,
          product: 'faction-lines',
          followQuery: null,
          params: {
            branchId: ctx.branch.id,
            kind,
            measure,
            ...(groupMembers ? { members: groupMembers, unread } : { membership }),
          },
          buildSpanParams: (base, span) => ({
            ...base,
            from: span.firstHour,
            to: span.lastHour,
            width: 512,
          }),
          dynamicKeys: ['from', 'to'],
          validate: validateFactionLines,
        },
      },
      hoverEntry,
    ],
  };
}

export function factionTotalsLinesOptions(
  kind: string,
  ctx: FamilyPlanContext,
  spec: { readonly seriesKind: FactionTotalsKind; readonly measure: string; readonly measureLabel: string; readonly note?: string },
  extra: { readonly extent: readonly [number, number]; readonly surface: string },
): FactionTotalsLinesOptions {
  return {
    kind,
    seriesKind: spec.seriesKind,
    measure: spec.measure,
    measureLabel: spec.measureLabel,
    ...(spec.note ? { note: spec.note } : {}),
    aggregated: isAggregatedSelection(ctx.widget.countries),
    directTiles: usesDirectFactionTiles(ctx),
    top: factionSelectionTop(ctx.widget.countries),
    factions: usesDirectFactionTiles(ctx) ? factionSpansFor(ctx) : [],
    groupLabel: groupTitleForSelection(ctx.widget.countries),
    endLabels: ctx.options.endLabels !== false,
    settings: lineChartSettings(ctx.options),
    extent: extra.extent,
    gaps: branchGapsOf(ctx.branch),
    surface: extra.surface,
  };
}

function projectDirectTiles(
  readView: BoundedReadView,
  options: FactionTotalsLinesOptions,
  yAxis: TimeChartModel['yAxis'],
  view: TimeChartModel['view'],
  gaps: TimeChartModel['gaps'],
  hoverDetail: ChartHoverDetail | null,
): TimeChartModel {
  const hour = readView.hour;
  const refused = readView.refused('main');
  if (refused) {
    return {
      kind: options.kind,
      measureTitle: options.measureLabel,
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis,
      series: [],
      gaps,
      ...(options.note ? { note: options.note } : {}),
      empty: { message: refused, kind: 'unavailable' },
      auditRows: [],
    };
  }

  const tiles = readView.series('main');
  const candidates = options.factions.flatMap((faction, rowIndex) => {
    if (faction.firstHour > hour) return [];
    const active = activeAt(faction.spans, hour);
    const reading = factionHeadReading(readView, faction.id, rowIndex);
    const known = reading && 'value' in reading ? reading.value : null;
    const missing = active && reading && 'missing' in reading ? reading.missing : null;
    return [{ faction, rowIndex, active, known, missing, bounded: tiles?.get(faction.id) ?? null }];
  });

  if (options.top !== null) {
    const activeNow = candidates.filter(item => item.active);
    if (activeNow.some(item => item.known === null && item.missing !== null)) {
      return {
        kind: options.kind,
        measureTitle: options.measureLabel,
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps,
        hoverDetail,
        ...(options.note ? { note: options.note } : {}),
        empty: {
          message: `The top ${options.top} factions cannot be proved: a faction's total is not known at this hour.`,
          kind: 'unavailable',
        },
        auditRows: [],
      };
    }
  }

  candidates.sort((a, b) => {
    if (a.active !== b.active) return a.active ? -1 : 1;
    if (a.known !== null && b.known !== null && a.known !== b.known) return b.known - a.known;
    if (a.known !== null && b.known === null) return -1;
    if (a.known === null && b.known !== null) return 1;
    return a.faction.id.localeCompare(b.faction.id);
  });

  const shown = options.top !== null ? candidates.slice(0, options.top) : candidates;
  const auditRows: FamilyAuditRow[] = [];
  const series: TimeChartSeriesModel[] = shown.map((item, rank) => {
    const isTail = rank >= COLOURED;
    const pres = factionPresentation(readView, item.faction.id, options.aggregated, options.groupLabel, options.surface, hour);
    const color = isTail ? TAIL_COLOUR : pres.color;
    const headValue = item.active ? item.known : null;
    const absentReason = item.active ? item.missing : null;
    if (item.active) {
      auditRows.push(
        auditRow(
          item.faction.id,
          headValue === null ? (absentReason ?? 'Not recorded at this hour') : formatScalar(headValue, 'integer'),
        ),
      );
    }
    return {
      id: item.faction.id,
      key: item.faction.id,
      name: pres.name,
      color,
      role: isTail ? 'tail' : 'primary',
      step: true,
      ...(item.bounded ? { bounded: item.bounded } : {}),
      headValue,
      headHour: hour,
      absentReason,
      missingSpans: clipMissingSpans(missingSpansOf(item.bounded), item.faction.spans),
      endLabel: isTail || headValue === null || !options.endLabels
        ? null
        : {
            colorBar: color,
            name: pres.name,
            fullName: pres.name,
            value: formatScalar(headValue, 'integer'),
          },
    };
  });

  if (series.length === 0) {
    return {
      kind: options.kind,
      measureTitle: options.measureLabel,
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis,
      series: [],
      gaps,
      hoverDetail,
      ...(options.note ? { note: options.note } : {}),
      empty: {
        message: options.aggregated ? 'No members of this group at this date.' : 'No factions at this date.',
        kind: 'empty',
      },
      auditRows,
    };
  }

  return {
    kind: options.kind,
    measureTitle: options.measureLabel,
    mode: 'lines',
    hour,
    extent: options.extent,
    ...(view ? { view } : {}),
    yAxis,
    series,
    gaps,
    hoverDetail,
    ...(options.note ? { note: options.note } : {}),
    auditRows,
  };
}

export function projectFactionTotalsLines(readView: BoundedReadView, options: FactionTotalsLinesOptions): TimeChartModel {
  const hour = readView.hour;
  const view = chartWindow(options.settings, hour, options.extent[0]);
  const yAxis = { format: 'number' as const, decimals: 0, ...chartAxis(options.settings) };
  const gaps = options.gaps.length > 0 ? options.gaps : undefined;
  const hoverDetail = buildHoverDetail(readView, options);

  if (options.directTiles) {
    return projectDirectTiles(readView, options, yAxis, view, gaps, hoverDetail);
  }

  const refused = readView.refused('lines');
  if (refused) {
    return {
      kind: options.kind,
      measureTitle: options.measureLabel,
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis,
      series: [],
      gaps,
      ...(options.note ? { note: options.note } : {}),
      empty: { message: refused, kind: 'unavailable' },
      auditRows: [],
    };
  }

  const linesPanel = readView.panel<FactionLines>('lines');
  if (linesPanel === undefined) {
    return {
      kind: options.kind,
      measureTitle: options.measureLabel,
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis,
      series: [],
      gaps,
      ...(options.note ? { note: options.note } : {}),
      auditRows: [],
    };
  }

  const rows = linesPanel.lines.map(line => {
    const points = summaryPoints(line.summary, linesPanel.bounds === true, hour);
    const last = latestPointValue(points);
    return { line, points, last };
  });
  rows.sort((a, b) => {
    if (a.last !== null && b.last !== null && a.last !== b.last) return b.last - a.last;
    if (a.last !== null && b.last === null) return -1;
    if (a.last === null && b.last !== null) return 1;
    return a.line.id.localeCompare(b.line.id);
  });
  const shown = options.top !== null ? rows.slice(0, options.top) : rows;
  const auditRows: FamilyAuditRow[] = [];
  const series: TimeChartSeriesModel[] = shown.map((item, rank) => {
    const isTail = rank >= COLOURED;
    const pres = factionPresentation(readView, item.line.id, options.aggregated, options.groupLabel, options.surface, hour);
    const color = isTail ? TAIL_COLOUR : pres.color;
    auditRows.push(
      auditRow(
        item.line.id,
        item.last === null ? 'Not recorded at this hour' : formatScalar(item.last, 'integer'),
      ),
    );
    return {
      id: item.line.id,
      key: item.line.id,
      name: pres.name,
      color,
      role: isTail ? 'tail' : 'primary',
      step: true,
      points: item.points,
      headValue: item.last,
      headHour: hour,
      missingSpans: item.line.missing,
      endLabel: isTail || item.last === null || !options.endLabels
        ? null
        : {
            colorBar: color,
            name: pres.name,
            fullName: pres.name,
            value: formatScalar(item.last, 'integer'),
          },
    };
  });

  if (series.length === 0) {
    return {
      kind: options.kind,
      measureTitle: options.measureLabel,
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis,
      series: [],
      gaps,
      hoverDetail,
      ...(options.note ? { note: options.note } : {}),
      warnings: linesPanel.warnings,
      empty: {
        message: options.aggregated ? 'No members of this group at this date.' : 'No factions at this date.',
        kind: 'empty',
      },
      auditRows,
    };
  }

  return {
    kind: options.kind,
    measureTitle: options.measureLabel,
    mode: 'lines',
    hour,
    extent: options.extent,
    ...(view ? { view } : {}),
    yAxis,
    series,
    gaps,
    hoverDetail,
    ...(options.note ? { note: options.note } : {}),
    warnings: linesPanel.warnings,
    auditRows,
  };
}
