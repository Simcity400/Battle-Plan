import { liveKey, type LiveValues } from '../../charts/live.ts';
import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { LegendStripEntry } from '../../draw/kit.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { inHourSpans, nextBoundaryAfter, timelineModel, wholeBranchProof } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineModel, type TimelineProjection, type TimelineSpan } from '../../families/timeline/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { formatDateOnly, formatScalar } from '../../families/valueFormat.ts';
import type { HourSpan, ResearchEntry, ResearchSlotNow, ResearchSlotsReply } from '../../generated/dashboard';
import { mix } from '../../shell/appearance/color.ts';
import {
  completedBy,
  historyTokens,
  isUnjoinedResearch,
  researchArt,
  researchArtwork,
  researchAt,
  researchFolderAt,
  researchFolderLooks,
  researchReadName,
  validateResearchReply,
  validateResearchSlots,
  type ResearchArtwork,
  type ResearchFolderLooks,
} from './researchData.ts';
import { completedRow, coverageAt, coverageGaps, researchDays } from './researchLayout.ts';
import { slotNumbers } from './researchSlots.ts';
import type { ResearchData, ResearchSlotHistory } from './types';

interface ResearchTimelineOptions {
  readonly folders: string;
  readonly firstHour: number;
  readonly surface: string;
}

interface ResearchSpanMeta {
  readonly slotIndex: number;
  readonly entry: ResearchEntry | null;
  readonly token: string;
  readonly databaseIndex: number | null;
  readonly techName: string;
  readonly folderName: string | null;
  readonly description: string | null;
  readonly openedHour: number;
  readonly unjoined: boolean;
}

interface ResearchLaneMeta {
  readonly slotIndex: number;
  readonly openedHour: number;
  readonly openedRecorded: boolean;
  readonly closedHour: number | null;
}

const EMPTY_ARTWORK = new Map<string, never>();
const nextDayAfter = (hour: number) => (Math.floor(hour / 24) + 1) * 24;

interface ResearchCache {
  readonly artwork: ResearchArtwork;
  readonly looks: ResearchFolderLooks;
  readonly boundaries: readonly number[];
}

const caches = new WeakMap<ResearchData, ResearchCache>();

function researchCache(data: ResearchData): ResearchCache {
  let cached = caches.get(data);
  if (cached) return cached;
  const hours: number[] = [];
  for (const span of data.history.coverage) hours.push(span.firstHour, span.lastHour + 1);
  for (const slot of data.history.slots) {
    for (const span of slot.availability) hours.push(span.firstHour, span.lastHour + 1);
    for (const entry of slot.entries) {
      hours.push(entry.firstObservedHour);
      if (entry.endedByHour !== null) hours.push(entry.endedByHour);
      if (entry.outcomeObservedHour !== null) hours.push(entry.outcomeObservedHour);
    }
  }
  cached = {
    artwork: researchArtwork(data, EMPTY_ARTWORK),
    looks: researchFolderLooks(data, historyTokens(data.history), []),
    boundaries: [...new Set(hours)].sort((a, b) => a - b),
  };
  caches.set(data, cached);
  return cached;
}

function unobservedSpans(coverage: readonly HourSpan[], firstHour: number, hour: number): HourSpan[] {
  const gaps = coverageGaps(coverage);
  if (!coverage.length) {
    return hour >= firstHour ? [{ firstHour, lastHour: hour }] : [];
  }
  const out: HourSpan[] = [];
  if (coverage[0]!.firstHour > firstHour) {
    out.push({ firstHour, lastHour: coverage[0]!.firstHour - 1 });
  }
  out.push(...gaps);
  const last = coverage[coverage.length - 1]!.lastHour;
  if (last < hour) {
    out.push({ firstHour: last + 1, lastHour: hour });
  }
  return out;
}

function liveProgressOf(live: LiveValues<string | number | bigint> | null | undefined, slotIndex: number): number | undefined {
  const raw = live?.get(liveKey(String(slotIndex), 'progress'));
  return typeof raw === 'number' ? raw : typeof raw === 'bigint' ? Number(raw) : undefined;
}

