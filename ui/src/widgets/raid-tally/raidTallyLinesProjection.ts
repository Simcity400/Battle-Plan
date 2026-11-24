import { CHART_MEASURE_COLOURS, SECONDARY_DASH } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { auditRow, type FamilyAuditRow } from '../../families/auditRow.ts';
import { artUrlOf, flagChoiceArt } from '../../families/art/provider.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourcePlan,
} from '../../families/contract.ts';
import type { HolderRun, HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import {
  seriesLookAt,
  type ChartGap,
  type ChartHoverDetail,
  type ChartInlinePoint,
  type ChartLegendEntry,
  type TimeChartSeriesModel,
} from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  countrySeriesLooks,
  memberAt,
} from '../../families/time-chart/seriesHelpers.ts';
import {
  chartAxis,
  chartWindow,
  lineChartSettings,
  type LineChartSettings,
} from '../../families/time-chart/settings.ts';
import { formatScalar, notRecordedLabel } from '../../families/valueFormat.ts';
import type {
  RaidTallyAtReply,
  RaidTallyMetric,
  RaidTallyReply,
  RaidTallySlotRun,
} from '../../generated/dashboard';
import { validateRaidTallyAt } from './raidTallyRaceProjection.ts';

const TAIL_COLOUR = CHART_MEASURE_COLOURS.tailLine;
const COLOURED = 8;
const LAUNCHED_SUFFIX = '\u0000launched';
const SUCCEEDED_SUFFIX = '\u0000succeeded';
const SUCCESS_NOTE = 'Success includes critical success and excludes limited success.';

export interface RaidTallyLinesOptions {
  readonly tags: readonly string[];
  readonly top: number | null;
  readonly metric: RaidTallyMetric;
  readonly holders: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly endLabels: boolean;
  readonly settings: LineChartSettings;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const isInt = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v);
const isText = (v: unknown): v is string => typeof v === 'string';
const maybe = (v: unknown, test: (x: unknown) => boolean): boolean => v === null || test(v);

function topOf(countries: FamilyPlanContext['countries']): number | null {
  return countries.top ?? (countries.all ? Math.max(1, countries.every.length) : null);
}

export function raidTallyMetric(raw: unknown): RaidTallyMetric {
  if (raw === 'launched' || raw === 'succeeded') return raw;
  return 'both';
}

export function raidTallyTitle(metric: RaidTallyMetric): string {
  if (metric === 'launched') return 'Raid launches';
  if (metric === 'succeeded') return 'Raid successes';
  return 'Raid launches and successes';
}

export function unresolvedSlotLabel(slot: number): string {
  return `Country #${slot} (tag not recorded)`;
}

export function raidTallySlotLabel(readView: BoundedReadView, slot: number, hour: number): string {
  const key = countryKey(slot);
  const who = readView.who('country', key, hour);
  if (who.name.status === 'ready') return who.name.value;
  if (who.tag?.status === 'ready') return who.tag.value;
  return unresolvedSlotLabel(slot);
}

export function validateRaidTallyReply(value: unknown): RaidTallyReply {
  const ok = isRecord(value)
    && isInt(value.hour)
    && maybe(value.nextChange, isInt)
    && maybe(value.available, item => typeof item === 'boolean')
    && maybe(value.unavailable, isText)
    && isInt(value.unattributed) && value.unattributed >= 0
    && Array.isArray(value.series) && value.series.every(item => isRecord(item)
      && isInt(item.slot) && item.slot >= 0
      && Array.isArray(item.runs) && item.runs.every(run => isRecord(run)
        && isInt(run.firstHour) && isInt(run.lastHour) && run.firstHour <= run.lastHour
        && run.lastHour <= (value.hour as number)
        && isInt(run.launched) && run.launched >= 0
        && isInt(run.succeeded) && run.succeeded >= 0)
      && maybe(item.membership, spans => Array.isArray(spans) && spans.every(span => isRecord(span)
        && isInt(span.firstHour) && isInt(span.lastHour) && span.firstHour <= span.lastHour
        && span.lastHour <= (value.hour as number))))
    && Array.isArray(value.missing) && value.missing.every(span => isRecord(span)
      && isInt(span.firstHour) && isInt(span.lastHour) && span.firstHour <= span.lastHour
      && span.lastHour <= (value.hour as number)
      && isText(span.reason))
    && Array.isArray(value.warnings) && value.warnings.every(isText);
  if (!ok) throw new Error('The raid tally reply is malformed.');
  return value as unknown as RaidTallyReply;
}

