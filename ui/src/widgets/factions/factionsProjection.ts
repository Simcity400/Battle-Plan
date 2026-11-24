import { colour } from '../../charts/palette.ts';
import { bridgeGaps } from '../../charts/timeline/runState.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { countryLook, inHourSpans, nextBoundaryAfter, timelineModel, spanStart } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineProjection, type TimelineSpan } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { CountryGroupReply, FactionGroup } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import {
  factionChangeHours,
  factionGaps,
  factionLanes,
  factionViews,
  laneCountries,
  UNNAMED,
  type FactionOrder,
  type FactionView,
  type MemberOrder,
} from './factionsData.ts';

export interface FactionsOptions {
  readonly order: FactionOrder;
  readonly members: MemberOrder;
  readonly showEnded: boolean;
  readonly firstHour: number;
  readonly surface: string;
  readonly selected: readonly string[] | null;
  readonly colours: Readonly<Record<string, string>>;
  readonly names: Readonly<Record<string, string>>;
}

export interface FactionLaneMeta {
  readonly factionId: string;
  readonly heading: boolean;
}

function nameOf(readView: BoundedReadView, opt: FactionsOptions, faction: FactionGroup, hour?: number) {
  const own = opt.names[faction.id];
  if (own) return { name: own, reason: null };
  const at = Math.min(hour ?? readView.hour, readView.hour);
  const who = readView.who('faction', faction.id, at);
  if (who.name.status === 'ready') return { name: who.name.value, reason: null };
  if (who.name.status === 'absent') return { name: null, reason: who.name.reason };
  const run = faction.names.find(item => item.firstHour <= at && at <= item.lastHour);
  return run ? { name: run.name, reason: run.reason } : { name: null, reason: null };
}

function viewsOf(readView: BoundedReadView, reply: CountryGroupReply, opt: FactionsOptions): FactionView[] {
  const ordinal = new Map(reply.factions.map((faction, index) => [faction.id, index]));
  return factionViews(
    reply,
    (faction, hour) => nameOf(readView, opt, faction, hour),
    id => opt.colours[id] ?? colour(opt.surface, ordinal.get(id) ?? 0),
  );
}