function headNoteOf(
  slotsNow: ResearchSlotsReply | undefined,
  live: LiveValues<string | number | bigint> | null | undefined,
  artwork: ResearchArtwork,
  hour: number,
  openSlotCount: number,
): string | null {
  if (!slotsNow) return null;
  if (slotsNow.missing && !slotsNow.slots.length) return `Head: ${slotsNow.missing}`;
  if (!slotsNow.slots.length) return openSlotCount > 0 ? 'Head: not recorded' : null;
  let activeCount = 0;
  let featured: { readonly name: string; readonly percent: string; readonly days: string; readonly sort: number } | null = null;
  for (const read of slotsNow.slots) {
    if (read.state.token === null) continue;
    activeCount++;
    const numbers = slotNumbers(read, hour, liveProgressOf(live, read.index));
    const rawName = researchReadName(artwork, read);
    const name = contentTitle(rawName, read.state.token, 'Technology name unavailable');
    const sort = numbers.left !== null ? -numbers.left : (numbers.fraction ?? -1) * 1000;
    if (!featured || sort > featured.sort) {
      featured = { name, percent: numbers.percent, days: numbers.days, sort };
    }
  }
  if (activeCount === 0) return 'Head: all slots idle';
  const countText = `Head: ${formatScalar(activeCount, 'integer')} in progress`;
  if (!featured) return countText;
  const parts = [featured.percent !== 'Not recorded' ? featured.percent : null, featured.days !== 'Not recorded' ? featured.days : null].filter(Boolean);
  return parts.length ? `${countText} · ${featured.name} ${parts.join(', ')}` : `${countText} · ${featured.name}`;
}

function slotNextChange(slotsNow: ResearchSlotsReply | undefined, hour: number): number {
  if (!slotsNow) return Infinity;
  let next = Infinity;
  for (const slot of slotsNow.slots) {
    if (slot.state.details?.remainingDays != null) {
      const boundary = slot.state.firstHour + nextDayAfter(hour - slot.state.firstHour);
      if (boundary > hour) next = Math.min(next, boundary);
    }
  }
  return next;
}

interface SlotsAtHit {
  readonly reply: ResearchSlotsReply | undefined;
  readonly live: LiveValues<string | number | bigint> | null | undefined;
  readonly pending: boolean;
}

function slotsAtHit(readView: BoundedReadView, at: number): SlotsAtHit {
  if (at >= readView.hour) {
    return { reply: readView.panel<ResearchSlotsReply>('slots'), live: readView.live('slots'), pending: false };
  }
  const reply = readView.hover<ResearchSlotsReply>('hover', at)?.data;
  return { reply, live: null, pending: !reply };
}

function slotClosedHour(slot: ResearchSlotHistory, coverage: readonly HourSpan[], hour: number): number | null {
  const last = slot.availability.at(-1);
  if (!last || last.lastHour >= hour) return null;
  const cov = coverage.find(span => span.firstHour <= last.lastHour && last.lastHour <= span.lastHour);
  return cov && cov.lastHour > last.lastHour ? last.lastHour + 1 : null;
}

