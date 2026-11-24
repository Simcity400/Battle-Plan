import { formatDay } from '../../dashboard/calendar.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import { describeRun, startUnseen } from '../../charts/timeline/runState.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { clipSpansToHour, inHourSpans, nextBoundaryAfter, paletteAt, urlArtHandle } from '../../families/timeline/helpers.ts';
import type { TimelineLaneModel, TimelineModel, TimelineProjection, TimelineSpanModel } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { IdeaHistoryReply } from '../../generated/dashboard';
import { endedBy, spirits, type SpiritSort } from '../national-spirits/spiritsData.ts';
import { OPTIONS } from './definition.ts';

export interface SpiritHistoryProjectionOptions {
  readonly sort: SpiritSort;
  readonly showEnded: boolean;
  readonly iconsOnBars: boolean;
  readonly branchFirst: number;
  readonly surface: string;
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

export const spiritHistoryProjection: TimelineProjection<SpiritHistoryProjectionOptions> = registerTimelineProjection({
  kind: 'spirit-history',

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
      return { mode: 'lanes', hour, extent, playhead: hour, lanes: [], empty: 'Loading national spirits\u2026' };
    }
    const list = spirits(reply);
    const unobserved = clipSpansToHour(reply.unobserved, hour);
    const seen = list.filter(spirit => {
      const first = spirit.spans[0]?.firstHour ?? Infinity;
      if (first > hour) return false;
      return opt.showEnded || endedBy(spirit, hour, reply.unobserved) === null;
    });

    if (!seen.length) {
      const unplaced = new Set(reply.ideas.filter(info => !info.category).map(info => info.token));
      const hasUnclassified = reply.spans.some(span => unplaced.has(span.token) && span.firstHour <= hour);
      return {
        mode: 'lanes',
        hour,
        extent,
        playhead: hour,
        lanes: [],
        empty: hasUnclassified
          ? 'National spirits unavailable: some held ideas could not be classified.'
          : 'No national spirits recorded yet.',
        emptyKind: hasUnclassified ? 'unavailable' : 'empty',
        warnings: reply.warnings,
      };
    }

    const firstGained = (s: typeof seen[number]) => s.spans[0]?.firstHour ?? Infinity;
    const endKey = (s: typeof seen[number]) => endedBy(s, hour, reply.unobserved) ?? Infinity;
    seen.sort(
      opt.sort === 'name'
        ? (a, b) => a.name.localeCompare(b.name)
        : opt.sort === 'ended'
          ? (a, b) => endKey(a) - endKey(b) || firstGained(a) - firstGained(b)
          : (a, b) => firstGained(a) - firstGained(b) || a.name.localeCompare(b.name),
    );

    const barColor = paletteAt(opt.surface, 0);
    const words = {
      since: (h: number) => `Active since ${formatDay(h)}`,
      before: (h: number) => `Held before ${formatDay(h)}`,
      ended: (h: number) => `Ended ${formatDay(h)}`,
      endedBy: (h: number) => `Ended by ${formatDay(h)}`,
      lastSeen: (h: number) => `Last recorded ${formatDay(h)}`,
    };

    const lanes: TimelineLaneModel[] = seen.map(spirit => {
      const visibleSpans: TimelineSpanModel[] = [];
      for (const s of spirit.spans) {
        if (s.firstHour > hour) continue;
        const openAtPlayhead = s.lastHour >= hour;
        const ended = !openAtPlayhead;
        visibleSpans.push({
          id: `${spirit.token}:${s.firstHour}`,
          firstHour: s.firstHour,
          lastHour: Math.min(s.lastHour, hour),
          color: barColor,
          label: spirit.name,
          icon: urlArtHandle(spirit.icon),
          iconMissing: !spirit.icon,
          heldBefore: startUnseen(s.firstHour, opt.branchFirst, reply.unobserved),
          fadedEnd: ended && inHourSpans(reply.unobserved, s.lastHour + 1),
          ended,
          openAtPlayhead,
          meta: { token: spirit.token, firstHour: s.firstHour, lastHour: s.lastHour },
        });
      }
      const subtitle = describeRun(spirit.spans, hour, words, reply.unobserved, opt.branchFirst) ?? '';
      return {
        key: spirit.token,
        kind: 'standard',
        label: spirit.name,
        subtitle,
        icon: urlArtHandle(spirit.icon),
        iconMissing: !spirit.icon,
        iconSize: 22,
        spans: visibleSpans,
        auditValue: `${spirit.name}|${subtitle}`,
        meta: { token: spirit.token },
      };
    });

    return {
      mode: 'lanes',
      hour,
      extent,
      playhead: hour,
      lanes,
      unobserved,
      iconsOnBars: opt.iconsOnBars,
      labelWidth: 210,
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
      : lane.spans?.find(s => s.firstHour <= hour && hour <= s.lastHour) ?? lane.spans?.at(-1);
    const artItem = reply?.artwork.find(a => a.token === lane.key);
    const items = span
      ? [
          { label: startUnseen(span.firstHour, opt.branchFirst, reply?.unobserved) ? 'Held before' : 'Since', value: formatDay(span.firstHour) },
          { label: 'Until', value: span.openAtPlayhead ? 'Active' : formatDay(span.lastHour) },
        ]
      : [{ label: 'Status', value: lane.subtitle || 'Not held', tone: 'muted' as const }];
    return {
      title: lane.label,
      subtitle: 'National spirit',
      hour,
      items,
      ...(artItem?.description ? { description: artItem.description } : {}),
    };
  },
});

export const spiritHistoryDeclaration: TimelineWidgetDeclaration<SpiritHistoryProjectionOptions> = {
  family: 'timeline',
  kind: 'spirit-history',
  label: 'National spirits history',
  subject: 'national spirits',
  emptyCampaign: 'Load a campaign and choose a country to see its national spirits.',
  icon: 'history',
  optionFields: OPTIONS,
  countries: { mode: 'single', choose: 'Choose a country to see its national spirits.' },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => ({
    owner: `spirit-history:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
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
  projection: spiritHistoryProjection,
  projectionOptions: ctx => ({
    sort: (ctx.options.sort as SpiritSort) ?? 'gained',
    showEnded: ctx.options.ended !== 'hidden',
    iconsOnBars: ctx.options.iconsOnBars !== false,
    branchFirst: ctx.branch.firstHour,
    surface: '#16181d',
  }),
};
