import { colour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { countryLook, nextBoundaryAfter, timelineModel, clipSpansToHour, wholeBranchProof } from '../../families/timeline/helpers.ts';
import { laneByKey, type TimelineProjection } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { DiplomacyPactsReply } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { aboutElsewhere, agreementKinds, agreementLanes, agreementsOf, changeHours, validatePacts, type AgreementLaneMeta } from './pactsData.ts';

export interface DiplomaticPactsOptions {
  readonly countryTag: string | null;
  readonly kinds: readonly string[] | null;
  readonly showEnded: boolean;
  readonly firstHour: number;
  readonly surface: string;
}

export const diplomaticPactsProjection: TimelineProjection<DiplomaticPactsOptions> = registerTimelineProjection<DiplomaticPactsOptions>({
  kind: 'diplomatic-pacts',

  nextChange(readView) {
    const reply = readView.panel<DiplomacyPactsReply>('main');
    return reply ? nextBoundaryAfter(changeHours(reply), readView.hour) : Infinity;
  },

  project(readView, opt) {
    const hour = readView.hour;
    const reply = readView.panel<DiplomacyPactsReply>('main');
    const base = { hour, firstHour: opt.firstHour, labelWidth: 220 };
    if (!reply || !opt.countryTag) return timelineModel(base);
    const nextChangeAt = nextBoundaryAfter(changeHours(reply), hour);
    const warnings = reply.warnings.filter(warning => !aboutElsewhere(warning));
    const own = countryLook(readView, opt.countryTag, hour);
    const kindOrder = new Map(agreementKinds(reply.kinds).map((item, index) => [item.kind, index]));
    const kindNames = new Map(agreementKinds(reply.kinds).map(item => [item.kind, item.name]));
    const agreements = agreementsOf(reply.pacts, opt.countryTag, hour, opt.kinds ? new Set(opt.kinds) : null);
    const lanes = agreementLanes(agreements, opt.showEnded, hour, {
      name: own.name,
      country: (tag, at) => countryLook(readView, tag, at),
      kindName: kind => kindNames.get(kind) ?? 'Relation name unavailable',
      colour: kind => colour(opt.surface, kindOrder.get(kind) ?? kindOrder.size),
    });
    if (!lanes.length) {
      return timelineModel({ ...base, empty: agreements.length ? 'No agreements in force' : 'No agreements recorded yet', warnings, nextChangeAt });
    }
    return timelineModel({ ...base, lanes, unobserved: clipSpansToHour(reply.unobserved, hour), warnings, nextChangeAt });
  },

  tooltip(model, hit) {
    const lane = laneByKey(model, hit.laneKey);
    const meta = lane?.meta as AgreementLaneMeta | undefined;
    if (!lane || !meta) return lane ? { title: lane.label, ...(lane.subtitle ? { subtitle: lane.subtitle } : {}) } : null;
    const { agreement } = meta;
    const state = agreement.state;
    return {
      title: meta.title,
      hour: hit.hour ?? model.hour,
      items: [
        { label: meta.mutual ? 'With' : agreement.outgoing ? 'To' : 'From', value: lane.label },
        { label: 'Since', value: `${agreement.sinceBound ? 'By ' : ''}${formatDateOnly(agreement.since)}` },
        ...(agreement.expires != null ? [{ label: 'Expires', value: formatDateOnly(agreement.expires) }] : []),
        state.state === 'present'
          ? { label: 'Status', value: 'In force' }
          : state.state === 'gone'
            ? { label: 'Ended', value: `By ${formatDateOnly(state.by)}` }
            : { label: 'Status', missingReason: `Not recorded after ${formatDateOnly(state.since)}` },
      ],
    };
  },
});

export const diplomaticPactsDeclaration: TimelineWidgetDeclaration<DiplomaticPactsOptions> = {
  family: 'timeline',
  kind: 'diplomatic-pacts',
  label: 'Diplomatic agreements',
  subject: 'diplomacy',
  emptyCampaign: 'Load a campaign to see diplomacy.',
  icon: 'link',
  optionFields: OPTIONS,
  countries: { mode: 'single', choose: 'Choose a country to see its diplomatic agreements.' },
  jumpToDate: true,
  keyByBranch: true,
  sources: ctx => ({
    owner: `diplomatic-pacts:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
    entries: ctx.countryTag
      ? [{
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'keyed', keys: [ctx.countryTag] },
            product: 'diplomatic-pacts',
            followQuery: null,
            params: { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) },
            buildSpanParams: params => params,
            provenInterval: wholeBranchProof,
            validate: validatePacts,
          },
        }]
      : [],
  }),
  projection: diplomaticPactsProjection,
  projectionOptions: (ctx, extra) => {
    const chosen = contentSet(ctx.options.kinds).map(contentKey);
    return {
      countryTag: ctx.countryTag,
      kinds: chosen.length ? chosen : null,
      showEnded: ctx.options.ended !== 'hidden',
      firstHour: extra.extent[0],
      surface: extra.surface,
    };
  },
};
