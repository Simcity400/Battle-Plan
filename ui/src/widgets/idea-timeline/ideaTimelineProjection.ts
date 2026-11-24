import { formatDay } from '../../dashboard/calendar.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { ideaName, ideaNameMissing, validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import { startUnseen } from '../../charts/timeline/runState.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { clipSpansToHour, inHourSpans, nextBoundaryAfter, paletteAt, urlArtHandle } from '../../families/timeline/helpers.ts';
import type { TimelineLaneModel, TimelineModel, TimelineProjection, TimelineSpanModel } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { Artwork, IdeaHistoryReply, IdeaInfo, IdeaSpan } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { chosenCategory, lawLevel, UNIDENTIFIED_HOLDER } from './ideaTimelineData.ts';

export interface IdeaTimelineProjectionOptions {
  readonly category: string;
  readonly iconsOnBars: boolean;
  readonly branchFirst: number;
  readonly surface: string;
}

function holderKey(token: string, character: number, script: string, name: string): string {
  return JSON.stringify([token, character, script, name]);
}

function collectBoundaries(reply: IdeaHistoryReply): number[] {
  const set = new Set<number>();
  for (const s of reply.spans) {
    set.add(s.firstHour);
    set.add(s.lastHour + 1);
  }
  for (const g of reply.unobserved) {
    set.add(g.firstHour);
    set.add(g.lastHour + 1);
  }
  return [...set].sort((a, b) => a - b);
}

