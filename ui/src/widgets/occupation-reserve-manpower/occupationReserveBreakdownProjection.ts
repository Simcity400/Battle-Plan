import { colour, nudgeApart, readableColour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey } from '../../dashboard/options.ts';
import { auditRow, type FamilyAuditRow } from '../../families/auditRow.ts';
import { flagChoiceArt } from '../../families/art/provider.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourcePlan,
} from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import type { ChartArtSlot, ChartGap, ChartHoverDetail, ChartStackBand } from '../../families/time-chart/model.ts';
import { branchGapsOf } from '../../families/time-chart/seriesHelpers.ts';
import { wholeBranchProof } from '../../families/timeline/helpers.ts';
import { formatDateOnly, formatScalar, notRecordedLabel } from '../../families/valueFormat.ts';
import type { CountryIdentitiesReply, OccupationReserveReply, ReserveHistoryReply, ReserveSegment } from '../../generated/dashboard';
import { parseColor, toHex } from '../../shell/appearance/color.ts';
import {
  holderOf,
  segmentAt,
  segmentParts,
  sourceTagAt,
  validateOccupationReserve,
  validateReserveHistory,
} from './occupationReserveData.ts';

export interface OccupationReserveHolderChoice {
  readonly tag: string;
  readonly id: number;
}

export interface OccupationReserveBreakdownOptions {
  readonly holder: OccupationReserveHolderChoice | null;
  readonly resolvingHolder?: boolean;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly yMin: number | null;
  readonly yMax: number | null;
}

