import { colour, nudgeApart, readableColour } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { contentKey } from '../../dashboard/options.ts';
import { auditRow, type FamilyAuditRow } from '../../families/auditRow.ts';
import { flagChoiceArt } from '../../families/art/provider.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourcePlan,
} from '../../families/contract.ts';
import { holderRunAt } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { IdentityIndex } from '../../families/identity/index.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import type { ChartArtSlot, ChartGap, ChartHoverDetail, ChartStackBand } from '../../families/time-chart/model.ts';
import { branchGapsOf } from '../../families/time-chart/seriesHelpers.ts';
import { formatDateOnly, formatScalar, notRecordedLabel } from '../../families/valueFormat.ts';
import type { CountryIdentitiesReply, OccupationReserveReply, ReserveHistoryReply, ReserveSegment } from '../../generated/dashboard';
import { parseColor, toHex } from '../../shell/appearance/color.ts';
import {
  holderOf,
  reserveHistoryProof,
  reserveHistoryProvenLast,
  segmentAt,
  segmentParts,
  segmentTotal,
  sourceTagAt,
  UNADMITTED_RESERVE_REASON,
  validateOccupationReserve,
  validateReserveHistory,
} from './occupationReserveData.ts';
import { identityName } from '../../families/table/identityCell.ts';

export interface OccupationReserveHolderChoice {
  readonly tag: string;
  readonly id: number;
  readonly slot?: number;
}

