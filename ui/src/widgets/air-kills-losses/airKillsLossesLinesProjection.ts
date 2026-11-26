import { SECONDARY_DASH, tailColour } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet, type OptionValue } from '../../dashboard/options.ts';
import { artUrlOf, flagChoiceArt } from '../../families/art/provider.ts';
import { auditRow, type FamilyAuditRow } from '../../families/auditRow.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
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
  type ChartLegendEntry,
  type TimeChartSeriesModel,
} from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  countrySeriesLooks,
  memberAt,
  missingSpansOf,
  rowsBySelectedKey,
  withinMapOf,
  chartTopOf,
} from '../../families/time-chart/seriesHelpers.ts';
import {
  chartAxis,
  chartWindow,
  lineChartSettings,
  type LineChartSettings,
} from '../../families/time-chart/settings.ts';
import { formatCompactScalar, formatScalar, notRecordedLabel } from '../../families/valueFormat.ts';
import type {
  AirLossesTileFamily,
  MilitaryAirAtReply,
  MilitaryAirAtRow,
  MilitaryAirMetric,
} from '../../generated/dashboard';
import {
  causeLabel,
  militaryAirParams,
  validateMilitaryAirAt,
  type MilitaryAirFilters,
} from '../bombing-damage/militaryAirAt.ts';

const COLOURED = 8;
const SCALE = 100_000;
const DEALT_SUFFIX = '\u0000dealt';
const TAKEN_SUFFIX = '\u0000taken';
const AIR_LEDGER_NOTE = 'Loss totals include corrections.';

export interface AirKillsLossesLinesOptions {
  readonly tags: readonly string[];
  readonly top: number | null;
  readonly rankMetric: MilitaryAirMetric;
  readonly holders: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly endLabels: boolean;
  readonly settings: LineChartSettings;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
}

const numbers = (value: OptionValue | undefined) =>
  contentSet(value ?? '').map(contentKey).map(Number).filter(Number.isInteger);

const filtersOf = (ctx: FamilyPlanContext): MilitaryAirFilters => ({
  regions: numbers(ctx.options.regions),
  causes: numbers(ctx.options.causes),
});


function rankMetricOf(raw: OptionValue | undefined): MilitaryAirMetric {
  return raw === 'dealt' ? 'dealt' : 'taken';
}

function slotKeyOf(row: Pick<MilitaryAirAtRow, 'identity' | 'tag'>): string {
  const slot = Number(row.identity.split(':')[0]);
  return Number.isSafeInteger(slot) ? countryKey(slot) : row.tag;
}

function formatAirCompact(raw: string): string {
  return formatCompactScalar(BigInt(raw), { decimals: 5, maxDecimals: 0 }).text;
}

function formatAirExact(raw: string): string {
  return formatScalar(BigInt(raw), { decimals: 5, maxDecimals: 5 });
}

function rawToPlot(raw: string): number {
  return Number(BigInt(raw)) / SCALE;
}

function ratioText(row: MilitaryAirAtRow): string | null {
  if (row.ratioHundredths === null) return null;
  const hundredths = BigInt(row.ratioHundredths);
  return `${hundredths / 100n}.${String(hundredths % 100n).padStart(2, '0')} : 1`;
}

export function airKillsLossesProvenInterval(data: unknown, hour: number): readonly [number, number] | null {
  const reply = data as MilitaryAirAtReply | null | undefined;
  if (!reply || typeof reply.hour !== 'number' || typeof reply.validUntil !== 'number') return null;
  return reply.hour <= hour && hour <= reply.validUntil ? [reply.hour, reply.validUntil] : null;
}

function unattributedNote(reply: MilitaryAirAtReply): string | null {
  const [dealt, taken] = reply.unattributed;
  if (dealt === '0' && taken === '0') return null;
  return `No recorded country: Own losses ${formatAirExact(taken)}, Enemy losses ${formatAirExact(dealt)}.`;
}