export function raidTallyProof(
  reply: unknown,
  _span: readonly [number, number],
  hour: number,
): readonly [number, number] | null {
  const checked = validateRaidTallyReply(reply);
  if (hour < checked.hour) return null;
  if (checked.nextChange !== null && hour >= checked.nextChange) return null;
  return [checked.hour, checked.nextChange === null ? Infinity : checked.nextChange - 1];
}

function clipRunsToRanges(
  runs: readonly RaidTallySlotRun[],
  ranges: readonly (readonly [number, number])[],
): RaidTallySlotRun[] {
  const out: RaidTallySlotRun[] = [];
  for (const run of runs) {
    for (const [a, b] of ranges) {
      const firstHour = Math.max(run.firstHour, a);
      const lastHour = Math.min(run.lastHour, b);
      if (firstHour <= lastHour) {
        out.push({ firstHour, lastHour, launched: run.launched, succeeded: run.succeeded });
      }
    }
  }
  return out;
}

function runsToStepPoints(
  runs: readonly RaidTallySlotRun[],
  field: 'launched' | 'succeeded',
  floorHour: number,
): ChartInlinePoint[] {
  const points: ChartInlinePoint[] = [];
  let prevLast: number | null = null;
  for (const run of runs) {
    if (prevLast !== null && run.firstHour > prevLast + 1) {
      points.push({ hour: prevLast + 1, value: null });
    }
    const val = run[field];
    points.push({ hour: run.firstHour, value: val });
    if (run.lastHour > run.firstHour) {
      points.push({ hour: run.lastHour, value: val });
    }
    prevLast = run.lastHour;
  }
  if (prevLast !== null && prevLast < floorHour) {
    points.push({ hour: prevLast + 1, value: null });
  }
  return points;
}

function runAtHour(runs: readonly RaidTallySlotRun[], hour: number): RaidTallySlotRun | null {
  for (const run of runs) {
    if (run.firstHour <= hour && hour <= run.lastHour) return run;
  }
  return null;
}

function missingAtHour(reply: RaidTallyReply, hour: number): string | null {
  for (const span of reply.missing) {
    if (span.firstHour <= hour && hour <= span.lastHour) return span.reason;
  }
  return null;
}

function formatHoverRowValue(metric: RaidTallyMetric, launched: number, succeeded: number): string {
  if (metric === 'launched') return formatScalar(launched, 'integer');
  if (metric === 'succeeded') return formatScalar(succeeded, 'integer');
  return `${formatScalar(launched, 'integer')} launched · ${formatScalar(succeeded, 'integer')} succeeded`;
}

function buildHoverDetail(
  readView: BoundedReadView,
  options: RaidTallyLinesOptions,
  looks: ReadonlyMap<string, ReturnType<typeof countrySeriesLooks> extends ReadonlyMap<string, infer T> ? T : never>,
): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<RaidTallyAtReply>('hover');
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
      items: [{ label: raidTallyTitle(options.metric), missingReason: reply.missing || notRecordedLabel(), tone: 'muted' }],
    };
  }
  const rows = options.membership
    ? reply.rows.filter(row => memberAt(options.membership, countryKey(row.country), hour))
    : reply.rows;
  if (rows.length === 0) {
    return {
      hour,
      replacesSeries: true,
      items: [{ label: 'No raids recorded at this hour', tone: 'muted' }],
    };
  }
  const items: TooltipListItem[] = rows.map(row => {
    const key = countryKey(row.country);
    const label = typeof row.country === 'number'
      ? raidTallySlotLabel(readView, row.country, hour)
      : (() => {
          const who = readView.who('country', key, hour);
          return who.name.status === 'ready' ? who.name.value : row.tag;
        })();
    const look = looks.get(key);
    const atHour = look ? seriesLookAt(look, hour) : null;
    const who = readView.who('country', key, hour);
    const flagUrl = artUrlOf(flagChoiceArt(readView, who.flagChoice));
    if (row.missing !== null) {
      return {
        label,
        missingReason: row.missing,
        tone: 'muted' as const,
        ...(atHour ? { swatch: atHour.color } : {}),
        ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
      };
    }
    return {
      label,
      value: formatHoverRowValue(options.metric, row.launched, row.succeeded),
      ...(atHour ? { swatch: atHour.color } : {}),
      ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
    };
  });
  return { hour, replacesSeries: true, items };
}