export interface OccupationReserveBreakdownOptions {
  readonly holder: OccupationReserveHolderChoice | null;
  readonly holderReason?: string;
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

function validSpan(value: unknown): boolean {
  if (!value || typeof value !== 'object') return false;
  const { firstHour, lastHour } = value as { firstHour?: unknown; lastHour?: unknown };
  return Number.isSafeInteger(firstHour) && Number.isSafeInteger(lastHour) && (firstHour as number) <= (lastHour as number);
}

function validU32(value: unknown): boolean {
  return Number.isSafeInteger(value) && (value as number) >= 0 && (value as number) <= 0xffff_ffff;
}

function validateIdentitiesAux(value: unknown): CountryIdentitiesReply {
  const reply = value as Partial<CountryIdentitiesReply> | null;
  const ok = !!reply && typeof reply === 'object'
    && (reply.known === undefined || (Array.isArray(reply.known) && reply.known.every(validSpan)))
    && Array.isArray(reply.unknown) && reply.unknown.every(span => validSpan(span) && typeof (span as { reason?: unknown }).reason === 'string')
    && Array.isArray(reply.countries) && reply.countries.every(entry => !!entry && typeof entry === 'object'
      && typeof entry.tag === 'string'
      && Array.isArray(entry.identities) && entry.identities.every(run => validSpan(run)
        && validU32((run as { slot?: unknown }).slot)
        && validU32((run as { countryId?: unknown }).countryId))
      && Array.isArray(entry.unresolved) && entry.unresolved.every(validSpan));
  if (!ok) {
    throw new Error('The country identities reply is malformed.');
  }
  return value as CountryIdentitiesReply;
}

function singleCountryKey(ctx: FamilyPlanContext<CountryIdentitiesReply>): string | null {
  return ctx.countries.top === null && !ctx.countries.all && ctx.countries.tags.length === 1
    ? ctx.countries.tags[0]!
    : null;
}

const identityIndexes = new WeakMap<CountryIdentitiesReply, IdentityIndex>();

function identityIndexOf(aux: CountryIdentitiesReply, branch: FamilyPlanContext['branch']): IdentityIndex {
  let index = identityIndexes.get(aux);
  if (!index) {
    index = new IdentityIndex({ sourceId: '', revision: '', branchId: branch.id, content: 0 }, [branch.firstHour, branch.lastHour]);
    index.setIdentities(aux);
    identityIndexes.set(aux, index);
  }
  return index;
}

function displayedPlanHour(ctx: FamilyPlanContext<CountryIdentitiesReply>, fallbackHour?: number): number {
  if (ctx.planHour !== undefined) return Math.floor(ctx.planHour);
  if (ctx.staticSpan) return Math.floor(ctx.staticSpan[1]);
  const pinned = ctx.widget?.time?.mode === 'pin' ? ctx.widget.time.windows[ctx.track.id] : undefined;
  return Math.floor(pinned ? ctx.branch.firstHour + pinned.lastOffset : fallbackHour ?? ctx.lastRecordedHour ?? ctx.branch.lastHour);
}

function holderAtHour(ctx: FamilyPlanContext<CountryIdentitiesReply>, atHour: number): {
  readonly holder: OccupationReserveHolderChoice | null;
  readonly valid: readonly [number, number];
  readonly reason?: string;
} {
  const explicit = ctx.options.holder ? holderOf(contentKey(String(ctx.options.holder))) : null;
  if (explicit) {
    if (!ctx.aux) return { holder: explicit, valid: [-Infinity, Infinity] };
    const resolved = identityIndexOf(ctx.aux, ctx.branch).countryHolderByIdAt(explicit.id, atHour);
    return {
      holder: resolved.holder ? { tag: resolved.holder.tag, id: explicit.id, slot: resolved.holder.slot } : null,
      valid: resolved.valid,
      ...(resolved.reason ? { reason: resolved.reason } : {}),
    };
  }
  const key = singleCountryKey(ctx);
  if (!key) return { holder: null, valid: [-Infinity, Infinity] };
  if (ctx.aux) {
    const resolved = identityIndexOf(ctx.aux, ctx.branch).countryHolderAt(key, atHour);
    return {
      holder: resolved.holder ? { tag: resolved.holder.tag, id: resolved.holder.countryId, slot: resolved.holder.slot } : null,
      valid: resolved.valid,
      ...(resolved.reason ? { reason: resolved.reason } : {}),
    };
  }
  const run = holderRunAt(ctx.countries.holders?.[key], atHour);
  if (run?.countryId !== undefined) {
    const parsed = countryFromKey(key);
    return {
      holder: typeof parsed === 'number' ? { tag: run.tag, id: run.countryId, slot: parsed } : { tag: run.tag, id: run.countryId },
      valid: [run.firstHour, run.lastHour],
    };
  }
  return { holder: null, valid: [-Infinity, Infinity] };
}

function sourcesIn(segments: readonly ReserveSegment[]): { readonly bands: number[]; readonly all: number[] } {
  const peak = new Map<number, number>();
  const seen = new Set<number>();
  for (const segment of segments) {
    for (const entry of segment.entries ?? []) {
      seen.add(entry.source);
      if (entry.amount > 0) peak.set(entry.source, Math.max(peak.get(entry.source) ?? 0, entry.amount));
    }
  }
  const bands = [...peak.entries()]
    .sort((a, b) => b[1] - a[1] || a[0] - b[0])
    .map(([source]) => source);
  const all = [...bands, ...[...seen].filter(source => !peak.has(source)).sort((a, b) => a - b)];
  return { bands, all };
}

function unresolvedSourceLabel(sourceId: number): string {
  return `Source #${sourceId} (tag not recorded)`;
}

function countryNameAt(readView: BoundedReadView, key: string | null, atHour: number, fallback: string): string {
  if (key === null) return fallback;
  return identityName(readView, 'country', key, Math.min(atHour, readView.hour));
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
  const bg = parseColor(surface)!;
  const floorHour = Math.floor(hour);
  const rawRgbs = sources.map((source, index) => {
    const tag = sourceTagAt(history, source, floorHour);
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
  history: ReserveHistoryReply,
  sources: readonly number[],
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
  const found = reply.holders.find(item => item.country.id === holder.id);
  const hoverFloor = Math.floor(hour);
  const sourceLook = (sourceId: number, tag: string | null, withSwatch = true) => {
    const flagSlot = countryFlagAt(readView, tag, hour);
    const flagUrl = flagSlot?.art.status === 'ready' ? flagSlot.art.url : undefined;
    const swatch = withSwatch ? colours.get(sourceId) : undefined;
    return {
      label: countryNameAt(readView, tag, hour, unresolvedSourceLabel(sourceId)),
      ...(swatch ? { swatch } : {}),
      ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
    };
  };
  if (!found) {
    const segment = segmentAt(history, hoverFloor);
    if (!segment || segment.entries === null) {
      return {
        hour,
        replacesSeries: true,
        items: [{ label: 'Reserve manpower', missingReason: segment?.unknown ?? notRecordedLabel(), tone: 'muted' }],
      };
    }
    const parts = segmentParts(segment, sources);
    const total = segmentTotal(segment, parts);
    const positive: TooltipListItem[] = [];
    const unread: TooltipListItem[] = [];
    for (let i = 0; i < sources.length; i++) {
      const part = parts[i]!;
      if (part.kind === 'zero') continue;
      const sourceId = sources[i]!;
      const look = sourceLook(sourceId, sourceTagAt(history, sourceId, hoverFloor));
      if (part.kind === 'known') positive.push({ ...look, value: formatScalar(part.value, 'integer') });
      else unread.push({ ...look, missingReason: part.reason, tone: 'muted' });
    }
    const negative: TooltipListItem[] = segment.entries
      .filter(entry => entry.amount < 0)
      .map(entry => ({ ...sourceLook(entry.source, sourceTagAt(history, entry.source, hoverFloor), false), value: formatScalar(entry.amount, 'integer') }));
    const totalItem: TooltipListItem = total.kind === 'notRecorded'
      ? { label: 'Reserve manpower', missingReason: total.reason, tone: 'muted', ...(positive.length ? { strong: true } : {}) }
      : { label: 'Reserve manpower held', value: formatScalar(total.value, 'integer'), ...(total.kind === 'zero' ? { tone: 'muted' as const } : { strong: true }) };
    return {
      hour,
      replacesSeries: true,
      items: [
        totalItem,
        ...[...positive, ...unread].map((item, idx) => (idx === 0 ? { ...item, divider: true } : item)),
        ...(negative.length ? [{ label: 'Negative entries', value: '', divider: true }, ...negative] : []),
      ],
    };
  }
  const positive: TooltipListItem[] = [];
  const negative: TooltipListItem[] = [];
  const witnessed = new Set<number>();
  for (const entry of found.entries) {
    witnessed.add(entry.source.id);
    if (entry.amount < 0) {
      negative.push({ ...sourceLook(entry.source.id, entry.source.tag, false), value: formatScalar(entry.amount, 'integer') });
    } else {
      positive.push({ ...sourceLook(entry.source.id, entry.source.tag), value: formatScalar(entry.amount, 'integer') });
    }
  }
  const unreadReason = reply.noneMissing ?? UNADMITTED_RESERVE_REASON;
  const unreadItems: TooltipListItem[] = found.admitted ? [] : sources.filter(id => !witnessed.has(id)).map(sourceId => ({
    ...sourceLook(sourceId, sourceTagAt(history, sourceId, hoverFloor)),
    missingReason: unreadReason,
    tone: 'muted',
  }));
  const totalItem: TooltipListItem = unreadItems.length
    ? { label: 'Reserve manpower held', missingReason: unreadReason, tone: 'muted', strong: true }
    : { label: 'Reserve manpower held', value: formatScalar(found.total, 'integer'), strong: true };
  const items: TooltipListItem[] = [
    totalItem,
    ...[...positive, ...unreadItems].map((item, idx) => (idx === 0 ? { ...item, divider: true } : item)),
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
    const name = countryNameAt(readView, tag, hour, unresolvedSourceLabel(entry.source));
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
  nextChange(readView) {
    return readView.panel<ReserveHistoryReply>('history')?.nextChange ?? Infinity;
  },
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
      const empty = options.resolvingHolder
        ? null
        : options.holderReason
          ? { message: options.holderReason, kind: options.holderReason === 'not a country at this hour' ? ('empty' as const) : ('unavailable' as const) }
          : { message: 'Choose a holder: click its bar in the race.', kind: 'empty' as const };
      return {
        kind: 'occupation-reserve-manpower',
        measureTitle: 'Reserve manpower by source country',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: baseAxis,
        series: [],
        empty,
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

    const floorHour = Math.floor(hour);
    const provenLast = reserveHistoryProvenLast(history);
    const segments = history.segments;
    const { bands: sources, all: allSources } = sourcesIn(segments);
    const colours = assignSourceColours(allSources, history, readView, hour, options.surface);
    const hoverDetail = buildHoverDetail(readView, holder, history, allSources, colours);
    const headSegment = segmentAt(history, floorHour);
    const note = negativeNoteAt(history, headSegment, readView, floorHour);
    const holderKey = holder.slot !== undefined ? countryKey(holder.slot) : holder.tag;
    const holderName = countryNameAt(readView, holderKey, hour, holder.tag);

    const hasRecorded = segments.some(segment => segment.entries !== null);
    if (!hasRecorded) {
      const reason = segments.find(segment => segment.unknown)?.unknown ?? `Reserve manpower was not recorded for ${holderName} at this date.`;
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
      const headParts = headSegment ? segmentParts(headSegment, allSources) : null;
      const headTotal = headSegment && headParts ? segmentTotal(headSegment, headParts) : null;
      if (!headTotal || headTotal.kind === 'notRecorded') {
        const auditRows: FamilyAuditRow[] = [];
        if (headSegment?.entries !== null && headTotal && headParts) {
          auditRows.push(auditRow('__total__', headTotal.reason));
          allSources.forEach((sourceId, index) => {
            const part = headParts[index]!;
            if (part.kind === 'notRecorded') auditRows.push(auditRow(String(sourceId), part.reason));
          });
        }
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
          empty: { message: headTotal?.reason ?? notRecordedLabel(), kind: 'unavailable' },
          auditRows,
        };
      }
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
    const bandValues: (number | null)[][] = allSources.map(() => []);
    const bandMissing: (string | null)[][] = allSources.map(() => []);
    const totalValues: (number | null)[] = [];
    const totalMissing: (string | null)[] = [];
    const gaps: ChartGap[] = [...options.gaps];

    const pushNull = (at: number, reason: string | null = null) => {
      if (hours.length > 0 && hours[hours.length - 1] === at) return;
      hours.push(at);
      for (let i = 0; i < allSources.length; i++) {
        bandValues[i]!.push(null);
        bandMissing[i]!.push(i < sources.length ? reason : null);
      }
      totalValues.push(null);
      totalMissing.push(reason);
    };

    let prevLast: number | null = null;
    for (const segment of segments) {
      const segLast = segment.lastHour === history.hour ? Math.max(segment.lastHour, Math.min(floorHour, provenLast)) : segment.lastHour;
      if (prevLast !== null && segment.firstHour > prevLast + 1) {
        pushNull(prevLast + 1);
      }
      if (segment.entries === null) {
        const reason = segment.unknown ?? notRecordedLabel();
        pushNull(segment.firstHour, reason);
        gaps.push({
          firstHour: segment.firstHour,
          lastHour: segLast,
          reason,
        });
      } else {
        const parts = segmentParts(segment, allSources);
        const total = segmentTotal(segment, parts);
        const anyKnown = parts.slice(0, sources.length).some(part => part.kind !== 'notRecorded');
        if (!anyKnown) {
          const reason = total.kind === 'notRecorded' ? total.reason : notRecordedLabel();
          hours.push(segment.firstHour);
          for (let i = 0; i < allSources.length; i++) {
            const part = parts[i]!;
            bandValues[i]!.push(null);
            bandMissing[i]!.push(part.kind === 'notRecorded' ? part.reason : null);
          }
          totalValues.push(null);
          totalMissing.push(reason);
          gaps.push({
            firstHour: segment.firstHour,
            lastHour: segLast,
            reason,
          });
        } else {
          hours.push(segment.firstHour);
          for (let i = 0; i < allSources.length; i++) {
            const part = parts[i]!;
            bandValues[i]!.push(i < sources.length && part.kind !== 'notRecorded' ? part.value : null);
            bandMissing[i]!.push(part.kind === 'notRecorded' ? part.reason : null);
          }
          totalValues.push(total.kind === 'notRecorded' ? null : total.value);
          totalMissing.push(total.kind === 'notRecorded' ? total.reason : null);
        }
      }
      prevLast = segLast;
    }
    if (prevLast !== null && prevLast < floorHour) {
      pushNull(prevLast + 1);
      gaps.push({
        firstHour: prevLast + 1,
        lastHour: hour,
        reason: notRecordedLabel(),
      });
    }

    const headParts = headSegment ? segmentParts(headSegment, allSources) : null;
    const headTotal = headSegment && headParts ? segmentTotal(headSegment, headParts) : null;
    const bandIndices = allSources
      .map((_, i) => i)
      .filter(i => i < sources.length || bandMissing[i]!.some(reason => reason !== null));

    const bands: ChartStackBand[] = bandIndices.map(index => {
      const sourceId = allSources[index]!;
      const tag = sourceTagAt(history, sourceId, floorHour);
      const name = countryNameAt(readView, tag, hour, unresolvedSourceLabel(sourceId));
      const flag = countryFlagAt(readView, tag, hour);
      const color = colours.get(sourceId) ?? colour(options.surface, index);
      const headPart = headParts?.[index];
      const missingReasons = bandMissing[index]!;
      return {
        id: String(sourceId),
        key: tag ?? String(sourceId),
        name,
        color,
        values: bandValues[index]!,
        ...(missingReasons.some(reason => reason !== null) ? { missingReason: missingReasons } : {}),
        ...(flag ? { flag } : {}),
        endLabel: headPart?.kind === 'known'
          ? {
              ...(flag ? { flag } : { colorBar: color }),
              name,
              fullName: name,
              value: formatScalar(headPart.value, 'integer'),
            }
          : null,
      };
    });

    const auditRows: FamilyAuditRow[] = [];
    if (headTotal) {
      auditRows.push(auditRow(
        '__total__',
        headTotal.kind === 'notRecorded' ? headTotal.reason : formatScalar(headTotal.value, 'integer'),
      ));
    }
    if (headSegment?.entries !== null && headParts) {
      allSources.forEach((sourceId, index) => {
        const part = headParts[index]!;
        if (part.kind === 'zero') return;
        auditRows.push(auditRow(
          String(sourceId),
          part.kind === 'notRecorded' ? part.reason : formatScalar(part.value, 'integer'),
        ));
      });
    }

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
        total: {
          values: totalValues,
          ...(totalMissing.some(reason => reason !== null) ? { missingReason: totalMissing } : {}),
        },
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
  countries: { mode: 'multi', allowSlots: true },
  aux: {
    product: 'country-identities',
    params: track => ({ branchId: track.branch.id }),
    validate: validateIdentitiesAux,
  },
  sources(ctx: FamilyPlanContext<CountryIdentitiesReply>, view): SourcePlan {
    const atHour = displayedPlanHour(ctx, view.staticSpan?.[1]);
    const { holder, valid } = holderAtHour(ctx, atHour);
    const owner = `occupation-reserve:breakdown:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${holder ? `${holder.slot ?? holder.tag}|${holder.id}` : ''}`;
    if (!holder) return { owner, entries: [], allowEmptyFollow: true, valid };
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    const hoverCountry = holder.slot !== undefined ? holder.slot : countryFromKey(holder.tag);
    return {
      owner,
      valid,
      entries: [
        {
          name: 'history',
          ...common,
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'keyed', keys: [holder.slot !== undefined ? countryKey(holder.slot) : holder.tag] },
            product: 'occupation-reserve-history',
            followQuery: null,
            params: { branchId: ctx.branch.id, holder: holder.id },
            buildSpanParams: (params, span) => ({ ...params, hour: span.lastHour }),
            provenInterval: reserveHistoryProof,
            dynamicKeys: ['hour'],
            validate: validateReserveHistory,
          },
        },
        {
          name: 'hover',
          ...common,
          read: {
            shape: 'exact-hover',
            product: 'occupation-reserve-manpower',
            params: { branchId: ctx.branch.id, countries: [hoverCountry] },
            validate: validateOccupationReserve,
          },
        },
      ],
    };
  },
  projection: occupationReserveBreakdownProjection,
  projectionOptions(ctx, { extent, surface }) {
    const atHour = displayedPlanHour(ctx, extent[1]);
    const { holder, reason } = holderAtHour(ctx, atHour);
    return {
      holder,
      ...(reason ? { holderReason: reason } : {}),
      resolvingHolder: holder === null && singleCountryKey(ctx) !== null && !ctx.aux,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      yMin: numericOption(ctx.options.yMin),
      yMax: numericOption(ctx.options.yMax),
    };
  },
};