export const researchProjection: TimelineProjection<ResearchTimelineOptions> = registerFamilyProjection<TimelineProjection<ResearchTimelineOptions>>({
  family: 'timeline',
  kind: 'research',

  nextChange(readView) {
    const data = readView.panel<ResearchData>('main');
    const slotsNow = readView.panel<ResearchSlotsReply>('slots');
    const historyNext = data ? nextBoundaryAfter(researchCache(data).boundaries, readView.hour) : Infinity;
    return Math.min(historyNext, slotNextChange(slotsNow, readView.hour));
  },

  project(readView, opt): TimelineModel {
    const hour = readView.hour;
    const data = readView.panel<ResearchData>('main');
    const slotsNow = readView.panel<ResearchSlotsReply>('slots');
    const live = readView.live('slots');
    const base = { hour, firstHour: opt.firstHour, labelWidth: 96, iconsOnBars: false };
    if (!data) return timelineModel(base);

    const cache = researchCache(data);
    const nextChangeAt = Math.min(nextBoundaryAfter(cache.boundaries, hour), slotNextChange(slotsNow, hour));
    const warnings = [...new Set([...data.warnings, ...(slotsNow?.warnings ?? [])])];
    const coverage = coverageAt(data.history.coverage, hour);
    if (!coverage.length) {
      return timelineModel({
        ...base,
        empty: 'Research history was not recorded at this date.',
        emptyKind: 'unavailable',
        warnings,
        nextChangeAt,
      });
    }

    const slots = researchAt(data.history, hour).filter(slot => slot.availability.length > 0).sort((a, b) => a.index - b.index);
    if (!slots.length) {
      return timelineModel({
        ...base,
        empty: `No research slots recorded by ${formatDateOnly(hour)}.`,
        emptyKind: 'empty',
        warnings,
        nextChangeAt,
      });
    }

    const chosen = new Set(contentSet(opt.folders).map(contentKey));
    const dimmed = (token: string, databaseIndex?: number | null) =>
      chosen.size > 0 && !chosen.has(researchFolderAt(cache.looks, token, databaseIndex)?.key ?? '');
    const unobserved = unobservedSpans(coverage, opt.firstHour, hour);
    const readBySlot = new Map<number, ResearchSlotNow>((slotsNow?.slots ?? []).map(item => [item.index, item]));

    let completedCount = 0;
    const lanes: TimelineLane[] = slots.map(slot => {
      const openedHour = slot.availability[0]!.firstHour;
      const closedHour = slotClosedHour(slot, coverage, hour);
      const spans: TimelineSpan[] = [];
      let hasOpenEntry = false;

      for (const entry of slot.entries) {
        const isOpen = entry.outcome === 'active';
        if (isOpen) hasOpenEntry = true;
        const isDim = dimmed(entry.token, entry.databaseIndex);
        if (entry.outcome === 'completed' && !isDim) completedCount++;
        const art = researchArt(cache.artwork, entry.token, entry.databaseIndex);
        const techName = contentTitle(art?.name, entry.token, 'Technology name unavailable');
        const folderName = researchFolderAt(cache.looks, entry.token, entry.databaseIndex)?.name ?? null;
        const heldBefore = entry.firstObservedHour <= opt.firstHour && entry.startedAfterHour === null;
        const startBy = !heldBefore && entry.startedAfterHour === null;
        const endUnseen = !isOpen && inHourSpans(unobserved, entry.lastObservedHour + 1);
        const lastHour = isOpen ? hour : Math.max(entry.firstObservedHour, (entry.endedByHour ?? entry.lastObservedHour) - 1);
        const color = isDim
          ? CHART_MEASURE_COLOURS.other
          : entry.outcome === 'switched' || entry.outcome === 'unknown'
            ? CHART_MEASURE_COLOURS.mutedFallback
            : CHART_MEASURE_COLOURS.intelCoverage;

        const meta: ResearchSpanMeta = {
          slotIndex: slot.index,
          entry,
          token: entry.token,
          databaseIndex: entry.databaseIndex,
          techName,
          folderName,
          description: art?.description ?? null,
          openedHour,
          unjoined: false,
        };
        spans.push({
          id: `${slot.index}:${entry.firstObservedHour}:${entry.token}`,
          firstHour: entry.firstObservedHour,
          lastHour: Math.min(lastHour, hour),
          color,
          label: techName,
          ...(heldBefore ? { heldBefore: true } : startBy ? { startBy: true } : {}),
          ...(endUnseen ? { endUnseen: true } : {}),
          open: isOpen,
          meta,
        });
      }

      const nowRead = readBySlot.get(slot.index);
      if (!hasOpenEntry && nowRead && isUnjoinedResearch(nowRead.state)) {
        const token = nowRead.state.token;
        const databaseIndex = nowRead.state.databaseIndex;
        const techName = contentTitle(nowRead.definition?.name, token, 'Technology name unavailable');
        const folderName = researchFolderAt(cache.looks, token, databaseIndex)?.name ?? null;
        const isDim = dimmed(token, databaseIndex);
        const meta: ResearchSpanMeta = {
          slotIndex: slot.index,
          entry: null,
          token,
          databaseIndex,
          techName,
          folderName,
          description: nowRead.definition?.description ?? null,
          openedHour,
          unjoined: true,
        };
        spans.push({
          id: `${slot.index}:unjoined:${nowRead.state.firstHour}:${token}`,
          firstHour: Math.min(nowRead.state.firstHour, hour),
          lastHour: hour,
          color: isDim ? CHART_MEASURE_COLOURS.other : CHART_MEASURE_COLOURS.intelCoverage,
          label: techName,
          startBy: true,
          open: true,
          meta,
        });
      }

      const label = `Slot ${slot.index + 1}`;
      const openingSpan = coverage.find(span => span.firstHour <= openedHour && openedHour <= span.lastHour);
      const openedRecorded = !!openingSpan && openedHour > openingSpan.firstHour;
      const laneMeta: ResearchLaneMeta = { slotIndex: slot.index, openedHour, openedRecorded, closedHour };
      return {
        key: `slot:${slot.index}`,
        kind: 'item',
        label,
        spans,
        ...(openedRecorded ? { openedMarker: { hour: openedHour, label: 'opened' } } : {}),
        auditValue: `${label}|${spans.length}`,
        meta: laneMeta,
      };
    });

    const legend: LegendStripEntry[] = [
      { label: 'Completed', color: mix(CHART_MEASURE_COLOURS.intelCoverage, opt.surface, 0.5), swatch: 'square' },
      { label: 'In progress', color: CHART_MEASURE_COLOURS.intelCoverage, swatch: 'square' },
      { label: 'Idle', outlined: true },
      ...(unobserved.length ? [{ label: 'Not recorded', hatch: true } satisfies LegendStripEntry] : []),
    ];
    const headNote = headNoteOf(slotsNow, live, cache.artwork, hour, slots.length);
    return timelineModel({
      ...base,
      lanes,
      unobserved,
      legend,
      summaryText: `${formatScalar(completedCount, 'integer')} completed`,
      notes: headNote ? [headNote] : undefined,
      warnings,
      nextChangeAt,
    });
  },

  tooltip(model, hit, readView) {
    const lane = laneByKey(model, hit.laneKey);
    if (!lane) return null;
    const at = Math.min(hit.hour ?? model.hour, readView.hour);
    const data = readView.panel<ResearchData>('main');
    const slotsHit = slotsAtHit(readView, at);
    const span = spanAt(lane, hit);

    if (!span) {
      const laneMeta = lane.meta as ResearchLaneMeta | undefined;
      if (inHourSpans(model.unobserved, at)) {
        return {
          title: lane.label,
          hour: at,
          items: [{ label: 'Status', value: 'Not recorded', tone: 'muted' }],
        };
      }
      if (laneMeta && at < laneMeta.openedHour) {
        return {
          title: lane.label,
          hour: at,
          items: [
            { label: 'Status', value: 'Not yet opened', tone: 'muted' },
            { label: laneMeta.openedRecorded ? 'Opened' : 'Opened by', value: formatDateOnly(laneMeta.openedHour) },
          ],
        };
      }
      if (laneMeta?.closedHour != null && at >= laneMeta.closedHour) {
        return {
          title: lane.label,
          hour: at,
          items: [
            { label: 'Status', value: 'Closed', tone: 'muted' },
            { label: 'Closed by', value: formatDateOnly(laneMeta.closedHour) },
          ],
        };
      }
      const hitRead = laneMeta ? slotsHit.reply?.slots.find(item => item.index === laneMeta.slotIndex) : undefined;
      const saved = hitRead && hitRead.state.token === null ? slotNumbers(hitRead, at).saved : '';
      return {
        title: lane.label,
        hour: at,
        items: [
          { label: 'Status', value: 'Idle', tone: 'muted' },
          ...(saved ? [{ label: 'Saved progress', value: saved }] : []),
        ],
      };
    }

    const meta = span.meta as ResearchSpanMeta;
    const subtitle = meta.folderName ? `${meta.folderName} · Slot ${meta.slotIndex + 1}` : `Slot ${meta.slotIndex + 1}`;
    if (span.open) {
      const hitRead = slotsHit.reply?.slots.find(item => item.index === meta.slotIndex);
      const numbers = hitRead && hitRead.state.token === meta.token
        && (meta.entry === null || hitRead.state.entryFirstObservedHour === meta.entry.firstObservedHour)
        ? slotNumbers(hitRead, at, liveProgressOf(slotsHit.live, meta.slotIndex))
        : null;
      const reading = slotsHit.pending ? { value: 'Loading…', tone: 'muted' as const } : null;
      const unread = slotsHit.reply?.missing ?? null;
      const items = [
        { label: 'Status', value: 'In progress' },
        ...(meta.entry
          ? [{
            label: 'Started',
            value: meta.entry.startedAfterHour !== null
              ? formatDateOnly(meta.entry.firstObservedHour)
              : `By ${formatDateOnly(meta.entry.firstObservedHour)}`,
          }]
          : []),
        {
          label: 'Progress',
          ...(reading ?? (numbers && numbers.fraction !== null
            ? { value: numbers.percent }
            : { missingReason: unread ?? 'Research progress not recorded' })),
        },
        {
          label: 'Days left',
          ...(reading ?? (numbers && numbers.left !== null
            ? { value: formatScalar(numbers.left, 'integer') }
            : { missingReason: unread ?? 'Days left estimate not recorded' })),
        },
      ];
      const note = meta.unjoined
        ? 'This recording does not identify the assignment, so its start date is unknown.'
        : numbers?.left != null
          ? 'Days left is the game’s estimate at this hour’s research speed.'
          : undefined;
      return {
        title: meta.techName,
        subtitle,
        hour: at,
        items,
        ...(meta.description ? { description: meta.description } : {}),
        ...(note ? { note } : {}),
      };
    }

    const entry = meta.entry!;
    if (entry.outcome === 'completed' && entry.endedByHour !== null && data) {
      const clippedSlots = researchAt(data.history, model.hour);
      const slot = clippedSlots.find(item => item.index === meta.slotIndex);
      if (slot) {
        const row = completedRow(clippedSlots, slot, entry, entry.endedByHour);
        const items = [
          { label: 'Completed', value: completedBy(entry) ? `By ${formatDateOnly(row.hour)}` : formatDateOnly(row.hour) },
          { label: 'Started', value: row.started.known ? formatDateOnly(row.started.hour) : `By ${formatDateOnly(row.started.hour)}` },
          ...(row.tookHours !== null ? [{ label: 'Took', value: researchDays(row.tookHours) }] : []),
          ...(row.startSlot !== null ? [{ label: 'Started in', value: `Slot ${row.startSlot + 1}` }] : []),
        ];
        return {
          title: meta.techName,
          subtitle,
          hour: at,
          items,
          ...(meta.description ? { description: meta.description } : {}),
        };
      }
    }

    const statusLabel = entry.outcome === 'switched' ? 'Switched' : 'Outcome not recorded';
    return {
      title: meta.techName,
      subtitle,
      hour: at,
      items: [
        { label: 'Status', value: statusLabel },
        { label: 'Started', value: entry.startedAfterHour !== null ? formatDateOnly(entry.firstObservedHour) : `By ${formatDateOnly(entry.firstObservedHour)}` },
        { label: entry.outcome === 'switched' ? 'Switched' : 'Last seen', value: formatDateOnly(entry.endedByHour ?? entry.lastObservedHour) },
      ],
      ...(meta.description ? { description: meta.description } : {}),
    };
  },
});