export const raidTallyLinesProjection: TimeChartProjection<RaidTallyLinesOptions> = registerFamilyProjection({
  kind: 'raid-tally',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Raid tally',
  nextChange(readView) {
    return readView.panel<RaidTallyReply>('tally')?.nextChange ?? Infinity;
  },
  project(readView: BoundedReadView, options: RaidTallyLinesOptions) {
    const hour = readView.hour;
    const floorHour = Math.floor(hour);
    const title = raidTallyTitle(options.metric);
    const yAxis = {
      format: 'compact' as const,
      decimals: 0,
      ...chartAxis(options.settings),
    };
    const view = chartWindow(options.settings, hour, options.extent[0]);
    const reply = readView.panel<RaidTallyReply>('tally');
    if (!reply) {
      return {
        kind: 'raid-tally',
        measureTitle: title,
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        note: SUCCESS_NOTE,
        auditRows: [],
      };
    }

    const hasEarlierGap = reply.missing.some(span => span.firstHour < floorHour);
    const warnings = [
      ...reply.warnings,
      ...(reply.unattributed > 0 ? [`${reply.unattributed} raids have no named raider and are not counted.`] : []),
      ...(hasEarlierGap ? ['Raids during an unrecorded stretch before this date are not counted.'] : []),
    ];
    const gaps: ChartGap[] = [
      ...options.gaps,
      ...reply.missing.map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour, reason: span.reason })),
    ];

    if (reply.available === false) {
      return {
        kind: 'raid-tally',
        measureTitle: title,
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis,
        series: [],
        gaps: gaps.length > 0 ? gaps : undefined,
        warnings,
        empty: { message: 'No raid types available.', detail: reply.unavailable ?? undefined, kind: 'unavailable' },
        auditRows: [],
      };
    }

    const headMissing = missingAtHour(reply, floorHour);
    const orderedKeys = reply.series.map(item => countryKey(item.slot));
    const slotByKey = new Map(reply.series.map(item => [countryKey(item.slot), item.slot]));
    const rowOf = new Map(orderedKeys.map((key, idx) => [key, idx]));
    const rawLooks = countrySeriesLooks(orderedKeys, readView, {
      holders: options.holders,
      from: options.extent[0],
      surface: options.surface,
      withFlag: true,
      recordedOrdinal: key => rowOf.get(key) ?? 0,
      tailColor: TAIL_COLOUR,
    });
    const looks = new Map(
      [...rawLooks.entries()].map(([key, look]) => {
        const slot = slotByKey.get(key)!;
        const cleanName = (candidate: string) => (candidate === key ? unresolvedSlotLabel(slot) : candidate);
        const identities = look.identities.map(run => ({ ...run, name: cleanName(run.name) }));
        return [key, { ...look, name: cleanName(look.name), identities }];
      }),
    );

    const hoverDetail = buildHoverDetail(readView, options, looks);
    const auditRows: FamilyAuditRow[] = [];
    if (headMissing !== null) {
      auditRows.push(auditRow('__head__', headMissing));
    }

    const series: TimeChartSeriesModel[] = [];
    reply.series.forEach((item, rank) => {
      const key = countryKey(item.slot);
      const isTail = rank >= COLOURED;
      const look = looks.get(key)!;
      const effectiveRanges: readonly (readonly [number, number])[] | null = item.membership !== null
        ? item.membership.map(span => [span.firstHour, span.lastHour] as const)
        : options.membership
          ? (options.membership[key] ?? [])
          : null;
      const runs = effectiveRanges !== null ? clipRunsToRanges(item.runs, effectiveRanges) : item.runs;
      if (runs.length === 0) return;

      const headRun = headMissing === null ? runAtHour(runs, floorHour) : null;
      const primaryField = options.metric === 'succeeded' ? 'succeeded' : 'launched';
      const primaryValue = headRun ? headRun[primaryField] : null;
      const primaryPoints = runsToStepPoints(runs, primaryField, floorHour);

      if (headRun !== null) {
        if (options.metric === 'both') {
          auditRows.push(auditRow(`${key}${LAUNCHED_SUFFIX}`, formatScalar(headRun.launched, 'integer')));
          auditRows.push(auditRow(`${key}${SUCCEEDED_SUFFIX}`, formatScalar(headRun.succeeded, 'integer')));
        } else {
          auditRows.push(auditRow(key, formatScalar(headRun[primaryField], 'integer')));
        }
      }

      const primaryId = options.metric === 'both' ? `${key}${LAUNCHED_SUFFIX}` : key;
      const primaryName = options.metric === 'both' ? `${look.name} · Launched` : look.name;
      series.push({
        id: primaryId,
        key: primaryId,
        name: primaryName,
        color: look.color,
        role: isTail ? 'tail' : 'primary',
        step: true,
        points: primaryPoints,
        headValue: primaryValue,
        headHour: hour,
        flag: look.flag,
        identities: options.metric === 'both'
          ? look.identities.map(run => ({ ...run, name: `${run.name} · Launched` }))
          : look.identities,
        missingSpans: reply.missing,
        rank,
        endLabel: isTail || primaryValue === null || !options.endLabels
          ? null
          : {
              flag: look.flag,
              name: look.name,
              fullName: look.name,
              value: formatScalar(primaryValue, 'integer'),
            },
      });

      if (options.metric === 'both') {
        const succeededPoints = runsToStepPoints(runs, 'succeeded', floorHour);
        const succeededId = `${key}${SUCCEEDED_SUFFIX}`;
        series.push({
          id: succeededId,
          key: succeededId,
          name: `${look.name} · Succeeded`,
          color: look.color,
          role: isTail ? 'tail' : 'secondary',
          step: true,
          dash: SECONDARY_DASH,
          points: succeededPoints,
          headValue: headRun ? headRun.succeeded : null,
          headHour: hour,
          identities: look.identities.map(run => ({ ...run, name: `${run.name} · Succeeded` })),
          missingSpans: reply.missing,
          rank,
          endLabel: null,
        });
      }
    });

    const legend: ChartLegendEntry[] | undefined = options.metric === 'both'
      ? [
          { id: 'launched', label: 'Solid: launched' },
          { id: 'succeeded', label: 'Dashed: succeeded', dash: SECONDARY_DASH },
        ]
      : undefined;

    if (series.length === 0) {
      return {
        kind: 'raid-tally',
        measureTitle: title,
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis,
        series: [],
        ...(legend ? { legend } : {}),
        gaps: gaps.length > 0 ? gaps : undefined,
        hoverDetail,
        note: SUCCESS_NOTE,
        warnings,
        empty: headMissing !== null
          ? { message: headMissing, kind: 'unavailable' }
          : {
              message: options.top !== null
                ? `No raids ${options.metric === 'both' ? 'launched or succeeded' : options.metric} yet.`
                : 'No raid counts recorded.',
              kind: 'empty',
            },
        auditRows,
      };
    }

    return {
      kind: 'raid-tally',
      measureTitle: title,
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis,
      series,
      ...(legend ? { legend } : {}),
      gaps: gaps.length > 0 ? gaps : undefined,
      hoverDetail,
      note: SUCCESS_NOTE,
      warnings,
      auditRows,
    };
  },
}, 'lines');