function buildHoverDetail(
  readView: BoundedReadView,
  orderedKeys: readonly string[],
  options: AirKillsLossesLinesOptions,
  looks: ReadonlyMap<string, ReturnType<typeof countrySeriesLooks> extends ReadonlyMap<string, infer T> ? T : never>,
): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<MilitaryAirAtReply>('hover');
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
      items: [{ label: 'Air kills & losses', missingReason: reply.missing || notRecordedLabel(), tone: 'muted' }],
    };
  }
  const eligible = orderedKeys.filter(key => memberAt(options.membership, key, hour));
  if (eligible.length === 0) return null;

  const rowOf = rowsBySelectedKey(reply.rows, slotKeyOf);
  const items: TooltipListItem[] = [];
  for (const key of eligible) {
    const look = looks.get(key);
    const atHour = look ? seriesLookAt(look, hour) : null;
    const label = atHour ? atHour.name : identityName(readView, 'country', key, hour);
    const who = readView.who('country', key, hour);
    const flagUrl = artUrlOf(flagChoiceArt(readView, who.flagChoice));
    const row = rowOf(key);
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
    items.push({
      label,
      strong: true,
      ...(atHour ? { swatch: atHour.color } : {}),
      ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
      divider: items.length > 0,
    });
    items.push({
      label: 'Enemy losses',
      value: formatAirExact(row.dealt),
      ...(row.partial ? { partialReason: row.partial } : {}),
      indent: true,
    });
    items.push({
      label: 'Own losses',
      value: formatAirExact(row.taken),
      ...(row.partial ? { partialReason: row.partial } : {}),
      indent: true,
    });
    const ratio = ratioText(row);
    if (ratio !== null) {
      items.push({ label: 'Ratio', value: ratio, indent: true });
    }
    for (const cause of row.causes) {
      items.push({
        label: causeLabel(cause),
        value: formatAirExact(cause.taken),
        ...(cause.missing ? { missingReason: cause.missing } : {}),
        indent: true,
      });
    }
  }
  return items.length > 0 ? { hour, replacesSeries: true, items } : null;
}

