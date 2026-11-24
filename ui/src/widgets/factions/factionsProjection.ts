import { unavailableArt } from '../../families/art/types.ts';
import { colour, readableColour } from '../../charts/palette.ts';
import type { HourRanges } from '../../families/holders.ts';
import { bridgeGaps } from '../../charts/timeline/runState.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { boundedRuns, clipSpansToHour, countryLook, inHourSpans, nextBoundaryAfter, spanStart, timelineModel, wholeBranchProof } from '../../families/timeline/helpers.ts';
import { laneByKey, spanAt, type TimelineLane, type TimelineProjection, type TimelineSpan } from '../../families/timeline/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { groupCountsRead, rankedCounts } from '../../dashboard/groupCounts.ts';
import type { CountryGroupReply, FactionGroup, GroupCountsReply } from '../../generated/dashboard';
import {
  factionChangeHours,
  factionGaps,
  factionLanes,
  factionViews,
  leaderAt,
  membersAt,
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
  readonly styles: Readonly<Record<string, { readonly colour?: string; readonly name?: string }>>;
  readonly shared: Readonly<Record<string, { readonly colour: string }>>;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
}

export interface FactionLaneMeta {
  readonly factionId: string;
  readonly heading: boolean;
}

function nameOf(readView: BoundedReadView, opt: FactionsOptions, faction: FactionGroup, hour?: number) {
  const own = opt.styles[faction.id]?.name;
  if (own) return { name: own, reason: null };
  const at = Math.min(hour ?? readView.hour, readView.hour);
  const who = readView.who('faction', faction.id, at);
  if (who.name.status === 'ready') return { name: who.name.value, reason: null };
  if (who.name.status === 'absent') return { name: null, reason: who.name.reason };
  const run = faction.names.find(item => item.firstHour <= at && at <= item.lastHour);
  return run ? { name: run.name, reason: run.reason } : { name: null, reason: null };
}

function sharedKey(faction: FactionGroup): string | null {
  const founder = faction.leaders[0]?.tag ?? null;
  const key = faction.recordedName ? JSON.stringify(['name', faction.recordedName]) : founder ? JSON.stringify(['leader', founder]) : null;
  return key;
}

function factionColour(readView: BoundedReadView, opt: FactionsOptions, faction: FactionGroup | undefined, id: string, ordinal: number): string {
  const key = faction ? sharedKey(faction) : null;
  const own = opt.styles[id]?.colour ?? (key ? opt.shared[key]?.colour : undefined);
  if (own) return own;
  const who = readView.who('faction', id, readView.hour);
  return who.colour?.status === 'ready' ? readableColour(who.colour.value, opt.surface) : colour(opt.surface, ordinal);
}

function viewsOf(readView: BoundedReadView, reply: CountryGroupReply, opt: FactionsOptions): FactionView[] {
  const ordinal = new Map(reply.factions.map((faction, index) => [faction.id, index]));
  const byId = new Map(reply.factions.map(faction => [faction.id, faction]));
  return factionViews(
    reply,
    (faction, hour) => nameOf(readView, opt, faction, hour),
    id => factionColour(readView, opt, byId.get(id), id, ordinal.get(id) ?? 0),
  );
}

function selectedAt(opt: FactionsOptions, hour: number): Set<string> | null {
  if (!opt.selected) return null;
  const membership = opt.membership;
  return new Set(opt.selected.filter(tag => {
    const ranges = membership?.[tag];
    return !ranges || ranges.some(([first, last]) => first <= hour && hour <= last);
  }));
}

