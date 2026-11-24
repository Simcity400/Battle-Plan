import { formatDay } from '../../dashboard/calendar.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { TimelineWidgetDeclaration } from '../../families/timeline/declaration.ts';
import { clipSpansToHour, nextBoundaryAfter, paletteAt, resolveCountryChip } from '../../families/timeline/helpers.ts';
import type { TimelineLaneModel, TimelineModel, TimelineProjection } from '../../families/timeline/model.ts';
import { registerTimelineProjection } from '../../families/timeline/registry.ts';
import type { DiplomacyPactsReply } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { aboutElsewhere, agreementsOf, changeHours, validatePacts, type Agreement } from './pactsData.ts';

export interface DiplomaticPactsProjectionOptions {
  readonly countryTag: string | null;
  readonly kinds: readonly string[] | null;
  readonly showEnded: boolean;
  readonly branchFirst: number;
  readonly surface: string;
}

const KNOWN_SPECS: readonly {
  readonly kind: string;
  readonly title: string;
  readonly one: string;
  readonly out?: (name: string) => string;
  readonly in?: (name: string) => string;
  readonly incomingFirst?: boolean;
}[] = [
  { kind: 'non_aggression_pact', title: 'Non-aggression pacts', one: 'Non-aggression pact' },
  { kind: 'guarantee', title: 'Guarantees', one: 'Guarantee', out: name => `${name} guarantees`, in: () => 'Guaranteed by' },
  { kind: 'military_access', title: 'Military access', one: 'Military access', out: name => `${name} has access in`, in: name => `Has access in ${name}` },
  { kind: 'docking_rights', title: 'Docking rights', one: 'Docking rights', out: name => `${name} docks in`, in: name => `Docks in ${name}` },
  { kind: 'air_base_access', title: 'Air base access', one: 'Air base access', out: name => `${name} flies from`, in: name => `Flies from ${name}` },
  { kind: 'embargo', title: 'Trade embargoes', one: 'Trade embargo', out: name => `${name} embargoes`, in: () => 'Embargoed by', incomingFirst: true },
  { kind: 'naval_blockade', title: 'Naval blockades', one: 'Naval blockade', out: name => `${name} blockades`, in: () => 'Blockaded by', incomingFirst: true },
];

