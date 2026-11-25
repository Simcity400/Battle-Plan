import { CATEGORY_LIMIT, CHART_MEASURE_COLOURS, colour } from '../../charts/palette.ts';
import { factionIdentity, type FactionStyle } from '../../dashboard/campaignSettings.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { CELL, cellState, cellValue, rowReason } from '../../dashboard/follow/message.ts';
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
import type { TimeChartWidgetDeclaration } from './declaration.ts';
import type {
  ChartGap,
  ChartHoverDetail,
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
  readonly founder: string | null;
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
  readonly styles?: Readonly<Record<string, FactionStyle>>;
  readonly shared?: Readonly<Record<string, { readonly colour: string }>>;
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
  const byId = new Map<string, { spans: [number, number][]; founder: string | null; founderHour: number }>();
  for (const run of ctx.countries.factions ?? []) {
    const entry = byId.get(run.id) ?? { spans: [], founder: null, founderHour: Infinity };
    entry.spans.push([run.firstHour, run.lastHour]);
    if (run.tag && run.firstHour < entry.founderHour) {
      entry.founder = run.tag;
      entry.founderHour = run.firstHour;
    }
    byId.set(run.id, entry);
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
    const hit = byId.get(id);
    const merged = merge(hit?.spans ?? []);
    return [{ id, founder: hit?.founder ?? null, firstHour: merged[0]?.[0] ?? ctx.branch.firstHour, spans: merged }];
  }
  return [...byId.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([id, entry]) => {
      const merged = merge(entry.spans);
      return { id, founder: entry.founder, firstHour: merged[0]?.[0] ?? ctx.branch.firstHour, spans: merged };
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

function sharedFactionColour(
  shared: FactionTotalsLinesOptions['shared'],
  resolvedName: string | null,
  founder: string | null,
): string | undefined {
  if (!shared) return undefined;
  const byName = resolvedName ? factionIdentity(resolvedName, null) : null;
  if (byName && shared[byName]?.colour) return shared[byName].colour;
  const byFounder = founder ? factionIdentity(null, founder) : null;
  return byFounder ? shared[byFounder]?.colour : undefined;
}

function factionPresentation(
  readView: BoundedReadView,
  id: string,
  options: Pick<FactionTotalsLinesOptions, 'aggregated' | 'groupLabel' | 'surface' | 'styles' | 'shared' | 'factions'>,
  atHour: number,
  founder?: string | null,
): { readonly name: string; readonly color: string; readonly customColour: boolean } {
  if (options.aggregated) {
    return { name: options.groupLabel, color: colour(options.surface, 0), customColour: false };
  }
  const resolved = resolveName(readView, 'faction', id, { atHour });
  const style = options.styles?.[id];
  const recordedName = resolved.value.kind === 'known' ? resolved.name : null;
  const name = style?.name ?? (resolved.value.kind === 'known' || resolved.loading ? resolved.name : `Faction ${id}`);
  const founderTag = founder ?? options.factions.find(item => item.id === id)?.founder ?? null;
  const ownColour = style?.colour ?? sharedFactionColour(options.shared, recordedName, founderTag);
  const color = ownColour
    ?? (resolved.who.colour?.status === 'ready'
      ? resolved.who.colour.value
      : colour(options.surface, stableColourIndex(id)));
  return { name, color, customColour: ownColour !== undefined };
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
    const pres = factionPresentation(readView, row.id, options, hover.hour);
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
    ...(!aggregated && membership === 'ever' ? { membership: 'ever' } : {}),
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
      ? `members:${ctx.countries.tags.join(',')}`
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

  const tileTags = aggregated ? ['group'] : factionSpansFor(ctx).map(item => item.id);
  const tileFamily: Record<string, unknown> = {
    kind: 'faction-group-lines',
    sourceKind: kind,
    measure,
    ...(group ? { group } : members ? { members } : { membership }),
  };
  const entries: SourceEntry[] = [];
  if (tileTags.length > 0) {
    entries.push({
      name: 'main',
      ...common,
      read: {
        shape: 'pixel-binned',
        coverage: group ? { mode: 'population', form: 'all' } : { mode: 'keyed', keys: tileTags },
        family: tileFamily,
        tags: tileTags,
      },
    });
  }
  entries.push(
    {
      name: 'head',
      ...common,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage,
        product: 'faction-totals-at',
        followQuery: { panel: 'faction-totals-at', ...atParams },
        params: atParams,
        validate: validateFactionTotalsAt,
      },
    },
    hoverEntry,
  );
  return { owner, entries };
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
    factions: factionSpansFor(ctx),
    groupLabel: groupTitleForSelection(ctx.widget.countries),
    endLabels: ctx.options.endLabels !== false,
    settings: lineChartSettings(ctx.options),
    extent: extra.extent,
    gaps: branchGapsOf(ctx.branch),
    surface: extra.surface,
  };
}

export function withFactionTotalsStyles(
  declaration: TimeChartWidgetDeclaration<FactionTotalsLinesOptions>,
  styles: FactionTotalsLinesOptions['styles'],
  shared: FactionTotalsLinesOptions['shared'],
): TimeChartWidgetDeclaration<FactionTotalsLinesOptions> {
  return {
    ...declaration,
    projectionOptions: (ctx, extra) => ({ ...declaration.projectionOptions(ctx, extra), styles, shared }),
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
    const pres = factionPresentation(readView, item.faction.id, options, hour, item.faction.founder);
    const color = isTail && !pres.customColour ? TAIL_COLOUR : pres.color;
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

  const refused = readView.refused('head') ?? readView.refused('main');
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

  const headPanel = readView.panel<FactionTotalsAtReply>('head');
  if (headPanel === undefined) {
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

  const tiles = readView.series('main');
  if (headPanel.missing !== null) {
    if (options.top !== null || !tiles || tiles.size === 0) {
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
        warnings: headPanel.warnings,
        empty: { message: headPanel.missing, kind: 'unavailable' },
        auditRows: [],
      };
    }
    const ids = options.aggregated ? ['group'] : [...tiles.keys()];
    const auditRows: FamilyAuditRow[] = [];
    const series: TimeChartSeriesModel[] = ids.map((id, rank) => {
      const isTail = rank >= COLOURED;
      const pres = factionPresentation(readView, id, options, hour);
      const color = isTail && !pres.customColour ? TAIL_COLOUR : pres.color;
      const bounded = tiles.get(id) ?? null;
      auditRows.push(auditRow(id, headPanel.missing!));
      return {
        id,
        key: id,
        name: pres.name,
        color,
        role: isTail ? 'tail' : 'primary',
        step: true,
        ...(bounded ? { bounded } : {}),
        headValue: null,
        headHour: hour,
        absentReason: headPanel.missing,
        missingSpans: missingSpansOf(bounded),
        endLabel: null,
      };
    });
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
      warnings: headPanel.warnings,
      auditRows,
    };
  }

  const shown = options.top !== null ? headPanel.rows.slice(0, options.top) : headPanel.rows;
  const auditRows: FamilyAuditRow[] = [];
  const series: TimeChartSeriesModel[] = shown.map((row, rank) => {
    const isTail = rank >= COLOURED;
    const pres = factionPresentation(readView, row.id, options, hour);
    const color = isTail && !pres.customColour ? TAIL_COLOUR : pres.color;
    const bounded = tiles?.get(row.id) ?? null;
    const headValue = row.total;
    const absentReason = headValue === null ? (row.missing ?? 'Not recorded at this hour') : null;
    auditRows.push(
      auditRow(
        row.id,
        headValue === null ? absentReason! : formatScalar(headValue, 'integer'),
      ),
    );
    return {
      id: row.id,
      key: row.id,
      name: pres.name,
      color,
      role: isTail ? 'tail' : 'primary',
      step: true,
      ...(bounded ? { bounded } : {}),
      headValue,
      headHour: hour,
      absentReason,
      missingSpans: missingSpansOf(bounded),
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
      warnings: headPanel.warnings,
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
    warnings: headPanel.warnings,
    auditRows,
  };
}
