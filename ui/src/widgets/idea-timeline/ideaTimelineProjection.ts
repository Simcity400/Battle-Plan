import { colour } from '../../charts/palette.ts';
import { bridgeGaps } from '../../charts/timeline/runState.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { ideaNameMissing, validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { inHourSpans, nextBoundaryAfter, sortedBoundaries, timelineModel, urlArtHandle, spanStart } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineProjection } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { IdeaHistoryReply, IdeaSpan } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { chosenCategory, lanesFor, lawLevel } from './ideaTimelineData.ts';

export interface IdeaTimelineOptions {
  readonly category: string;
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

function bridgedSpans(spans: readonly IdeaSpan[], unobserved: IdeaHistoryReply['unobserved']): IdeaSpan[] {
  const byHolder = new Map<string, IdeaSpan[]>();
  for (const span of spans) {
    const key = JSON.stringify([span.token, span.slot, span.character?.id ?? null, span.character?.token ?? null, span.character?.name ?? null]);
    const list = byHolder.get(key);
    if (list) list.push(span); else byHolder.set(key, [span]);
  }
  const out: IdeaSpan[] = [];
  for (const list of byHolder.values()) {
    list.sort((a, b) => a.firstHour - b.firstHour);
    for (const stretch of bridgeGaps(list, unobserved)) out.push({ ...stretch.runs[0]!, lastHour: stretch.lastHour });
  }
  return out.sort((a, b) => a.firstHour - b.firstHour);
}

export const ideaTimelineProjection: TimelineProjection<IdeaTimelineOptions> = registerTimelineProjection<IdeaTimelineOptions>({
  kind: 'idea-timeline',

  nextChange(readView) {
    const reply = readView.panel<IdeaHistoryReply>('main');
    return reply ? nextBoundaryAfter(boundaries(reply), readView.hour) : Infinity;
  },

  project(readView, opt) {
    const hour = readView.hour;
    const reply = readView.panel<IdeaHistoryReply>('main');
    const base = { hour, firstHour: opt.firstHour, labelWidth: 200, iconsOnBars: opt.iconsOnBars };
    if (!reply) return timelineModel(base);
    const nextChangeAt = nextBoundaryAfter(boundaries(reply), hour);
    const category = chosenCategory(reply.categories, opt.category);
    if (!category) {
      return timelineModel({ ...base, empty: opt.category ? 'Category not in this recording' : 'No idea categories recorded', emptyKind: 'unavailable', warnings: reply.warnings, nextChangeAt });
    }
    const seen = { ...reply, spans: bridgedSpans(reply.spans.filter(span => span.firstHour <= hour), reply.unobserved) };
    const slotOrdinal = new Map(category.slots.map((slot, index) => [slot.token, index]));
    const gapNow = inHourSpans(reply.unobserved, hour);
    const lanes: TimelineLane[] = lanesFor(seen, category).map(group => {
      const fill = colour(opt.surface, slotOrdinal.get(group.slot) ?? 0);
      const held = bridgeGaps(group.spans, reply.unobserved, (a, b) => a.token === b.token && a.label === b.label);
      const spans = held.map(stretch => {
        const span = { ...stretch.runs[0]!, lastHour: stretch.lastHour };
        const open = span.lastHour >= hour;
        return {
          id: span.id,
          firstHour: span.firstHour,
          lastHour: Math.min(span.lastHour, hour),
          color: fill,
          label: span.label,
          icon: urlArtHandle(span.icon),
          iconMissing: !span.icon,
          ...spanStart(span.firstHour, opt.firstHour, reply.unobserved),
          endUnseen: !open && inHourSpans(reply.unobserved, span.lastHour + 1),
          open,
          meta: { token: span.token },
        };
      });
      const current = spans.find(span => span.open);
      const subtitle = current?.label ?? (gapNow ? 'Not recorded' : 'Empty');
      return {
        key: group.id,
        kind: 'item' as const,
        label: group.label,
        subtitle,
        ...(current ? { icon: current.icon, iconMissing: current.iconMissing } : {}),
        spans,
        auditValue: `${group.label}|${subtitle}`,
      };
    });
    return timelineModel({ ...base, lanes, unobserved: reply.unobserved, warnings: reply.warnings, nextChangeAt });
  },

  tooltip(model, hit, readView, opt) {
    const reply = readView.panel<IdeaHistoryReply>('main');
    const lane = laneByKey(model, hit.laneKey);
    if (!lane) return null;
    const span = spanAt(lane, hit);
    if (!span) {
      const at = hit.hour ?? model.hour;
      return { title: lane.label, hour: at, items: [{ label: 'Held', value: inHourSpans(reply?.unobserved, at) ? 'Not recorded' : 'Empty' }] };
    }
    const token = (span.meta as { token: string }).token;
    const info = reply?.ideas.find(item => item.token === token);
    const art = reply?.artwork.find(item => item.token === token);
    const category = reply ? chosenCategory(reply.categories, opt.category) : null;
    const level = category ? lawLevel(category, info) : null;
    const why = ideaNameMissing(art, info);
    return {
      title: span.label ?? lane.label,
      subtitle: level ? `${lane.label} · Level ${level}` : lane.label,
      hour: hit.hour ?? model.hour,
      items: [
        { label: span.heldBefore ? 'Held before' : 'Since', value: `${span.startBy ? 'By ' : ''}${formatDateOnly(span.firstHour)}` },
        { label: 'Until', value: span.open ? 'Held now' : span.endUnseen ? `By ${formatDateOnly(span.lastHour + 1)}` : formatDateOnly(span.lastHour) },
      ],
      ...(art?.description ? { description: art.description } : {}),
      ...(why ? { note: why } : {}),
    };
  },
});

export const ideaTimelineDeclaration: TimelineWidgetDeclaration<IdeaTimelineOptions> = {
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
            buildSpanParams: params => params,
            validate: validateIdeaHistoryReply,
          },
        }]
      : [],
  }),
  projection: ideaTimelineProjection,
  projectionOptions: (ctx, extra) => ({
    category: String(ctx.options.category ?? ''),
    iconsOnBars: ctx.options.iconsOnBars !== false,
    firstHour: ctx.branch.firstHour,
    surface: extra.surface,
  }),
};
