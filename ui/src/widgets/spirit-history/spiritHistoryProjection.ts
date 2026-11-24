import { colour } from '../../charts/palette.ts';
import { describeRun } from '../../charts/timeline/runState.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { inHourSpans, nextBoundaryAfter, sortedBoundaries, timelineModel, urlArtHandle, spanStart } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineProjection } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { IdeaHistoryReply } from '../../generated/dashboard';
import { endedBy, spirits, type SpiritSort } from '../national-spirits/spiritsData.ts';
import { OPTIONS } from './definition.ts';

export interface SpiritHistoryOptions {
  readonly sort: SpiritSort;
  readonly showEnded: boolean;
  readonly iconsOnBars: boolean;
  readonly firstHour: number;
  readonly surface: string;
}

function boundaries(reply: IdeaHistoryReply): number[] {
  const hours: number[] = [];
  for (const span of reply.spans) hours.push(span.firstHour, span.lastHour + 1);
  for (const gap of reply.unobserved) hours.push(gap.firstHour, gap.lastHour + 1);
  return sortedBoundaries(hours);
}

const WORDS = {
  since: (h: number) => `Active since ${formatDateOnly(h)}`,
  before: (h: number) => `Held before ${formatDateOnly(h)}`,
  ended: (h: number) => `Ended ${formatDateOnly(h)}`,
  endedBy: (h: number) => `Ended by ${formatDateOnly(h)}`,
  lastSeen: (h: number) => `Last recorded ${formatDateOnly(h)}`,
};

export const spiritHistoryProjection: TimelineProjection<SpiritHistoryOptions> = registerTimelineProjection<SpiritHistoryOptions>({
  kind: 'spirit-history',

  nextChange(readView) {
    const reply = readView.panel<IdeaHistoryReply>('main');
    return reply ? nextBoundaryAfter(boundaries(reply), readView.hour) : Infinity;
  },

  project(readView, opt) {
    const hour = readView.hour;
    const reply = readView.panel<IdeaHistoryReply>('main');
    const base = { hour, firstHour: opt.firstHour, labelWidth: 220, iconsOnBars: opt.iconsOnBars };
    if (!reply) return timelineModel(base);
    const nextChangeAt = nextBoundaryAfter(boundaries(reply), hour);
    const seen = spirits(reply).filter(spirit => {
      const first = spirit.spans[0]?.firstHour ?? Infinity;
      return first <= hour && (opt.showEnded || endedBy(spirit, hour, reply.unobserved) === null);
    });
    if (!seen.length) {
      return timelineModel({ ...base, empty: 'No national spirits recorded yet', warnings: reply.warnings, nextChangeAt });
    }
    const gained = (s: (typeof seen)[number]) => s.spans[0]?.firstHour ?? Infinity;
    const ended = (s: (typeof seen)[number]) => endedBy(s, hour, reply.unobserved) ?? Infinity;
    seen.sort(opt.sort === 'name'
      ? (a, b) => a.name.localeCompare(b.name)
      : opt.sort === 'ended'
        ? (a, b) => ended(a) - ended(b) || gained(a) - gained(b)
        : (a, b) => gained(a) - gained(b) || a.name.localeCompare(b.name));
    const fill = colour(opt.surface, 0);
    const lanes: TimelineLane[] = seen.map(spirit => {
      const icon = urlArtHandle(spirit.icon);
      const spans = spirit.spans.filter(span => span.firstHour <= hour).map(span => {
        const open = span.lastHour >= hour;
        return {
          id: `${spirit.token}:${span.firstHour}`,
          firstHour: span.firstHour,
          lastHour: Math.min(span.lastHour, hour),
          color: fill,
          label: spirit.name,
          icon,
          iconMissing: !icon,
          ...spanStart(span.firstHour, opt.firstHour, reply.unobserved),
          endUnseen: !open && inHourSpans(reply.unobserved, span.lastHour + 1),
          open,
        };
      });
      const subtitle = describeRun(spirit.spans, hour, WORDS, reply.unobserved, opt.firstHour) ?? '';
      return {
        key: spirit.token,
        kind: 'item' as const,
        label: spirit.name,
        subtitle,
        icon,
        iconMissing: !icon,
        spans,
        auditValue: `${spirit.name}|${subtitle}`,
      };
    });
    return timelineModel({ ...base, lanes, unobserved: reply.unobserved, warnings: reply.warnings, nextChangeAt });
  },

  tooltip(model, hit, readView) {
    const reply = readView.panel<IdeaHistoryReply>('main');
    const lane = laneByKey(model, hit.laneKey);
    if (!lane) return null;
    const span = spanAt(lane, hit) ?? lane.spans.at(-1);
    const art = reply?.artwork.find(item => item.token === lane.key);
    return {
      title: lane.label,
      subtitle: 'National spirit',
      hour: hit.hour ?? model.hour,
      items: span
        ? [
            { label: span.heldBefore ? 'Held before' : 'Since', value: `${span.startBy ? 'By ' : ''}${formatDateOnly(span.firstHour)}` },
            { label: 'Until', value: span.open ? 'Held now' : span.endUnseen ? `By ${formatDateOnly(span.lastHour + 1)}` : formatDateOnly(span.lastHour) },
          ]
        : [{ label: 'Status', value: lane.subtitle || 'Not held' }],
      ...(art?.description ? { description: art.description } : {}),
    };
  },
});

export const spiritHistoryDeclaration: TimelineWidgetDeclaration<SpiritHistoryOptions> = {
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
            buildSpanParams: params => params,
            validate: validateIdeaHistoryReply,
          },
        }]
      : [],
    allowEmptyFollow: true,
  }),
  projection: spiritHistoryProjection,
  projectionOptions: (ctx, extra) => ({
    sort: (ctx.options.sort as SpiritSort) ?? 'gained',
    showEnded: ctx.options.ended !== 'hidden',
    iconsOnBars: ctx.options.iconsOnBars !== false,
    firstHour: ctx.branch.firstHour,
    surface: extra.surface,
  }),
};