export const diplomaticPactsProjection: TimelineProjection<DiplomaticPactsProjectionOptions> = registerTimelineProjection({
  kind: 'diplomatic-pacts',

  nextChange(readView) {
    const reply = readView.panel<DiplomacyPactsReply>('main');
    if (!reply) return Infinity;
    return nextBoundaryAfter(changeHours(reply), readView.hour);
  },

  project(readView, opt): TimelineModel {
    const hour = readView.hour;
    const reply = readView.panel<DiplomacyPactsReply>('main');
    const extent: [number, number] = [opt.branchFirst, Math.max(opt.branchFirst + 1, hour)];
    if (!reply || !opt.countryTag) {
      return { mode: 'lanes', hour, extent, playhead: hour, lanes: [], empty: 'Loading diplomacy\u2026' };
    }

    const kindSet = opt.kinds && opt.kinds.length > 0 ? new Set(opt.kinds) : null;
    const agreements = agreementsOf(reply.pacts, opt.countryTag, hour, kindSet);
    const present = agreements.filter(item => opt.showEnded || item.state.state === 'present');
    const ownChip = resolveCountryChip(readView, opt.countryTag, hour);
    const warnings = reply.warnings.filter(w => !aboutElsewhere(w));

    if (!present.length) {
      return {
        mode: 'lanes',
        hour,
        extent,
        playhead: hour,
        lanes: [],
        empty: `${ownChip.name} has no ${agreements.length ? 'agreements in force' : 'recorded agreements'} at this time.`,
        warnings,
      };
    }

    const kindNames = new Map(reply.kinds.map(k => [k.kind, k.name]));
    const knownRank = new Map(KNOWN_SPECS.map((s, i) => [s.kind, i]));
    const kinds = [...new Set(present.map(item => item.kind))].sort(
      (a, b) => (knownRank.get(a) ?? KNOWN_SPECS.length) - (knownRank.get(b) ?? KNOWN_SPECS.length)
        || (kindNames.get(a) ?? a).localeCompare(kindNames.get(b) ?? b),
    );

    const lanes: TimelineLaneModel[] = [];
    kinds.forEach((kind, kindIdx) => {
      const spec = KNOWN_SPECS.find(s => s.kind === kind);
      const title = spec?.title ?? kindNames.get(kind) ?? 'Relation name unavailable';
      const items = present.filter(item => item.kind === kind);
      const activeCount = items.filter(item => item.state.state === 'present').length;
      const color = paletteAt(opt.surface, kindIdx);

      lanes.push({
        key: `kind:${kind}`,
        kind: 'section-header',
        label: `${title} \u00b7 ${activeCount} in force`,
        height: 26,
      });

      const sides: { readonly label: string | null; readonly items: Agreement[] }[] = spec && !spec.out
        ? [{ label: null, items }]
        : [
            { label: spec?.out?.(ownChip.name) ?? `${ownChip.name} \u2192 partner`, items: items.filter(i => i.outgoing) },
            { label: spec?.in?.(ownChip.name) ?? `Partner \u2192 ${ownChip.name}`, items: items.filter(i => !i.outgoing) },
          ];
      if (spec?.incomingFirst) sides.reverse();

      for (const side of sides) {
        if (!side.items.length) continue;
        if (side.label) {
          lanes.push({
            key: `side:${kind}:${side.label}`,
            kind: 'section-header',
            depth: 1,
            label: side.label,
            height: 20,
          });
        }
        const ordered = [...side.items].sort((a, b) => Number(b.state.state === 'present') - Number(a.state.state === 'present') || a.since - b.since);
        for (const item of ordered) {
          const partnerChip = resolveCountryChip(readView, item.partner, Math.min(hour, item.lastSeen));
          const openAtPlayhead = item.state.state === 'present';
          const ended = !openAtPlayhead;
          const endHour = openAtPlayhead ? hour : Math.min(hour, item.lastSeen);
          lanes.push({
            key: `pact:${item.key}`,
            kind: 'standard',
            depth: 1,
            height: 26,
            label: partnerChip.name,
            flag: partnerChip.flag,
            flagTag: item.partner ?? undefined,
            dimmed: ended,
            spans: [{
              id: `bar:${item.key}`,
              firstHour: Math.max(opt.branchFirst, item.since),
              lastHour: Math.max(Math.max(opt.branchFirst, item.since), endHour),
              color,
              fadedStart: item.sinceBound,
              fadedEnd: item.state.state === 'unknown',
              ended,
              openAtPlayhead,
            }],
            auditValue: `${title}|${partnerChip.name}|${openAtPlayhead ? 'In force' : 'Ended'}`,
            meta: {
              title: spec?.one ?? title,
              mutual: Boolean(spec && !spec.out),
              outgoing: item.outgoing,
              partnerName: partnerChip.name,
              since: item.since,
              sinceBound: item.sinceBound,
              expires: item.expires,
              state: item.state,
            },
          });
        }
      }
    });

    return {
      mode: 'lanes',
      hour,
      extent,
      playhead: hour,
      lanes,
      unobserved: clipSpansToHour(reply.unobserved, hour),
      iconsOnBars: false,
      labelWidth: 210,
      warnings,
      nextChangeAt: nextBoundaryAfter(changeHours(reply), hour),
    };
  },

  tooltip(model, target, hour) {
    const lane = model.lanes?.find(l => l.key === (target.laneKey ?? target.key));
    if (!lane) return null;
    const meta = lane.meta as {
      title?: string;
      mutual?: boolean;
      outgoing?: boolean;
      partnerName?: string;
      since?: number;
      sinceBound?: boolean;
      expires?: number | null;
      state?: Agreement['state'];
    } | undefined;
    if (!meta) return null;
    const st = meta.state;
    return {
      title: meta.title ?? lane.label,
      hour,
      items: [
        { label: meta.mutual ? 'With' : meta.outgoing ? 'To' : 'From', value: meta.partnerName ?? lane.label },
        ...(meta.since !== undefined ? [{ label: 'Since', value: `${meta.sinceBound ? 'by ' : ''}${formatDay(meta.since)}` }] : []),
        ...(meta.expires != null ? [{ label: 'Expires', value: formatDay(meta.expires) }] : []),
        st?.state === 'present'
          ? { label: 'Status', value: 'In force' }
          : st?.state === 'gone'
            ? { label: 'Ended', value: `by ${formatDay(st.by)}` }
            : st
              ? { label: 'Last recorded', value: formatDay(st.since), tone: 'muted' as const }
              : { label: 'Status', value: 'Not recorded', tone: 'muted' as const },
      ],
    };
  },
});

export const diplomaticPactsDeclaration: TimelineWidgetDeclaration<DiplomaticPactsProjectionOptions> = {
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
            buildSpanParams: base => base,
            validate: validatePacts,
          },
        }]
      : [],
    allowEmptyFollow: true,
  }),
  projection: diplomaticPactsProjection,
  projectionOptions: ctx => {
    const chosen = contentSet(ctx.options.kinds).map(contentKey);
    return {
      countryTag: ctx.countryTag,
      kinds: chosen.length ? chosen : null,
      showEnded: ctx.options.ended !== 'hidden',
      branchFirst: ctx.branch.firstHour,
      surface: '#16181d',
    };
  },
};