export const researchDeclaration: TimelineWidgetDeclaration<ResearchTimelineOptions> = {
  family: 'timeline',
  kind: 'research',
  label: 'Research history',
  subject: 'research',
  emptyCampaign: 'Load a campaign and choose a country to see its research.',
  icon: 'flask',
  countries: { mode: 'single', choose: 'Choose a country to see its research.' },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => {
    const tag = ctx.countryTag;
    if (!tag) return { owner: `research:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:`, entries: [] };
    const country = countryFromKey(tag);
    return {
      owner: `research:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${tag}`,
      entries: [
        {
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'keyed', keys: [tag] },
            product: 'research',
            followQuery: null,
            params: { branchId: ctx.branch.id, country },
            buildSpanParams: params => params,
            provenInterval: wholeBranchProof,
            validate: validateResearchReply,
          },
        },
        {
          name: 'slots',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'point',
            channel: 'panel',
            coverage: { mode: 'keyed', keys: [tag] },
            product: 'research-slots',
            followQuery: { panel: 'research-slots', branchId: ctx.branch.id, country },
            params: { branchId: ctx.branch.id, country },
            validate: validateResearchSlots,
          },
        },
        {
          name: 'hover',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'exact-hover',
            product: 'research-slots',
            params: { branchId: ctx.branch.id, country },
            validate: validateResearchSlots,
          },
        },
      ],
    };
  },
  projection: researchProjection,
  projectionOptions: (ctx, extra) => ({
    folders: String(ctx.options.folders ?? ''),
    firstHour: extra.extent[0],
    surface: extra.surface,
  }),
};