export const raidTallyLinesDeclaration: TimeChartWidgetDeclaration<RaidTallyLinesOptions> = {
  family: 'time-chart',
  kind: 'raid-tally',
  label: 'Raid tally',
  subject: 'raid tally',
  emptyCampaign: 'Load a campaign to see its raid tally.',
  icon: 'target',
  countries: { mode: 'multi', labels: true, colours: true, allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const top = topOf(ctx.countries);
    const tags = top === null ? ctx.countries.tags : [];
    const metric = raidTallyMetric(ctx.options.metric);
    const owner = `raid-tally:lines:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${top === null ? tags.join(',') : `top${top}`}:${metric}`;
    if (top === null && tags.length === 0) {
      return { owner, entries: [] };
    }
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    const countries = top !== null ? [] : tags.map(countryFromKey);
    return {
      owner,
      entries: [
        {
          name: 'tally',
          ...common,
          read: {
            shape: 'branch-to-hour',
            coverage: top !== null
              ? { mode: 'population', form: 'top', limit: top }
              : { mode: 'keyed', keys: tags },
            product: 'raid-tally',
            followQuery: null,
            params: { branchId: ctx.branch.id, countries, top, metric },
            buildSpanParams: (params, span) => ({ ...params, hour: span.lastHour }),
            provenInterval: raidTallyProof,
            dynamicKeys: ['hour'],
            validate: validateRaidTallyReply,
          },
        },
        {
          name: 'hover',
          ...common,
          read: {
            shape: 'exact-hover',
            product: 'raid-tally-at',
            params: { branchId: ctx.branch.id, countries, ...(top !== null ? { top } : {}), metric },
            validate: validateRaidTallyAt,
          },
        },
      ],
    };
  },
  projection: raidTallyLinesProjection,
  projectionOptions(ctx, { extent, surface }) {
    const top = topOf(ctx.countries);
    return {
      tags: top === null ? ctx.countries.tags : [],
      top,
      metric: raidTallyMetric(ctx.options.metric),
      holders: ctx.countries.holders,
      membership: top === null ? ctx.countries.membership : null,
      endLabels: ctx.options.endLabels !== false,
      settings: lineChartSettings(ctx.options),
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
    };
  },
};