export const factionsProjection: TimelineProjection<FactionsOptions> = registerTimelineProjection<FactionsOptions>({
  kind: 'factions',

  nextChange(readView, opt) {
    const reply = readView.panel<CountryGroupReply>('main');
    return reply ? nextBoundaryAfter(factionChangeHours(reply, viewsOf(readView, reply, opt)), readView.hour) : Infinity;
  },

  project(readView, opt) {
    const hour = readView.hour;
    const reply = readView.panel<CountryGroupReply>('main');
    const base = { hour, firstHour: opt.firstHour, labelWidth: 240 };
    if (!reply) return timelineModel(base);
    const views = viewsOf(readView, reply, opt);
    const nextChangeAt = nextBoundaryAfter(factionChangeHours(reply, views), hour);
    if (!reply.factions.length) return timelineModel({ ...base, empty: 'No factions recorded', warnings: reply.warnings, nextChangeAt });
    const gaps = factionGaps(reply);
    const selected = opt.selected ? new Set(opt.selected) : null;
    const built = factionLanes(views, hour, {
      order: opt.order,
      members: opt.members,
      ended: opt.showEnded,
      countries: selected,
      countryIds: selected ? views[0]?.identities.selected([...selected]) ?? null : null,
    }, { country: tag => ({ name: countryLook(readView, tag, hour).name }), day: formatDateOnly }, gaps, opt.firstHour);
    if (!built.lanes.length) {
      return timelineModel({ ...base, empty: selected ? 'None of the chosen countries has been in a faction yet' : 'No factions yet', warnings: reply.warnings, nextChangeAt });
    }
    let headerKey = '';
    const lanes: TimelineLane[] = built.lanes.map(source => {
      const about = built.info.get(source.id)!;
      const heading = about.member === null;
      if (heading) headerKey = source.id;
      const unknown = heading ? gaps : about.faction.unknown;
      const spans: TimelineSpan[] = [];
      const runs = source.spans.filter(span => span.firstHour <= hour).sort((a, b) => a.firstHour - b.firstHour);
      for (const stretch of bridgeGaps(runs, unknown, (a, b) => a.glyph === b.glyph)) {
        const span = { ...stretch.runs[0]!, lastHour: stretch.lastHour };
        const open = span.lastHour >= hour;
        spans.push({
          id: `${source.id}@${span.id}`,
          firstHour: span.firstHour,
          lastHour: Math.min(span.lastHour, hour),
          color: about.faction.color,
          ...spanStart(span.firstHour, opt.firstHour, unknown),
          endUnseen: !open && inHourSpans(unknown, span.lastHour + 1),
          open,
        });
      }
      const name = source.identity?.label ?? UNNAMED;
      const [tag] = heading ? [] : laneCountries([source], built.info, hour);
      const look = tag ? countryLook(readView, tag, hour) : null;
      const meta: FactionLaneMeta = { factionId: about.faction.id, heading };
      return {
        key: source.id,
        kind: heading ? 'group' as const : 'item' as const,
        ...(heading ? { swatch: about.faction.color, ruleColor: about.faction.color } : { depth: 1, parentKey: headerKey }),
        ...(!heading ? (look ? { flag: look.flag, flagTag: look.tag } : { flagTag: '?' }) : {}),
        label: name,
        subtitle: source.label,
        spans,
        auditValue: `${name}|${source.label}`,
        meta,
      };
    });
    return timelineModel({ ...base, lanes, unobserved: gaps, warnings: reply.warnings, nextChangeAt });
  },

  tooltip(model, hit) {
    const lane = laneByKey(model, hit.laneKey);
    if (!lane) return null;
    const at = hit.hour ?? model.hour;
    if (lane.kind === 'group') {
      return { title: lane.label, subtitle: 'Faction', hour: at, items: [{ label: 'Members', value: lane.subtitle ?? '', ...(lane.swatch ? { swatch: lane.swatch } : {}) }] };
    }
    const span = spanAt(lane, hit);
    const faction = model.lanes.find(item => item.key === lane.parentKey);
    return {
      title: lane.label,
      subtitle: faction?.label ?? 'Faction member',
      hour: at,
      items: [
        { label: 'Membership', value: lane.subtitle ?? '' },
        ...(span ? [{ label: span.heldBefore ? 'Member before' : 'Joined', value: `${span.startBy ? 'By ' : ''}${formatDateOnly(span.firstHour)}` }] : []),
      ],
    };
  },
});

export const factionsDeclaration: TimelineWidgetDeclaration<FactionsOptions> = {
  family: 'timeline',
  kind: 'factions',
  label: 'Factions',
  subject: 'factions',
  emptyCampaign: 'Load a campaign to see its factions.',
  icon: 'shield',
  optionFields: OPTIONS,
  countries: { mode: 'multi', labels: true, allTagsForLabels: true },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => ({
    owner: `factions:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'branch-to-hour',
        coverage: { mode: 'population', form: 'all' },
        product: 'country-groups',
        followQuery: null,
        params: { branchId: ctx.branch.id },
        buildSpanParams: params => params,
      },
    }],
  }),
  projection: factionsProjection,
  projectionOptions: (ctx, extra) => ({
    order: (ctx.options.order as FactionOrder) ?? 'founded',
    members: (ctx.options.members as MemberOrder) ?? 'joined',
    showEnded: ctx.options.ended !== 'hidden',
    firstHour: ctx.branch.firstHour,
    surface: extra.surface,
    selected: ctx.countries.all ? null : ctx.countries.tags,
    colours: {},
    names: {},
  }),
};

export function factionsDeclarationWith(
  colours: Readonly<Record<string, string>>,
  names: Readonly<Record<string, string>>,
): TimelineWidgetDeclaration<FactionsOptions> {
  return {
    ...factionsDeclaration,
    projectionOptions: (ctx, extra) => ({ ...factionsDeclaration.projectionOptions(ctx, extra), colours, names }),
  };
}
