import { colour } from '../../charts/palette.ts';
import { bridgeGaps, describeRun } from '../../charts/timeline/runState.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { urlArtHandle } from '../../families/art/provider.ts';
import { boundedRuns, clipSpansToHour, inHourSpans, nextBoundaryAfter, sortedBoundaries, spanStart, timelineModel, untilItem, wholeBranchProof } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineProjection } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { IdeaHistoryReply } from '../../generated/dashboard';
import { spirits, spiritsShown } from '../national-spirits/spiritsData.ts';
import { OPTIONS } from './definition.ts';

export interface SpiritHistoryOptions {
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
    const gaps = clipSpansToHour(reply.unobserved, hour);
    const seen = spiritsShown(spirits({ ...reply, spans: boundedRuns(reply.spans, hour) }), hour, opt.showEnded, gaps);
    if (!seen.length) {
      const classified = new Set(reply.ideas.filter(info => info.category).map(info => info.token));
      const unclassified = reply.spans.some(span => span.firstHour <= hour && !classified.has(span.token));
      return timelineModel(unclassified
        ? { ...base, empty: 'National spirits not recorded: some held ideas could not be classified', emptyKind: 'unavailable', warnings: reply.warnings, nextChangeAt }
        : { ...base, empty: 'No national spirits recorded yet', warnings: reply.warnings, nextChangeAt });
    }
    const fill = colour(opt.surface, 0);
    const lanes: TimelineLane[] = seen.map(spirit => {
      const icon = urlArtHandle(spirit.icon);
      const spans = bridgeGaps(spirit.spans, gaps).map(span => {
        const open = span.lastHour >= hour;
        return {
          id: `${spirit.token}:${span.firstHour}`,
          firstHour: span.firstHour,
          lastHour: Math.min(span.lastHour, hour),
          color: fill,
          label: spirit.name,
          icon,
          iconMissing: !icon,
          ...spanStart(span.firstHour, opt.firstHour, gaps),
          endUnseen: !open && inHourSpans(gaps, span.lastHour + 1),
          open,
        };
      });
      const subtitle = describeRun(spirit.spans, hour, WORDS, gaps, opt.firstHour) ?? '';
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
    return timelineModel({ ...base, lanes, unobserved: gaps, warnings: reply.warnings, nextChangeAt });
  },

  tooltip(model, hit, readView) {
    const reply = readView.panel<IdeaHistoryReply>('main');
    const lane = laneByKey(model, hit.laneKey);
    if (!lane) return null;
    const span = spanAt(lane, hit);
    const art = reply?.artwork.find(item => item.token === lane.key);
    return {
      title: lane.label,
      subtitle: 'National spirit',
      hour: hit.hour ?? model.hour,
      items: span
        ? [
            { label: span.heldBefore ? 'Held before' : 'Since', value: `${span.startBy ? 'By ' : ''}${formatDateOnly(span.firstHour)}` },
            untilItem(span, model.unobserved, hit.hour ?? model.hour, { held: 'Held now', until: 'Until' }),
          ]
        : [{ label: 'Held', value: inHourSpans(model.unobserved, hit.hour ?? model.hour) ? 'Not recorded' : 'Not held' }],
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
            provenInterval: wholeBranchProof,
            validate: validateIdeaHistoryReply,
          },
        }]
      : [],
  }),
  projection: spiritHistoryProjection,
  projectionOptions: (ctx, extra) => ({
    showEnded: ctx.options.ended !== 'hidden',
    iconsOnBars: ctx.options.iconsOnBars !== false,
    firstHour: extra.extent[0],
    surface: extra.surface,
  }),
};
