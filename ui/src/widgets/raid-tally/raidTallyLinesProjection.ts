import { SECONDARY_DASH, tailColour } from '../../charts/palette.ts';
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
  chartTopOf,
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
import { identityName, identityNameOr } from '../../families/table/identityCell.ts';

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
  if (who.name.status === 'ready' || who.name.status === 'pending') return identityName(readView, 'country', key, hour);
  if (who.tag?.status === 'ready') return who.tag.value;
  return unresolvedSlotLabel(slot);
}

export function validateRaidTallyReply(value: unknown): RaidTallyReply {
  const validSpans = (spans: unknown, maxHour: number): boolean => Array.isArray(spans) && spans.every(span => isRecord(span)
    && isInt(span.firstHour) && isInt(span.lastHour) && span.firstHour <= span.lastHour
    && span.lastHour <= maxHour
    && isText(span.reason));
  const ok = isRecord(value)
    && isInt(value.hour)
    && maybe(value.nextChange, isInt)
    && maybe(value.available, item => typeof item === 'boolean')
    && maybe(value.unavailable, isText)
    && (value.rankMissing === undefined || maybe(value.rankMissing, isText))
    && (value.rankMissingFrom === undefined || maybe(value.rankMissingFrom, isInt))
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
    && validSpans(value.missing, value.hour)
    && (value.partial === undefined || validSpans(value.partial, value.hour))
    && (value.partialLaunched === undefined || maybe(value.partialLaunched, spans => validSpans(spans, value.hour as number)))
    && (value.partialSucceeded === undefined || maybe(value.partialSucceeded, spans => validSpans(spans, value.hour as number)))
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
  const partialReason = reply.partial ?? null;
  const isUnattributedOnly = partialReason !== null && reply.unattributed > 0 && partialReason.includes('unrecorded raider');
  const gapPartial = partialReason !== null && !isUnattributedOnly ? partialReason : null;
  const launchedPartial = gapPartial
    ?? (reply.unattributedLaunched > 0
      ? `${formatScalar(reply.unattributedLaunched, 'integer')} raid(s) up to this date have an unrecorded raider`
      : null);
  const succeededPartial = gapPartial
    ?? (reply.unattributedSucceeded > 0
      ? `${formatScalar(reply.unattributedSucceeded, 'integer')} raid(s) up to this date have an unrecorded raider`
      : null);
  const items: TooltipListItem[] = [];
  for (const row of rows) {
    const key = countryKey(row.country);
    const label = typeof row.country === 'number'
      ? raidTallySlotLabel(readView, row.country, hour)
      : identityNameOr(readView, 'country', key, row.tag, hour);
    const look = looks.get(key);
    const atHour = look ? seriesLookAt(look, hour) : null;
    const who = readView.who('country', key, hour);
    const flagUrl = artUrlOf(flagChoiceArt(readView, who.flagChoice));
    const lookMeta = {
      ...(atHour ? { swatch: atHour.color } : {}),
      ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
    };
    if (row.missing !== null) {
      items.push({
        label,
        missingReason: row.missing,
        tone: 'muted' as const,
        ...lookMeta,
      });
      continue;
    }
    if (options.metric === 'both' && launchedPartial !== succeededPartial) {
      items.push({
        label: `${label} · Launched`,
        value: formatScalar(row.launched, 'integer'),
        ...(launchedPartial ? { partialReason: launchedPartial } : {}),
        ...lookMeta,
      });
      items.push({
        label: `${label} · Succeeded`,
        value: formatScalar(row.succeeded, 'integer'),
        ...(succeededPartial ? { partialReason: succeededPartial } : {}),
        ...lookMeta,
      });
      continue;
    }
    const rowPartial = options.metric === 'succeeded' ? succeededPartial : launchedPartial;
    items.push({
      label,
      value: formatHoverRowValue(options.metric, row.launched, row.succeeded),
      ...(rowPartial ? { partialReason: rowPartial } : {}),
      ...lookMeta,
    });
  }
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
        summaryText: SUCCESS_NOTE,
        auditRows: [],
      };
    }

    const extendSpans = <T extends { readonly lastHour: number }>(spans: readonly T[]): readonly T[] => {
      if (floorHour <= reply.hour || spans.length === 0) return spans;
      const last = spans[spans.length - 1]!;
      return last.lastHour === reply.hour ? [...spans.slice(0, -1), { ...last, lastHour: floorHour }] : spans;
    };
    const effectiveMissing = extendSpans(reply.missing);
    const effectivePartial = extendSpans(reply.partial ?? []);
    const effectiveLaunchedPartial = options.metric === 'both' && reply.partialLaunched != null
      ? extendSpans(reply.partialLaunched)
      : effectivePartial;
    const effectiveSucceededPartial = options.metric === 'both' && reply.partialSucceeded != null
      ? extendSpans(reply.partialSucceeded)
      : effectivePartial;
    const hasEarlierGap = effectiveMissing.some(span => span.firstHour < floorHour);
    const warnings = [
      ...reply.warnings,
      ...(reply.unattributed > 0 ? [`${reply.unattributed} raids have no named raider and are not counted.`] : []),
      ...(hasEarlierGap ? ['Raids during an unrecorded stretch before this date are not counted.'] : []),
    ];
    const rankRefusal = options.top !== null && reply.rankMissing ? reply.rankMissing : null;
    const rankRefusalFrom = rankRefusal !== null
      ? Math.min(floorHour, reply.rankMissingFrom ?? effectiveMissing[0]?.firstHour ?? effectivePartial[0]?.firstHour ?? floorHour)
      : null;
    const seriesMissing = rankRefusal !== null && rankRefusalFrom !== null
      ? [
          ...effectiveMissing.filter(span => span.lastHour < rankRefusalFrom),
          { firstHour: rankRefusalFrom, lastHour: floorHour, reason: rankRefusal },
        ]
      : effectiveMissing;
    const gaps: ChartGap[] = [
      ...options.gaps,
      ...seriesMissing.map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour, reason: span.reason })),
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

    if (rankRefusal !== null && reply.series.length === 0) {
      return {
        kind: 'raid-tally',
        measureTitle: title,
        mode: 'lines',
        hour,
        extent: options.extent,
        ...(view ? { view } : {}),
        yAxis,
        series: [],
        gaps: gaps.length > 0 ? gaps : undefined,
        summaryText: SUCCESS_NOTE,
        warnings,
        empty: { message: rankRefusal, kind: 'unavailable' },
        auditRows: [auditRow('__rank__', rankRefusal)],
      };
    }

    const headMissing = rankRefusal ?? (effectiveMissing.find(span => span.firstHour <= floorHour && floorHour <= span.lastHour)?.reason ?? null);
    const partialAtHead = (spans: readonly { readonly firstHour: number; readonly lastHour: number; readonly reason: string }[]) =>
      headMissing === null ? (spans.find(span => span.firstHour <= floorHour && floorHour <= span.lastHour)?.reason ?? null) : null;
    const headPartial = partialAtHead(effectivePartial);
    const headLaunchedPartial = partialAtHead(effectiveLaunchedPartial);
    const headSucceededPartial = partialAtHead(effectiveSucceededPartial);
    const primaryPartialSpans = rankRefusal !== null
      ? undefined
      : options.metric === 'both'
        ? (effectiveLaunchedPartial.length ? effectiveLaunchedPartial : undefined)
        : (effectivePartial.length ? effectivePartial : undefined);
    const succeededPartialSpans = rankRefusal !== null
      ? undefined
      : (effectiveSucceededPartial.length ? effectiveSucceededPartial : undefined);
    const orderedKeys = reply.series.map(item => countryKey(item.slot));
    const slotByKey = new Map(reply.series.map(item => [countryKey(item.slot), item.slot]));
    const rawLooks = countrySeriesLooks(orderedKeys, readView, {
      holders: options.holders,
      from: options.extent[0],
      surface: options.surface,
      withFlag: true,
      tailColor: tailColour(options.surface),
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
    if (rankRefusal !== null) {
      auditRows.push(auditRow('__rank__', rankRefusal));
    } else if (headMissing !== null) {
      auditRows.push(auditRow('__head__', headMissing));
    }

    const series: TimeChartSeriesModel[] = [];
    reply.series.forEach((item, rank) => {
      const key = countryKey(item.slot);
      const isTail = rank >= COLOURED;
      const look = looks.get(key)!;
      const rawRuns = extendSpans(item.runs);
      const effectiveRanges: readonly (readonly [number, number])[] | null = item.membership !== null
        ? extendSpans(item.membership).map(span => [span.firstHour, span.lastHour] as const)
        : options.membership
          ? (options.membership[key] ?? [])
          : null;
      const runs = effectiveRanges !== null ? clipRunsToRanges(rawRuns, effectiveRanges) : rawRuns;
      if (runs.length === 0) return;

      const headRun = headMissing === null ? runAtHour(runs, floorHour) : null;
      const primaryField = options.metric === 'succeeded' ? 'succeeded' : 'launched';
      const primaryValue = headRun ? headRun[primaryField] : null;
      const primaryPoints = runsToStepPoints(runs, primaryField, floorHour);

      if (headRun !== null) {
        if (options.metric === 'both') {
          const launchedSuffix = headLaunchedPartial ? ` (recorded subtotal: ${headLaunchedPartial})` : '';
          const succeededSuffix = headSucceededPartial ? ` (recorded subtotal: ${headSucceededPartial})` : '';
          auditRows.push(auditRow(`${key}${LAUNCHED_SUFFIX}`, `${formatScalar(headRun.launched, 'integer')}${launchedSuffix}`));
          auditRows.push(auditRow(`${key}${SUCCEEDED_SUFFIX}`, `${formatScalar(headRun.succeeded, 'integer')}${succeededSuffix}`));
        } else {
          const suffix = headPartial ? ` (recorded subtotal: ${headPartial})` : '';
          auditRows.push(auditRow(key, `${formatScalar(headRun[primaryField], 'integer')}${suffix}`));
        }
      }

      const primaryId = options.metric === 'both' ? `${key}${LAUNCHED_SUFFIX}` : key;
      const primaryName = options.metric === 'both' ? `${look.name} · Launched` : look.name;
      const endLabelValue = headRun === null
        ? null
        : options.metric === 'both'
          ? (headLaunchedPartial && headSucceededPartial
            ? `${formatScalar(headRun.launched, 'integer')} / ${formatScalar(headRun.succeeded, 'integer')} (partial)`
            : `${formatScalar(headRun.launched, 'integer')}${headLaunchedPartial ? ' (partial)' : ''} / ${formatScalar(headRun.succeeded, 'integer')}${headSucceededPartial ? ' (partial)' : ''}`)
          : `${formatScalar(headRun[primaryField], 'integer')}${headPartial ? ' (partial)' : ''}`;
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
        missingSpans: seriesMissing,
        ...(primaryPartialSpans ? { partialSpans: primaryPartialSpans } : {}),
        endLabel: isTail || endLabelValue === null || !options.endLabels
          ? null
          : {
              flag: look.flag,
              name: look.name,
              fullName: look.name,
              value: endLabelValue,
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
          missingSpans: seriesMissing,
          ...(succeededPartialSpans ? { partialSpans: succeededPartialSpans } : {}),
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
        summaryText: SUCCESS_NOTE,
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
      summaryText: SUCCESS_NOTE,
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
  countries: { mode: 'multi', content: 'history', allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const top = chartTopOf(ctx.countries);
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
    const top = chartTopOf(ctx.countries);
    return {
      tags: top === null ? ctx.countries.tags : [],
      top,
      metric: raidTallyMetric(ctx.options.metric),
      holders: ctx.countries.holders,
      membership: top === null ? ctx.countries.membership : null,
      endLabels: ctx.options.endLabels !== false,
      settings: lineChartSettings(ctx.options, ctx.following),
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
    };
  },
};
