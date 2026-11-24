import { liveKey, type LiveValues } from '../../charts/live.ts';
import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { runAt } from '../../charts/series.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { LegendStripEntry } from '../../draw/kit.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { inHourSpans, timelineModel } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineModel, type TimelineProjection, type TimelineSpan } from '../../families/timeline/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { formatDateOnly, formatScalar } from '../../families/valueFormat.ts';
import type { HourSpan, ResearchBound, ResearchHistoryReply, ResearchItem, ResearchSlotsReply } from '../../generated/dashboard';
import { mix } from '../../shell/appearance/color.ts';
import { isUnjoinedResearch, researchHistoryProof, researchLooks, validateResearchHistory, validateResearchSlots, type ResearchLooks } from './researchData.ts';
import { slotNumbers } from './researchSlots.ts';

interface ResearchTimelineOptions {
  readonly folders: string;
  readonly firstHour: number;
  readonly surface: string;
}

interface ResearchSpanMeta {
  readonly slotIndex: number;
  readonly item: ResearchItem | null;
  readonly token: string;
  readonly techName: string;
  readonly nameMissing: string | null;
  readonly folderName: string | null;
  readonly description: string | null;
}

interface ResearchLaneMeta {
  readonly slotIndex: number;
  readonly openedHour: number | null;
  readonly openedRecorded: boolean;
  readonly availability: readonly HourSpan[];
}

const nextDayAfter = (hour: number) => (Math.floor(hour / 24) + 1) * 24;

function throughHour(spans: readonly HourSpan[], answered: number, hour: number): HourSpan[] {
  return spans.map(span => (span.lastHour === answered && hour > answered ? { firstHour: span.firstHour, lastHour: hour } : span));
}

function unobservedSpans(coverage: readonly HourSpan[], firstHour: number, hour: number): HourSpan[] {
  if (!coverage.length) return hour >= firstHour ? [{ firstHour, lastHour: hour }] : [];
  const out: HourSpan[] = [];
  if (coverage[0]!.firstHour > firstHour) out.push({ firstHour, lastHour: coverage[0]!.firstHour - 1 });
  for (let at = 1; at < coverage.length; at++) {
    const gap = { firstHour: coverage[at - 1]!.lastHour + 1, lastHour: coverage[at]!.firstHour - 1 };
    if (gap.lastHour >= gap.firstHour) out.push(gap);
  }
  const last = coverage[coverage.length - 1]!.lastHour;
  if (last < hour) out.push({ firstHour: last + 1, lastHour: hour });
  return out;
}

function liveProgressOf(live: LiveValues<string | number | bigint> | null | undefined, slotIndex: number): number | undefined {
  const raw = live?.get(liveKey(String(slotIndex), 'progress'));
  return typeof raw === 'number' ? raw : typeof raw === 'bigint' ? Number(raw) : undefined;
}