export const airKillsLossesLinesProjection: TimeChartProjection<AirKillsLossesLinesOptions> = registerFamilyProjection({
  kind: 'air-kills-losses',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Air kills & losses',
  project(readView: BoundedReadView, options: AirKillsLossesLinesOptions) {
    const hour = readView.hour;
    const yAxis = {
      format: 'compact' as const,
      decimals: 0,
      ...chartAxis(options.settings),
    };
    const view = chartWindow(options.settings, hour, options.extent[0]);
    const top = options.top !== null;

    const refused = readView.refused('head') ?? (top ? (readView.refused('dealt') ?? readView.refused('taken')) : undefined);
    if (refused) {
      return {
        kind: 'air-kills-losses',
        measureTitle: 'Air kills & losses',
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        empty: { message: refused, kind: 'unavailable' },
        auditRows: [],
      };
    }

    const headPanel = readView.panel<MilitaryAirAtReply>('head');
    const head = headPanel ? validateMilitaryAirAt(headPanel) : undefined;
    const dealtSeries = readView.series('dealt');
    const takenSeries = readView.series('taken');

    if (head && head.missing !== null) {
      return {
        kind: 'air-kills-losses',
        measureTitle: 'Air kills & losses',
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        warnings: head.warnings,
        empty: { message: head.missing, kind: 'unavailable' },
        auditRows: [auditRow('__head__', head.missing)],
      };
    }

    if (top && head && head.rankMissing !== null && head.rows.length === 0) {
      const note = unattributedNote(head);
      return {
        kind: 'air-kills-losses',
        measureTitle: 'Air kills & losses',
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        summaryText: AIR_LEDGER_NOTE,
        ...(note ? { note } : {}),
        warnings: head.warnings,
        empty: { message: head.rankMissing, kind: 'unavailable' },
        auditRows: [auditRow('__rank__', head.rankMissing)],
      };
    }

    const rowOfKey = rowsBySelectedKey(head?.rows ?? [], slotKeyOf);
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
        for (const row of head.rows) addKey(slotKeyOf(row));
      } else {
        for (const key of dealtSeries?.keys() ?? []) addKey(key);
        for (const key of takenSeries?.keys() ?? []) addKey(key);
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
    const warnings = [
      ...(head?.warnings ?? []),
      ...(!top && head?.rankMissing ? [head.rankMissing] : []),
    ];
    const note = head ? unattributedNote(head) : null;
    const legend: ChartLegendEntry[] = [
      { id: 'dealt', label: 'Solid: enemy losses' },
      { id: 'taken', label: 'Dashed: own losses', dash: SECONDARY_DASH },
    ];

    const auditRows: FamilyAuditRow[] = [];
    const series: TimeChartSeriesModel[] = [];

    orderedKeys.forEach((key, rank) => {
      const isTail = rank >= COLOURED;
      const look = looks.get(key)!;
      const row = rowOfKey(key);
      const present = memberAt(options.membership, key, hour);
      const dealtBounded = dealtSeries?.get(key) ?? null;
      const takenBounded = takenSeries?.get(key) ?? null;

      const dealtValue = present && row ? rawToPlot(row.dealt) : null;
      const takenValue = present && row ? rawToPlot(row.taken) : null;

      if (present && row) {
        const atLeast = row.partial ? ` (recorded subtotal: ${row.partial})` : '';
        auditRows.push(auditRow(`${key}${DEALT_SUFFIX}`, `${formatAirExact(row.dealt)}${atLeast}`));
        auditRows.push(auditRow(`${key}${TAKEN_SUFFIX}`, `${formatAirExact(row.taken)}${atLeast}`));
      } else if (present && head) {
        auditRows.push(auditRow(`${key}${DEALT_SUFFIX}`, 'This country is not present at this date.'));
        auditRows.push(auditRow(`${key}${TAKEN_SUFFIX}`, 'This country is not present at this date.'));
      }

      const labelOnDealt = options.rankMetric === 'dealt';
      const bothFigures = !isTail && options.endLabels && row && present
        ? { flag: look.flag, name: look.name, fullName: look.name, value: `${formatAirCompact(row.dealt)} / ${formatAirCompact(row.taken)}${row.partial ? ' (partial)' : ''}` }
        : null;
      const dealtEndLabel = labelOnDealt ? bothFigures : null;
      const takenEndLabel = labelOnDealt ? null : bothFigures;

      series.push({
        id: `${key}${DEALT_SUFFIX}`,
        key: `${key}${DEALT_SUFFIX}`,
        name: `${look.name} · Enemy losses`,
        color: look.color,
        role: isTail ? 'tail' : 'primary',
        step: true,
        bounded: dealtBounded,
        headValue: dealtValue,
        headHour: hour,
        flag: look.flag,
        identities: look.identities.map(run => ({ ...run, name: `${run.name} · Enemy losses` })),
        missingSpans: missingSpansOf(dealtBounded),
        endLabel: dealtEndLabel,
      });

      series.push({
        id: `${key}${TAKEN_SUFFIX}`,
        key: `${key}${TAKEN_SUFFIX}`,
        name: `${look.name} · Own losses`,
        color: look.color,
        role: isTail ? 'tail' : 'secondary',
        step: true,
        dash: SECONDARY_DASH,
        bounded: takenBounded,
        headValue: takenValue,
        headHour: hour,
        flag: look.flag,
        identities: look.identities.map(run => ({ ...run, name: `${run.name} · Own losses` })),
        missingSpans: missingSpansOf(takenBounded),
        endLabel: takenEndLabel,
      });
    });

    if (series.length === 0 && head) {
      return {
        kind: 'air-kills-losses',
        measureTitle: 'Air kills & losses',
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        legend,
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        hoverDetail,
        summaryText: AIR_LEDGER_NOTE,
        ...(note ? { note } : {}),
        warnings,
        empty: {
          message: options.rankMetric === 'dealt'
            ? 'No enemy aircraft destroyed yet.'
            : 'No aircraft lost yet.',
          kind: 'empty',
        },
        auditRows,
      };
    }

    return {
      kind: 'air-kills-losses',
      measureTitle: 'Air kills & losses',
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis,
      series,
      legend,
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      hoverDetail,
      summaryText: AIR_LEDGER_NOTE,
      ...(note ? { note } : {}),
      warnings,
      auditRows,
    };
  },
}, 'lines');

