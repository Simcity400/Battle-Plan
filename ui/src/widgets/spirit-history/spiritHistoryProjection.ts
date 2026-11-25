import { colour } from '../../charts/palette.ts';
import { holdingLabel, holdingSince, holdingSpanFlags, holdingUntil, ideaHistoryRead, lookOf, unobservedThrough } from '../../dashboard/ideaHistoryReply.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { artworkArt } from '../../families/art/provider.ts';
import { inHourSpans, timelineModel } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineProjection } from '../../families/timeline/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { IdeaHistoryReply, IdeaHolding } from '../../generated/dashboard';

export interface SpiritHistoryOptions {
  readonly showEnded: boolean;
  readonly iconsOnBars: boolean;
  readonly firstHour: number;
  readonly surface: string;
}

const NATIONAL_SPIRIT = 'national_spirit';
const UNCLASSIFIED = 'Category not recorded';

interface SpiritRow {
  readonly token: string;
  readonly holdings: IdeaHolding[];
  readonly unclassified: string | null;
}

function spiritRows(reply: IdeaHistoryReply): SpiritRow[] {
  const spiritCategories = new Set(reply.categories.filter(category => category.kind === NATIONAL_SPIRIT).map(category => category.token));
  const rows = new Map<string, SpiritRow>();
  for (const holding of reply.holdings) {
    const info = lookOf(reply, holding).info;
    if (info.category !== null && !spiritCategories.has(info.category)) continue;
    const row = rows.get(holding.token);
    if (row) row.holdings.push(holding);
    else rows.set(holding.token, { token: holding.token, holdings: [holding], unclassified: info.category === null ? info.unknown ?? 'The idea is in no category the content defines' : null });
  }
  return [...rows.values()];
}

function describe(holding: IdeaHolding): string {
  switch (holding.end.kind) {
    case 'held':
      return holding.start === 'heldBefore' ? `Held before ${formatDateOnly(holding.firstHour)}`
        : holding.start === 'firstObserved' ? `First observed ${formatDateOnly(holding.firstHour)}`
          : `Active since ${formatDateOnly(holding.firstHour)}`;
    case 'ended': return `Ended ${formatDateOnly(holding.lastHour + 1)}`;
    case 'endedBy': return `Ended by ${formatDateOnly(holding.end.hour)}`;
    case 'lastSeen': return `Last recorded ${formatDateOnly(holding.lastHour)}`;
  }
}

const ended = (holding: IdeaHolding) => holding.end.kind === 'ended' || holding.end.kind === 'endedBy';

export const spiritHistoryProjection: TimelineProjection<SpiritHistoryOptions> = registerFamilyProjection<TimelineProjection<SpiritHistoryOptions>>({
  family: 'timeline',
  kind: 'spirit-history',

  nextChange(readView) {
    return readView.panel<IdeaHistoryReply>('main')?.nextChange ?? Infinity;
  },

  project(readView, opt) {
    const hour = readView.hour;
    const reply = readView.panel<IdeaHistoryReply>('main');
    const base = { hour, firstHour: opt.firstHour, labelWidth: 220, iconsOnBars: opt.iconsOnBars };
    if (!reply) return timelineModel(base);
    const nextChangeAt = reply.nextChange ?? Infinity;
    const all = spiritRows(reply);
    const shown = opt.showEnded ? all : all.filter(row => !ended(row.holdings.at(-1)!));
    if (!shown.length) {
      const empty = all.length ? 'No national spirits held'
        : reply.unrecorded && !reply.holdings.length ? `National spirits not recorded: ${reply.unrecorded}` : 'No national spirits recorded yet';
      return timelineModel({ ...base, empty, ...(all.length || !reply.unrecorded || reply.holdings.length ? {} : { emptyKind: 'unavailable' as const }), warnings: reply.warnings, nextChangeAt });
    }
    const fill = colour(opt.surface, 0);
    const lanes: TimelineLane[] = shown.map(row => {
      const latest = row.holdings.at(-1)!;
      const latestLook = lookOf(reply, latest);
      const name = holdingLabel(latestLook, latest);
      const icon = artworkArt(readView, latestLook.artwork);
      const spans = row.holdings.map(holding => {
        const look = lookOf(reply, holding);
        const spanIcon = artworkArt(readView, look.artwork);
        return {
          id: `${row.token}:${holding.firstHour}`,
          firstHour: holding.firstHour,
          color: fill,
          label: holdingLabel(look, holding),
          icon: spanIcon,
          ...holdingSpanFlags(holding, hour),
          meta: { holding },
        };
      });
      const subtitle = row.unclassified ? UNCLASSIFIED : describe(latest);
      return {
        key: row.token,
        kind: 'item' as const,
        label: name,
        subtitle,
        icon,
        spans,
        auditValue: `${name}|${subtitle}`,
      };
    });
    return timelineModel({ ...base, lanes, unobserved: unobservedThrough(reply, hour), warnings: reply.warnings, nextChangeAt });
  },

  tooltip(model, hit, readView) {
    const reply = readView.panel<IdeaHistoryReply>('main');
    const lane = laneByKey(model, hit.laneKey);
    if (!lane) return null;
    const span = spanAt(lane, hit);
    const at = hit.hour ?? model.hour;
    if (!span || !reply) {
      return { title: lane.label, subtitle: 'National spirit', hour: at, items: [{ label: 'Held', value: inHourSpans(model.unobserved, at) ? 'Not recorded' : 'Not held' }] };
    }
    const holding = (span.meta as { holding: IdeaHolding }).holding;
    const look = lookOf(reply, holding);
    return {
      title: span.label ?? lane.label,
      subtitle: look.info.category === null ? UNCLASSIFIED : 'National spirit',
      hour: at,
      items: [holdingSince(holding), holdingUntil(holding)],
      ...(look.artwork.description ? { description: look.artwork.description } : {}),
      ...(look.info.category === null ? { note: look.info.unknown ?? 'The idea is in no category the content defines' } : {}),
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
          read: ideaHistoryRead(ctx.branch.id, ctx.countryTag),
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