function headNoteOf(
  slotsNow: ResearchSlotsReply | undefined,
  live: LiveValues<string | number | bigint> | null | undefined,
  looks: ResearchLooks,
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
    const name = isUnjoinedResearch(read.state)
      ? read.definition?.name ?? read.state.token
      : looks.name(read.state.token, read.state.databaseIndex).label;
    const sort = numbers.left !== null ? -numbers.left : (numbers.fraction ?? -1) * 1000;
    if (!featured || sort > featured.sort) featured = { name, percent: numbers.percent, days: numbers.days, sort };
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

function boundText(bound: ResearchBound): string {
  return bound.kind === 'exact' || bound.kind === 'lastSeen' ? formatDateOnly(bound.hour) : `By ${formatDateOnly(bound.hour)}`;
}

function nextChangeOf(reply: ResearchHistoryReply | undefined, slotsNow: ResearchSlotsReply | undefined, hour: number): number {
  return Math.min(reply?.nextChange ?? Infinity, slotNextChange(slotsNow, hour));
}

function itemSpans(slotIndex: number, item: ResearchItem, reply: ResearchHistoryReply, hour: number, meta: ResearchSpanMeta, colour: string): TimelineSpan[] {
  const open = item.outcome === 'active';
  const last = item.pieces.length - 1;
  return item.pieces.map((piece, at) => {
    const reaches = open && at === last && piece.lastHour === reply.hour;
    return {
      id: `${slotIndex}:${item.firstObservedHour}:${item.token}:${item.databaseIndex}:${at}`,
      firstHour: piece.firstHour,
      lastHour: reaches ? hour : piece.lastHour,
      color: colour,
      label: meta.techName,
      ...(at === 0 && item.start.kind === 'heldBefore' ? { heldBefore: true } : at === 0 && item.start.kind === 'by' ? { startBy: true } : {}),
      ...(at === last && item.end?.kind === 'lastSeen' ? { endUnseen: true } : {}),
      open: reaches,
      meta,
    };
  });
}

export const researchProjection: TimelineProjection<ResearchTimelineOptions> = registerFamilyProjection<TimelineProjection<ResearchTimelineOptions>>({
  family: 'timeline',
  kind: 'research',

  nextChange(readView) {
    return nextChangeOf(readView.panel<ResearchHistoryReply>('main'), readView.panel<ResearchSlotsReply>('slots'), readView.hour);
  },

  project(readView, opt): TimelineModel {
    const hour = readView.hour;
    const reply = readView.panel<ResearchHistoryReply>('main');
    const slotsNow = readView.panel<ResearchSlotsReply>('slots');
    const live = readView.live('slots');
    const base = { hour, firstHour: opt.firstHour, labelWidth: 96, iconsOnBars: false };
    if (!reply) return timelineModel(base);

    const nextChangeAt = nextChangeOf(reply, slotsNow, hour);
    const warnings = [...new Set([...reply.warnings, ...(slotsNow?.warnings ?? [])])];
    if (reply.unrecorded) {
      return timelineModel({ ...base, empty: `${reply.unrecorded}.`, emptyKind: 'unavailable', warnings, nextChangeAt });
    }
    if (!reply.slots.length) {
      return timelineModel({
        ...base,
        ...(reply.partial
          ? { empty: `${reply.partial}.`, emptyKind: 'unavailable' as const }
          : { empty: `No research slots recorded by ${formatDateOnly(hour)}.`, emptyKind: 'empty' as const }),
        warnings,
        nextChangeAt,
      });
    }

    const looks = researchLooks(reply);
    const chosen = new Set(contentSet(opt.folders).map(contentKey));
    const coverage = throughHour(reply.coverage, reply.hour, hour);
    const unobserved = unobservedSpans(coverage, opt.firstHour, hour);
    const readBySlot = new Map((slotsNow?.slots ?? []).map(item => [item.index, item]));
    const completedColour = mix(CHART_MEASURE_COLOURS.intelCoverage, opt.surface, 0.5);

    const lanes: TimelineLane[] = reply.slots.map(slot => {
      const availability = throughHour(slot.availability, reply.hour, hour);
      const openedHour = availability[0]?.firstHour ?? null;
      const spans: TimelineSpan[] = [];
      let hasOpenItem = false;
      for (const item of slot.items) {
        if (item.outcome === 'active') hasOpenItem = true;
        const folder = looks.folder(item.token, item.databaseIndex);
        const dim = chosen.size > 0 && !chosen.has(folder?.key ?? '');
        const name = looks.name(item.token, item.databaseIndex);
        const colour = dim
          ? CHART_MEASURE_COLOURS.other
          : item.outcome === 'switched' || item.outcome === 'unknown'
            ? CHART_MEASURE_COLOURS.mutedFallback
            : CHART_MEASURE_COLOURS.intelCoverage;
        const meta: ResearchSpanMeta = {
          slotIndex: slot.index,
          item,
          token: item.token,
          techName: name.label,
          nameMissing: name.missing,
          folderName: folder?.name ?? null,
          description: name.description,
        };
        spans.push(...itemSpans(slot.index, item, reply, hour, meta, colour));
      }

      const nowRead = readBySlot.get(slot.index);
      if (!hasOpenItem && nowRead && isUnjoinedResearch(nowRead.state)) {
        const token = nowRead.state.token;
        const folder = looks.folder(token, nowRead.state.databaseIndex);
        const dim = chosen.size > 0 && !chosen.has(folder?.key ?? '');
        const meta: ResearchSpanMeta = {
          slotIndex: slot.index,
          item: null,
          token,
          techName: nowRead.definition?.name ?? token,
          nameMissing: nowRead.definition?.name == null ? reply.unnamed : null,
          folderName: folder?.name ?? null,
          description: nowRead.definition?.description ?? null,
        };
        spans.push({
          id: `${slot.index}:unjoined:${nowRead.state.firstHour}:${token}`,
          firstHour: Math.min(nowRead.state.firstHour, hour),
          lastHour: hour,
          color: dim ? CHART_MEASURE_COLOURS.other : CHART_MEASURE_COLOURS.intelCoverage,
          label: meta.techName,
          startBy: true,
          open: true,
          meta,
        });
      }

      const label = `Slot ${slot.index + 1}`;
      const openingSpan = openedHour === null ? undefined : coverage.find(span => span.firstHour <= openedHour && openedHour <= span.lastHour);
      const openedRecorded = openedHour !== null && !!openingSpan && openedHour > openingSpan.firstHour;
      const laneMeta: ResearchLaneMeta = { slotIndex: slot.index, openedHour, openedRecorded, availability };
      return {
        key: `slot:${slot.index}`,
        kind: 'item',
        label,
        spans,
        ...(openedRecorded && openedHour !== null ? { openedMarker: { hour: openedHour, label: 'opened' } } : {}),
        auditValue: `${label}|${availability.map(span => `${span.firstHour}-${span.lastHour}`).join(',')}`,
        meta: laneMeta,
      };
    });

    const legend: LegendStripEntry[] = [
      { label: 'Completed', color: completedColour, swatch: 'square' },
      { label: 'In progress', color: CHART_MEASURE_COLOURS.intelCoverage, swatch: 'square' },
      { label: 'Idle', outlined: true },
      ...(unobserved.length ? [{ label: 'Not recorded', hatch: true } satisfies LegendStripEntry] : []),
    ];
    const headNote = headNoteOf(slotsNow, live, looks, hour, lanes.length);
    return timelineModel({
      ...base,
      lanes,
      unobserved,
      legend,
      summaryText: `${formatScalar(reply.completed, 'integer')} completed`,
      summaryPartial: reply.partial,
      notes: headNote ? [headNote] : undefined,
      warnings,
      nextChangeAt,
    });
  },

  tooltip(model, hit, readView) {
    const lane = laneByKey(model, hit.laneKey);
    if (!lane) return null;
    const at = Math.min(hit.hour ?? model.hour, readView.hour);
    const slotsHit = slotsAtHit(readView, at);
    const span = spanAt(lane, hit);

    if (!span) {
      const laneMeta = lane.meta as ResearchLaneMeta | undefined;
      const status = (value: string, extra: readonly { label: string; value: string }[] = []) => ({
        title: lane.label,
        hour: at,
        items: [{ label: 'Status', value, tone: 'muted' as const }, ...extra],
      });
      if (inHourSpans(model.unobserved, at)) return status('Not recorded');
      if (laneMeta?.openedHour != null && at < laneMeta.openedHour) {
        return status('Not yet opened', [{ label: laneMeta.openedRecorded ? 'Opened' : 'Opened by', value: formatDateOnly(laneMeta.openedHour) }]);
      }
      if (laneMeta && laneMeta.availability.length && !runAt(laneMeta.availability, at)) {
        const before = laneMeta.availability.filter(span => span.lastHour < at).at(-1);
        return status('Closed', before ? [{ label: 'Closed by', value: formatDateOnly(before.lastHour + 1) }] : []);
      }
      const hitRead = laneMeta ? slotsHit.reply?.slots.find(item => item.index === laneMeta.slotIndex) : undefined;
      const saved = hitRead && hitRead.state.token === null ? slotNumbers(hitRead, at).saved : '';
      return status('Idle', saved ? [{ label: 'Saved progress', value: saved }] : []);
    }

    const meta = span.meta as ResearchSpanMeta;
    const subtitle = meta.folderName ? `${meta.folderName} · Slot ${meta.slotIndex + 1}` : `Slot ${meta.slotIndex + 1}`;
    const naming = meta.nameMissing ? [{ label: 'Name', missingReason: `${meta.nameMissing} (key ${meta.token})` }] : [];
    const described = meta.description ? { description: meta.description } : {};
    const item = meta.item;
    if (!item || item.outcome === 'active') {
      const hitRead = slotsHit.reply?.slots.find(read => read.index === meta.slotIndex);
      const numbers = hitRead && hitRead.state.token === meta.token
        && (item === null || hitRead.state.entryFirstObservedHour === item.firstObservedHour)
        ? slotNumbers(hitRead, at, liveProgressOf(slotsHit.live, meta.slotIndex))
        : null;
      const reading = slotsHit.pending ? { value: 'Loading…', tone: 'muted' as const } : null;
      const unread = slotsHit.reply?.missing ?? null;
      const note = !item
        ? 'This recording does not identify the assignment, so its start date is unknown.'
        : numbers?.left != null
          ? 'Days left is the game’s estimate at this hour’s research speed.'
          : undefined;
      return {
        title: meta.techName,
        subtitle,
        hour: at,
        items: [
          { label: 'Status', value: 'In progress' },
          ...(item ? [{ label: 'Started', value: boundText(item.start) }] : []),
          {
            label: 'Progress',
            ...(reading ?? (numbers && numbers.fraction !== null ? { value: numbers.percent } : { missingReason: unread ?? 'Research progress not recorded' })),
          },
          {
            label: 'Days left',
            ...(reading ?? (numbers && numbers.left !== null ? { value: formatScalar(numbers.left, 'integer') } : { missingReason: unread ?? 'Days left estimate not recorded' })),
          },
          ...naming,
        ],
        ...described,
        ...(note ? { note } : {}),
      };
    }

    if (item.outcome === 'completed' && item.end) {
      const began = item.origin?.start ?? item.start;
      return {
        title: meta.techName,
        subtitle,
        hour: at,
        items: [
          { label: 'Completed', value: boundText(item.end) },
          { label: 'Started', value: boundText(began) },
          item.tookDays !== null
            ? { label: 'Took', value: `${formatScalar(item.tookDays, 'integer')} ${item.tookDays === 1 ? 'day' : 'days'}` }
            : { label: 'Took', missingReason: 'The start or the completion date is an observation bound' },
          ...(item.origin ? [{ label: 'Started in', value: `Slot ${item.origin.slot + 1}` }] : []),
          ...naming,
        ],
        ...described,
      };
    }

    const switched = item.outcome === 'switched';
    return {
      title: meta.techName,
      subtitle,
      hour: at,
      items: [
        { label: 'Status', value: switched ? 'Switched' : 'Outcome not recorded' },
        { label: 'Started', value: boundText(item.start) },
        ...(item.end ? [{ label: switched ? 'Switched' : 'Last seen', value: boundText(item.end) }] : []),
        ...naming,
      ],
      ...described,
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
    const subject = ctx.countrySubject;
    if (!subject) return { owner: `research:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:`, entries: [] };
    const country = countryFromKey(subject);
    const folders = contentSet(String(ctx.options.folders ?? '')).map(contentKey);
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    return {
      owner: `research:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${subject}`,
      entries: [
        {
          ...common,
          name: 'main',
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'keyed', keys: [subject] },
            product: 'research-history',
            followQuery: null,
            params: { branchId: ctx.branch.id, country, folders },
            buildSpanParams: (params, span) => ({ ...params, hour: span.lastHour }),
            provenInterval: researchHistoryProof,
            dynamicKeys: ['hour'],
            validate: validateResearchHistory,
          },
        },
        {
          ...common,
          name: 'slots',
          read: {
            shape: 'point',
            channel: 'panel',
            coverage: { mode: 'keyed', keys: [subject] },
            product: 'research-slots',
            followQuery: { panel: 'research-slots', branchId: ctx.branch.id, country },
            params: { branchId: ctx.branch.id, country },
            validate: validateResearchSlots,
          },
        },
        {
          ...common,
          name: 'hover',
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