export const airKillsLossesLinesDeclaration: TimeChartWidgetDeclaration<AirKillsLossesLinesOptions> = {
  family: 'time-chart',
  kind: 'air-kills-losses',
  label: 'Air kills & losses',
  subject: 'aircraft losses',
  emptyCampaign: 'Load a campaign to see aircraft losses.',
  icon: 'target',
  countries: { mode: 'multi', allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const top = chartTopOf(ctx.countries);
    const tags = top === null ? ctx.countries.tags : [];
    const rankMetric = rankMetricOf(ctx.options.measure);
    const filters = filtersOf(ctx);
    const fromHour = ctx.staticSpan && ctx.widget.time.mode === 'range' ? ctx.staticSpan[0] : undefined;
    const owner = `air-kills-losses:lines:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${top === null ? tags.join(',') : `top${top}`}:${rankMetric}:${filters.regions.join(',')}:${filters.causes.join(',')}:${fromHour ?? ''}`;
    if (top === null && tags.length === 0) {
      return { owner, entries: [] };
    }
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    const within = top === null ? withinMapOf(ctx.countries.membership) : undefined;
    const tileFamily = (side: 'dealt' | 'taken'): AirLossesTileFamily => ({
      kind: 'air-losses',
      side,
      regions: [...filters.regions],
      causes: [...filters.causes],
      ...(fromHour !== undefined ? { fromHour } : {}),
      ...(top !== null ? { rankMetric } : {}),
    });
    const headParams = militaryAirParams(ctx, 'airLosses', rankMetric, filters);
    const group = top === null ? groupSelection(ctx.widget.countries) : null;
    const hoverParams: Record<string, unknown> = {
      branchId: ctx.branch.id,
      family: 'airLosses',
      regions: [...filters.regions],
      causes: [...filters.causes],
      ...(fromHour !== undefined ? { fromHour } : {}),
      metric: rankMetric,
      ...(top === null ? { countries: tags.map(countryFromKey) } : {}),
    };
    const tileEntry = (name: 'dealt' | 'taken'): SourceEntry => ({
      name,
      ...common,
      read: {
        shape: 'pixel-binned',
        coverage: top !== null
          ? { mode: 'population', form: 'top', limit: top }
          : { mode: 'keyed', keys: tags },
        family: tileFamily(name) as unknown as Readonly<Record<string, unknown>>,
        tags,
        ...(within ? { within } : {}),
        ...(top !== null ? { top: { n: top } } : {}),
        headQuery: null,
      },
    });
    const headCoverage = top !== null
      ? { mode: 'population' as const, form: 'top' as const, limit: top }
      : group
        ? { mode: 'population' as const, form: 'all' as const }
        : { mode: 'keyed' as const, keys: tags };
    const head: SourceEntry = {
      name: 'head',
      ...common,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage: headCoverage,
        product: 'military-air-at',
        exactProduct: 'military-air-at',
        followQuery: null,
        params: headParams,
        dynamicKeys: ['hour'],
        provenInterval: airKillsLossesProvenInterval,
        validate: validateMilitaryAirAt,
      },
    };
    return {
      owner,
      exactEntry: head,
      entries: [
        tileEntry('dealt'),
        tileEntry('taken'),
        head,
        {
          name: 'hover',
          ...common,
          read: {
            shape: 'exact-hover',
            product: 'military-air-at',
            params: hoverParams,
            ...(top !== null ? { countriesOf: 'dealt' } : {}),
            ...(top === null && ctx.countries.membership ? { membership: ctx.countries.membership } : {}),
            validate: validateMilitaryAirAt,
          },
        },
      ],
    };
  },
  projection: airKillsLossesLinesProjection,
  projectionOptions(ctx, { extent, surface }) {
    const top = chartTopOf(ctx.countries);
    return {
      tags: top === null ? ctx.countries.tags : [],
      top,
      rankMetric: rankMetricOf(ctx.options.measure),
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