export const ideaTimelineProjection: TimelineProjection<IdeaTimelineProjectionOptions> = registerTimelineProjection({
  kind: 'idea-timeline',

  nextChange(readView) {
    const reply = readView.panel<IdeaHistoryReply>('main');
    if (!reply) return Infinity;
    return nextBoundaryAfter(collectBoundaries(reply), readView.hour);
  },

  project(readView, opt): TimelineModel {
    const hour = readView.hour;
    const reply = readView.panel<IdeaHistoryReply>('main');
    const extent: [number, number] = [opt.branchFirst, Math.max(opt.branchFirst + 1, hour)];
    if (!reply) {
      return { mode: 'lanes', hour, extent, playhead: hour, lanes: [], empty: 'Loading ideas\u2026' };
    }
    const category = chosenCategory(reply.categories, opt.category);
    if (!category) {
      return {
        mode: 'lanes',
        hour,
        extent,
        playhead: hour,
        lanes: [],
        empty: opt.category ? 'Choose another idea category in settings.' : 'No idea categories available.',
        emptyKind: 'unavailable',
        warnings: reply.warnings,
      };
    }

    const infos = new Map<string, IdeaInfo>(reply.ideas.map(info => [info.token, info]));
    const art = new Map<string, Artwork>(reply.artwork.map(item => [item.token, item]));
    const holders = new Map<string, Artwork>(reply.holders.map(item => [holderKey(item.token, item.character, item.characterToken, item.name), item.artwork]));
    const unobserved = clipSpansToHour(reply.unobserved, hour);
    const lanes: TimelineLaneModel[] = [];
    const slotColorIndex = new Map<string, number>();
    let colorSeq = 0;

    const toSpanModel = (item: IdeaSpan, color: string): TimelineSpanModel => {
      const clippedLast = Math.min(item.lastHour, hour);
      const openAtPlayhead = item.lastHour >= hour;
      const ended = !openAtPlayhead;
      const heldBefore = startUnseen(item.firstHour, opt.branchFirst, reply.unobserved);
      const fadedEnd = ended && inHourSpans(reply.unobserved, item.lastHour + 1);
      let label: string;
      let iconUrl: string | null = null;
      let iconMissing = false;
      if (item.character) {
        const look = holders.get(holderKey(item.token, item.character.id, item.character.token, item.character.name));
        label = look?.name ?? UNIDENTIFIED_HOLDER;
        iconUrl = look?.icon ?? null;
        iconMissing = !iconUrl;
      } else {
        const look = art.get(item.token);
        label = ideaName(look, infos.get(item.token), 'Idea name unavailable');
        iconUrl = look?.icon ?? null;
        iconMissing = !iconUrl;
      }
      return {
        id: `${item.token}:${item.firstHour}`,
        firstHour: item.firstHour,
        lastHour: Math.max(item.firstHour, clippedLast),
        color,
        label,
        icon: urlArtHandle(iconUrl),
        iconMissing,
        heldBefore,
        fadedEnd,
        ended,
        openAtPlayhead,
        meta: { token: item.token, slot: item.slot ?? infos.get(item.token)?.slot ?? null, rawFirst: item.firstHour, rawLast: item.lastHour },
      };
    };

    for (const slot of category.slots) {
      const slotLabel = contentTitle(slot.name, slot.token, 'Slot name unavailable');
      if (!slotColorIndex.has(slot.token)) slotColorIndex.set(slot.token, colorSeq++);
      const baseColor = paletteAt(opt.surface, slotColorIndex.get(slot.token)!);
      const held = reply.spans
        .filter(item => item.firstHour <= hour && (item.slot ?? infos.get(item.token)?.slot) === slot.token)
        .sort((a, b) => a.firstHour - b.firstHour);

      if (!slot.character) {
        const spans = held.map(item => {
          const levelIdx = slot.laws.indexOf(item.token);
          const color = slot.laws.length > 0 && levelIdx >= 0 ? paletteAt(opt.surface, levelIdx) : baseColor;
          return toSpanModel(item, color);
        });
        const current = spans.find(s => s.openAtPlayhead) ?? null;
        const subtitle = current
          ? current.label
          : inHourSpans(reply.unobserved, hour)
            ? 'Not recorded'
            : 'Empty';
        lanes.push({
          key: slot.token,
          kind: 'standard',
          label: slotLabel,
          subtitle,
          icon: current?.icon ?? null,
          iconMissing: current?.iconMissing ?? false,
          iconSize: 24,
          spans,
          auditValue: `${slotLabel}|${subtitle}`,
        });
        continue;
      }

      const seats: { spans: TimelineSpanModel[]; last: IdeaSpan | null }[] = Array.from(
        { length: Math.max(1, slot.seats) },
        () => ({ spans: [], last: null }),
      );
      for (const item of held) {
        let seat = item.character
          ? seats.find(({ last }) => last?.character?.id === item.character!.id && last.lastHour + 1 === item.firstHour)
          : undefined;
        seat ??= seats.find(({ last }) => (last?.lastHour ?? -Infinity) < item.firstHour);
        if (!seat) {
          seat = { spans: [], last: null };
          seats.push(seat);
        }
        seat.spans.push(toSpanModel(item, baseColor));
        seat.last = item;
      }
      seats.forEach(({ spans }, idx) => {
        const current = spans.find(s => s.openAtPlayhead) ?? null;
        const subtitle = current
          ? current.label
          : inHourSpans(reply.unobserved, hour)
            ? 'Not recorded'
            : 'Empty';
        lanes.push({
          key: `${slot.token}#${idx}`,
          kind: 'standard',
          label: slotLabel,
          subtitle,
          icon: current?.icon ?? null,
          iconMissing: current?.iconMissing ?? false,
          iconSize: 24,
          spans,
          auditValue: `${slotLabel}|${subtitle}`,
        });
      });
    }

    return {
      mode: 'lanes',
      hour,
      extent,
      playhead: hour,
      lanes,
      unobserved,
      iconsOnBars: opt.iconsOnBars,
      labelWidth: 196,
      warnings: reply.warnings,
      nextChangeAt: nextBoundaryAfter(collectBoundaries(reply), hour),
    };
  },

  tooltip(model, target, hour, readView, opt) {
    const reply = readView.panel<IdeaHistoryReply>('main');
    const lane = model.lanes?.find(l => l.key === (target.laneKey ?? target.key));
    if (!lane) return null;
    const span = target.spanId
      ? lane.spans?.find(s => s.id === target.spanId)
      : lane.spans?.find(s => s.firstHour <= hour && hour <= s.lastHour);
    if (!span) {
      const missing = inHourSpans(reply?.unobserved, hour);
      return {
        title: lane.label,
        hour,
        items: [{ label: missing ? 'Not recorded' : 'Empty', value: formatDay(hour), tone: 'muted' }],
      };
    }
    const meta = span.meta as { token?: string } | undefined;
    const info = meta?.token ? reply?.ideas.find(i => i.token === meta.token) : undefined;
    const artItem = meta?.token ? reply?.artwork.find(a => a.token === meta.token) : undefined;
    const category = reply ? chosenCategory(reply.categories, opt.category) : null;
    const level = category && info ? lawLevel(category, info) : null;
    const why = meta?.token ? ideaNameMissing(artItem, info) : null;
    return {
      title: span.label ?? lane.label,
      subtitle: level ? `${lane.label} \u00b7 level ${level}` : lane.label,
      hour,
      items: [
        { label: span.heldBefore ? 'Held before' : 'Since', value: formatDay(span.firstHour) },
        { label: 'Until', value: span.openAtPlayhead ? 'Active' : formatDay(span.lastHour) },
      ],
      ...(artItem?.description ? { description: artItem.description } : {}),
      ...(why ? { note: why } : {}),
    };
  },
});

export const ideaTimelineDeclaration: TimelineWidgetDeclaration<IdeaTimelineProjectionOptions> = {
  family: 'timeline',
  kind: 'idea-timeline',
  label: 'Idea timeline',
  subject: 'idea timeline',
  emptyCampaign: 'Load a campaign and choose a country to see its idea history.',
  icon: 'layers',
  optionFields: OPTIONS,
  countries: { mode: 'single', choose: 'Choose a country to see its idea history.' },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => ({
    owner: `idea-timeline:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
    entries: ctx.countryTag
      ? [{
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'keyed', keys: [ctx.countryTag] },
            product: 'idea-history',
            followQuery: null,
            params: { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) },
            buildSpanParams: base => base,
            validate: validateIdeaHistoryReply,
          },
        }]
      : [],
    allowEmptyFollow: true,
  }),
  projection: ideaTimelineProjection,
  projectionOptions: ctx => ({
    category: String(ctx.options.category ?? ''),
    iconsOnBars: ctx.options.iconsOnBars !== false,
    branchFirst: ctx.branch.firstHour,
    surface: '#16181d',
  }),
};