export const factionsProjection: TimelineProjection<FactionsOptions> = registerFamilyProjection<TimelineProjection<FactionsOptions>>({
  family: 'timeline',
  kind: 'factions',

  nextChange(readView, opt) {
    const reply = readView.panel<CountryGroupReply>('main');
    const counts = readView.panel<GroupCountsReply>('counts')?.nextChange ?? Infinity;
    return reply ? Math.min(counts, nextBoundaryAfter(factionChangeHours(reply, viewsOf(readView, reply, opt)), readView.hour)) : Infinity;
  },

  project(readView, opt) {
    const hour = readView.hour;
    const reply = readView.panel<CountryGroupReply>('main');
    const base = { hour, firstHour: opt.firstHour, labelWidth: 240 };
    if (!reply) return timelineModel(base);
    const views = viewsOf(readView, reply, opt);
    const counts = rankedCounts(readView.panel<GroupCountsReply>('counts'));
    const nextChangeAt = Math.min(counts?.nextChange ?? Infinity, nextBoundaryAfter(factionChangeHours(reply, views), hour));
    if (!reply.factions.length) return timelineModel({ ...base, empty: 'No factions recorded', warnings: reply.warnings, nextChangeAt });
    const gaps = factionGaps(reply);
    const selected = selectedAt(opt, hour);
    const built = factionLanes(views, hour, {
      order: opt.order,
      members: opt.members,
      ended: opt.showEnded,
      countries: selected,
      countryIds: selected ? views[0]?.identities.selected([...selected]) ?? null : null,
      counts,
    }, { country: tag => ({ name: countryLook(readView, tag, hour).name }), day: formatDateOnly }, gaps, opt.firstHour);
    if (!built.lanes.length) {
      return timelineModel({ ...base, empty: selected ? 'None of the chosen countries has been in a faction yet' : 'No factions yet', warnings: reply.warnings, nextChangeAt });
    }
    let headerKey = '';
    const lanes: TimelineLane[] = built.lanes.map(source => {
      const about = built.info.get(source.id)!;
      const heading = about.member === null;
      if (heading) headerKey = source.id;
      const unknown = clipSpansToHour(heading ? gaps : about.faction.unknown, hour);
      const spans: TimelineSpan[] = [];
      const runs = boundedRuns(source.spans, hour).sort((a, b) => a.firstHour - b.firstHour);
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
        ...(!heading ? (look ? { flag: look.flag, flagTag: look.tag } : { flag: unavailableArt('The country is not recorded at this hour'), flagTag: '?' }) : {}),
        label: name,
        subtitle: source.label,
        spans,
        auditValue: `${name}|${source.label}`,
        meta,
      };
    });
    return timelineModel({ ...base, lanes, unobserved: gaps, warnings: reply.warnings, nextChangeAt });
  },

  tooltip(model, hit, readView, opt) {
    const lane = laneByKey(model, hit.laneKey);
    if (!lane) return null;
    const at = hit.hour ?? model.hour;
    const meta = lane.meta as FactionLaneMeta | undefined;
    const reply = readView.panel<CountryGroupReply>('main');
    const view = meta && reply ? viewsOf(readView, reply, opt).find(item => item.id === meta.factionId) : undefined;
    if (lane.kind === 'group') {
      const count = view ? membersAt(view, at) : null;
      return {
        title: lane.label,
        subtitle: 'Faction',
        hour: at,
        items: [count === null
          ? { label: 'Members', missingReason: 'Members not recorded at this hour' }
          : { label: 'Members', value: String(count), ...(lane.swatch ? { swatch: lane.swatch } : {}) }],
      };
    }
    const span = spanAt(lane, hit);
    const faction = model.lanes.find(item => item.key === lane.parentKey);
    const leads = view && lane.flagTag ? leaderAt(view.leaders, at) === lane.flagTag : false;
    return {
      title: lane.label,
      subtitle: faction?.label ?? 'Faction member',
      hour: at,
      items: [
        ...(span ? [{ label: span.heldBefore ? 'Member before' : 'Joined', value: `${span.startBy ? 'By ' : ''}${formatDateOnly(span.firstHour)}` }] : [{ label: 'Membership', value: inHourSpans(model.unobserved, at) ? 'Not recorded' : 'Not a member' }]),
        ...(span && leads ? [{ label: 'Role', value: 'Leader' }] : []),
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
        provenInterval: wholeBranchProof,
      },
    }, {
      name: 'counts',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: groupCountsRead(ctx.branch.id, 'factions'),
    }],
  }),
  projection: factionsProjection,
  projectionOptions: (ctx, extra) => ({
    order: (ctx.options.order as FactionOrder) ?? 'founded',
    members: (ctx.options.members as MemberOrder) ?? 'joined',
    showEnded: ctx.options.ended !== 'hidden',
    firstHour: extra.extent[0],
    surface: extra.surface,
    selected: ctx.countries.all ? null : ctx.countries.tags,
    membership: ctx.countries.membership,
    styles: {},
    shared: {},
  }),
};

export function factionsDeclarationWithStyles(
  styles: FactionsOptions['styles'],
  shared: FactionsOptions['shared'],
): TimelineWidgetDeclaration<FactionsOptions> {
  return {
    ...factionsDeclaration,
    projectionOptions: (ctx, extra) => ({ ...factionsDeclaration.projectionOptions(ctx, extra), styles, shared }),
  };
}
