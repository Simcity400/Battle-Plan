import { colour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { holdingSince, holdingSpanFlags, holdingUntil, ideaHistoryProof, ideaNameMissing, lookOf, unobservedThrough, validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { inHourSpans, timelineModel, urlArtHandle } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineProjection } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { IdeaHistoryReply, IdeaHolding } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { chosenCategory, lanesFor, lawLevel } from './ideaTimelineData.ts';

export interface IdeaTimelineOptions {
  readonly category: string;
  readonly iconsOnBars: boolean;
  readonly firstHour: number;
  readonly surface: string;
}

export const ideaTimelineProjection: TimelineProjection<IdeaTimelineOptions> = registerTimelineProjection<IdeaTimelineOptions>({
  kind: 'idea-timeline',

  nextChange(readView) {
    return readView.panel<IdeaHistoryReply>('main')?.nextChange ?? Infinity;
  },

  project(readView, opt) {
    const hour = readView.hour;
    const reply = readView.panel<IdeaHistoryReply>('main');
    const base = { hour, firstHour: opt.firstHour, labelWidth: 200, iconsOnBars: opt.iconsOnBars };
    if (!reply) return timelineModel(base);
    const nextChangeAt = reply.nextChange ?? Infinity;
    const category = chosenCategory(reply.categories, opt.category);
    if (!category) {
      return timelineModel({ ...base, empty: opt.category ? 'Category not in this recording' : 'No idea categories recorded', emptyKind: 'unavailable', warnings: reply.warnings, nextChangeAt });
    }
    const slotOrdinal = new Map(category.slots.map((slot, index) => [slot.token, index]));
    const lanes: TimelineLane[] = lanesFor(reply, category).map(group => {
      const fill = colour(opt.surface, slotOrdinal.get(group.slot) ?? 0);
      const spans = group.spans.map(span => ({
        id: span.id,
        firstHour: span.holding.firstHour,
        color: fill,
        label: span.label,
        icon: urlArtHandle(span.icon),
        iconMissing: !span.icon,
        ...holdingSpanFlags(span.holding, hour),
        meta: { holding: span.holding },
      }));
      const current = spans.find(span => span.open);
      const subtitle = current?.label ?? (reply.unrecorded ? 'Not recorded' : 'Empty');
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
    return timelineModel({ ...base, lanes, unobserved: unobservedThrough(reply, hour), warnings: reply.warnings, nextChangeAt });
  },

  tooltip(model, hit, readView, opt) {
    const reply = readView.panel<IdeaHistoryReply>('main');
    const lane = laneByKey(model, hit.laneKey);
    if (!lane) return null;
    const span = spanAt(lane, hit);
    if (!span || !reply) {
      const at = hit.hour ?? model.hour;
      return { title: lane.label, hour: at, items: [{ label: 'Held', value: inHourSpans(model.unobserved, at) ? 'Not recorded' : 'Empty' }] };
    }
    const holding = (span.meta as { holding: IdeaHolding }).holding;
    const look = lookOf(reply, holding);
    const category = chosenCategory(reply.categories, opt.category);
    const level = category ? lawLevel(category, look.info) : null;
    const why = ideaNameMissing(look.artwork, look.info);
    return {
      title: span.label ?? lane.label,
      subtitle: level ? `${lane.label} · Level ${level}` : lane.label,
      hour: hit.hour ?? model.hour,
      items: [holdingSince(holding), holdingUntil(holding)],
      ...(look.artwork.description ? { description: look.artwork.description } : {}),
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
            buildSpanParams: (params, span) => ({ ...params, hour: span.lastHour }),
            provenInterval: ideaHistoryProof,
            validate: validateIdeaHistoryReply,
          },
        }]
      : [],
  }),
  projection: ideaTimelineProjection,
  projectionOptions: (ctx, extra) => ({
    category: String(ctx.options.category ?? ''),
    iconsOnBars: ctx.options.iconsOnBars !== false,
    firstHour: extra.extent[0],
    surface: extra.surface,
  }),
};