function numericOption(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function validateIdentitiesAux(value: unknown): CountryIdentitiesReply {
  if (!value || typeof value !== 'object' || !Array.isArray((value as CountryIdentitiesReply).countries)) {
    throw new Error('The country identities reply is malformed.');
  }
  return value as CountryIdentitiesReply;
}

function singleCountryKey(ctx: FamilyPlanContext<CountryIdentitiesReply>): string | null {
  return ctx.countries.top === null && !ctx.countries.all && ctx.countries.tags.length === 1
    ? ctx.countries.tags[0]!
    : null;
}

function resolveHolder(ctx: FamilyPlanContext<CountryIdentitiesReply>): OccupationReserveHolderChoice | null {
  const explicit = ctx.options.holder ? holderOf(contentKey(String(ctx.options.holder))) : null;
  if (explicit) return explicit;
  const key = singleCountryKey(ctx);
  if (!key) return null;
  const run = ctx.countries.holders?.[key]?.find(item => item.countryId !== undefined);
  if (run && run.countryId !== undefined) {
    return { tag: run.tag, id: run.countryId };
  }
  if (!ctx.aux) return null;
  const parsed = countryFromKey(key);
  if (typeof parsed === 'number') {
    for (const entry of ctx.aux.countries) {
      const hit = entry.identities.find(item => item.slot === parsed);
      if (hit) return { tag: entry.tag, id: hit.countryId };
    }
    return null;
  }
  const byTag = ctx.aux.countries.find(entry => entry.tag === parsed);
  const first = byTag?.identities[0];
  return first ? { tag: byTag.tag, id: first.countryId } : null;
}

function latestSourceTagUpTo(history: ReserveHistoryReply, source: number, hour: number): string | null {
  const exact = sourceTagAt(history, source, hour);
  if (exact !== null) return exact;
  const tags = history.sources.find(item => item.id === source)?.tags ?? [];
  for (let i = tags.length - 1; i >= 0; i--) {
    const item = tags[i]!;
    if (item.firstHour <= hour && item.tag !== null) return item.tag;
  }
  return null;
}

function clipSegments(segments: readonly ReserveSegment[], hour: number): ReserveSegment[] {
  const out: ReserveSegment[] = [];
  for (const segment of segments) {
    if (segment.firstHour > hour) break;
    out.push(segment.lastHour > hour ? { ...segment, lastHour: hour } : segment);
  }
  return out;
}

function sourcesUpTo(segments: readonly ReserveSegment[]): number[] {
  const peak = new Map<number, number>();
  for (const segment of segments) {
    for (const entry of segment.entries ?? []) {
      if (entry.amount <= 0) continue;
      peak.set(entry.source, Math.max(peak.get(entry.source) ?? 0, entry.amount));
    }
  }
  return [...peak.entries()]
    .sort((a, b) => b[1] - a[1] || a[0] - b[0])
    .map(([source]) => source);
}

function countryNameAt(readView: BoundedReadView, tag: string | null, atHour: number, fallback: string): string {
  if (tag === null) return fallback;
  const who = readView.who('country', tag, Math.min(atHour, readView.hour));
  return who.name.status === 'ready' ? who.name.value : tag;
}

function countryFlagAt(readView: BoundedReadView, tag: string | null, atHour: number): ChartArtSlot | undefined {
  if (tag === null) return undefined;
  const who = readView.who('country', tag, Math.min(atHour, readView.hour));
  return { art: flagChoiceArt(readView, who.flagChoice), tag };
}

function assignSourceColours(
  sources: readonly number[],
  history: ReserveHistoryReply,
  readView: BoundedReadView,
  hour: number,
  surface: string,
): ReadonlyMap<number, string> {
  const out = new Map<number, string>();
  const bg = parseColor(surface) ?? { r: 17, g: 17, b: 17 };
  const rawRgbs = sources.map((source, index) => {
    const tag = latestSourceTagUpTo(history, source, hour);
    const who = tag ? readView.who('country', tag, hour) : null;
    const hex = who?.colour?.status === 'ready'
      ? readableColour(who.colour.value, surface)
      : colour(surface, index);
    return parseColor(hex) ?? { r: 120, g: 160, b: 220 };
  });
  const distinct = nudgeApart(rawRgbs, bg);
  sources.forEach((source, index) => out.set(source, toHex(distinct[index]!)));
  return out;
}

function readExactHover(readView: BoundedReadView): { readonly hour: number; readonly reply: OccupationReserveReply } | null {
  let hover;
  try {
    hover = readView.hover<OccupationReserveReply>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover || !hover.data) return null;
  return { hour: hover.hour, reply: hover.data };
}

function buildHoverDetail(
  readView: BoundedReadView,
  holder: OccupationReserveHolderChoice,
  colours: ReadonlyMap<number, string>,
): ChartHoverDetail | null {
  const exact = readExactHover(readView);
  if (!exact) return null;
  const { hour, reply } = exact;
  if (reply.missing !== null) {
    return {
      hour,
      replacesSeries: true,
      items: [{ label: 'Reserve manpower', missingReason: reply.missing || notRecordedLabel(), tone: 'muted' }],
    };
  }
  const found = reply.holders.find(item => item.country.id === holder.id) ?? reply.holders[0];
  if (!found) {
    return {
      hour,
      replacesSeries: true,
      items: [{ label: 'Reserve manpower held', value: '0', tone: 'muted' }],
    };
  }
  const positive: TooltipListItem[] = [];
  const negative: TooltipListItem[] = [];
  for (const entry of found.entries) {
    const tag = entry.source.tag;
    const name = countryNameAt(readView, tag, hour, 'Source country unavailable');
    const flagSlot = countryFlagAt(readView, tag, hour);
    const flagUrl = flagSlot?.art.status === 'ready' ? flagSlot.art.url : undefined;
    const swatch = colours.get(entry.source.id);
    if (entry.amount < 0) {
      negative.push({
        label: name,
        value: formatScalar(entry.amount, 'integer'),
        ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
      });
    } else {
      positive.push({
        label: name,
        value: formatScalar(entry.amount, 'integer'),
        ...(swatch ? { swatch } : {}),
        ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
      });
    }
  }
  const items: TooltipListItem[] = [
    { label: 'Reserve manpower held', value: formatScalar(found.total, 'integer'), strong: true },
    ...positive.map((item, idx) => (idx === 0 ? { ...item, divider: true } : item)),
    ...(negative.length ? [{ label: 'Negative entries', value: '', divider: true }, ...negative] : []),
    ...(reply.readHour !== null ? [{ label: 'As of', value: formatDateOnly(reply.readHour), tone: 'muted' as const }] : []),
  ];
  return { hour, replacesSeries: true, items };
}

function negativeNoteAt(
  history: ReserveHistoryReply,
  segment: ReserveSegment | null,
  readView: BoundedReadView,
  hour: number,
): string | null {
  const negative = segment?.entries?.filter(entry => entry.amount < 0) ?? [];
  if (!negative.length) return null;
  const list = negative.map(entry => {
    const tag = sourceTagAt(history, entry.source, hour);
    const name = countryNameAt(readView, tag, hour, 'Source country unavailable');
    return `${name} ${formatScalar(entry.amount, 'integer')}`;
  });
  return `Negative entries: ${list.join(' · ')}`;
}

export const occupationReserveBreakdownProjection: TimeChartProjection<OccupationReserveBreakdownOptions> = registerFamilyProjection({
  kind: 'occupation-reserve-manpower',
  family: 'time-chart',
  variant: 'stacked-area',
  joinMode: 'step',
  measureTitle: 'Reserve manpower by source country',
  project(readView: BoundedReadView, options: OccupationReserveBreakdownOptions) {
    const hour = readView.hour;
    const holder = options.holder;
    const baseAxis = {
      format: 'compact' as const,
      decimals: 0,
      ...(options.yMin !== null ? { min: options.yMin } : {}),
      ...(options.yMax !== null ? { max: options.yMax } : {}),
    };
    if (!holder) {
      return {
        kind: 'occupation-reserve-manpower',
        measureTitle: 'Reserve manpower by source country',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: baseAxis,
        series: [],
        empty: options.resolvingHolder ? null : { message: 'Choose a holder: click its bar in the race.', kind: 'empty' },
        auditRows: [],
      };
    }

    const history = readView.panel<ReserveHistoryReply>('history');
    if (!history) {
      return {
        kind: 'occupation-reserve-manpower',
        measureTitle: 'Reserve manpower by source country',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: baseAxis,
        series: [],
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        auditRows: [],
      };
    }

    const clipped = clipSegments(history.segments, hour);
    const sources = sourcesUpTo(clipped);
    const colours = assignSourceColours(sources, history, readView, hour, options.surface);
    const hoverDetail = buildHoverDetail(readView, holder, colours);
    const headSegment = segmentAt({ ...history, segments: clipped }, Math.floor(hour));
    const note = negativeNoteAt(history, headSegment, readView, Math.floor(hour));
    const holderName = countryNameAt(readView, holder.tag, hour, holder.tag);

    const hasRecorded = clipped.some(segment => segment.entries !== null);
    if (!hasRecorded) {
      const reason = clipped.find(segment => segment.unknown)?.unknown ?? `Reserve manpower was not recorded for ${holderName} at this date.`;
      return {
        kind: 'occupation-reserve-manpower',
        measureTitle: 'Reserve manpower by source country',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: baseAxis,
        series: [],
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        hoverDetail,
        warnings: history.warnings,
        empty: { message: reason, kind: 'unavailable' },
        auditRows: [],
      };
    }

    if (!sources.length) {
      return {
        kind: 'occupation-reserve-manpower',
        measureTitle: 'Reserve manpower by source country',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: baseAxis,
        series: [],
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        note,
        hoverDetail,
        warnings: history.warnings,
        empty: { message: `${holderName} holds no reserve manpower at this date.`, kind: 'empty' },
        auditRows: [auditRow('__total__', '0')],
      };
    }

    const hours: number[] = [];
    const bandValues: (number | null)[][] = sources.map(() => []);
    const totalValues: (number | null)[] = [];
    const gaps: ChartGap[] = [...options.gaps];

    const pushNull = (at: number) => {
      if (hours.length > 0 && hours[hours.length - 1] === at) return;
      hours.push(at);
      for (const vals of bandValues) vals.push(null);
      totalValues.push(null);
    };

    let prevLast: number | null = null;
    for (const segment of clipped) {
      if (prevLast !== null && segment.firstHour > prevLast + 1) {
        pushNull(prevLast + 1);
      }
      if (segment.entries === null) {
        pushNull(segment.firstHour);
        gaps.push({
          firstHour: segment.firstHour,
          lastHour: segment.lastHour,
          reason: segment.unknown ?? notRecordedLabel(),
        });
      } else {
        const parts = segmentParts(segment, sources);
        const anyKnown = parts.some(value => value !== undefined);
        if (!anyKnown) {
          pushNull(segment.firstHour);
        } else {
          hours.push(segment.firstHour);
          let sum = 0;
          for (let i = 0; i < sources.length; i++) {
            const val = parts[i] ?? 0;
            bandValues[i]!.push(val);
            sum += val;
          }
          totalValues.push(sum);
        }
      }
      prevLast = segment.lastHour;
    }
    if (prevLast !== null && prevLast < hour) {
      pushNull(prevLast + 1);
      gaps.push({
        firstHour: prevLast + 1,
        lastHour: hour,
        reason: notRecordedLabel(),
      });
    }

    const headParts = headSegment ? segmentParts(headSegment, sources) : sources.map(() => undefined);
    const headTotal = headSegment?.entries
      ? headSegment.entries.reduce((sum, entry) => sum + Math.max(0, entry.amount), 0)
      : null;

    const bands: ChartStackBand[] = sources.map((sourceId, index) => {
      const tag = latestSourceTagUpTo(history, sourceId, hour);
      const name = countryNameAt(readView, tag, hour, 'Source country unavailable');
      const flag = countryFlagAt(readView, tag, hour);
      const color = colours.get(sourceId) ?? colour(options.surface, index);
      const headVal = headParts[index];
      return {
        id: String(sourceId),
        key: tag ?? String(sourceId),
        name,
        color,
        values: bandValues[index]!,
        ...(flag ? { flag } : {}),
        endLabel: headVal !== undefined && headVal > 0
          ? {
              ...(flag ? { flag } : { colorBar: color }),
              name,
              fullName: name,
              value: formatScalar(headVal, 'integer'),
            }
          : null,
      };
    });

    const auditRows: FamilyAuditRow[] = [];
    if (headTotal !== null) {
      auditRows.push(auditRow('__total__', formatScalar(headTotal, 'integer')));
    }
    bands.forEach((band, index) => {
      const val = headParts[index];
      if (val !== undefined && val > 0) {
        auditRows.push(auditRow(band.id, formatScalar(val, 'integer')));
      }
    });

    return {
      kind: 'occupation-reserve-manpower',
      measureTitle: 'Reserve manpower by source country',
      mode: 'stack',
      hour,
      extent: options.extent,
      yAxis: baseAxis,
      series: [],
      stack: {
        hours,
        bands,
        step: true,
        total: { values: totalValues },
      },
      gaps: gaps.length > 0 ? gaps : undefined,
      note,
      hoverDetail,
      warnings: history.warnings,
      auditRows,
    };
  },
}, 'breakdown');

export const occupationReserveBreakdownDeclaration: TimeChartWidgetDeclaration<OccupationReserveBreakdownOptions, CountryIdentitiesReply> = {
  family: 'time-chart',
  kind: 'occupation-reserve-manpower',
  label: 'Holder breakdown',
  subject: 'occupation manpower',
  emptyCampaign: 'Load a campaign to see occupation manpower.',
  icon: 'flag',
  countries: { mode: 'multi', labels: true, colours: true, allowSlots: true },
  aux: {
    product: 'country-identities',
    params: track => ({ branchId: track.branch.id }),
    validate: validateIdentitiesAux,
  },
  sources(ctx: FamilyPlanContext<CountryIdentitiesReply>): SourcePlan {
    const holder = resolveHolder(ctx);
    const owner = `occupation-reserve:breakdown:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${holder ? `${holder.tag}|${holder.id}` : ''}`;
    if (!holder) return { owner, entries: [], allowEmptyFollow: true };
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    return {
      owner,
      entries: [
        {
          name: 'history',
          ...common,
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'keyed', keys: [holder.tag] },
            product: 'occupation-reserve-history',
            followQuery: null,
            params: { branchId: ctx.branch.id, holder: holder.id },
            buildSpanParams: params => params,
            provenInterval: wholeBranchProof,
            validate: validateReserveHistory,
          },
        },
        {
          name: 'hover',
          ...common,
          read: {
            shape: 'exact-hover',
            product: 'occupation-reserve-manpower',
            params: { branchId: ctx.branch.id, countries: [countryFromKey(holder.tag)] },
            validate: validateOccupationReserve,
          },
        },
      ],
    };
  },
  projection: occupationReserveBreakdownProjection,
  projectionOptions(ctx, { extent, surface }) {
    const holder = resolveHolder(ctx);
    return {
      holder,
      resolvingHolder: holder === null && singleCountryKey(ctx) !== null && !ctx.aux,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      yMin: numericOption(ctx.options.yMin),
      yMax: numericOption(ctx.options.yMax),
    };
  },
};
