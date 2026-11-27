import { liveKey, type LiveValues } from '../../charts/live.ts';
import { gameDate } from '../../dashboard/calendar.ts';
import { subjectHandle } from '../../dashboard/chosen.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { CardWidgetDeclaration } from '../../families/card/declaration.ts';
import { cardFlag, type CardItemRow, type CardModel, type CardProjection, type CardRow } from '../../families/card/model.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded, partialValue, type Value } from '../../families/value.ts';
import { formatDateOnly, formatShare } from '../../families/valueFormat.ts';
import type { LeaderHourReply, LeaderSelection, LeaderServiceStart, LeaderTrait } from '../../generated/dashboard';
import { experienceText, experienceWhole } from './experience.ts';

export const DOSSIER_VIEW = 'dossier';

export interface DossierOptions {
  readonly reference: string | null;
}

type Live = LiveValues<number | bigint | string> | null | undefined;

const COLUMNS = { art: { width: 30, height: 38 }, figure: 180, figureAlign: 'right' } as const;
const ROW_HEIGHT = 48;
const PARTIAL = 'Battle counts are incomplete: some battles were not recorded.';

export function validateLeaderHour(input: unknown): LeaderHourReply {
  const reply = input as LeaderHourReply;
  if (typeof reply !== 'object' || reply === null || !('reference' in reply) || !Array.isArray(reply.traitCatalogue) || !Array.isArray(reply.warnings)) {
    throw new Error('The leader reply is malformed.');
  }
  return reply;
}

export function leaderParams(ctx: FamilyPlanContext): { readonly reference: string } | { readonly selection: LeaderSelection } | null {
  const chosen = String(ctx.options.leader ?? '');
  if (chosen) {
    const reference = subjectHandle(ctx.track.source.metadata.campaignKey, chosen);
    return reference === null ? null : { reference };
  }
  const group = groupSelection(ctx.widget.countries);
  if (group) return { selection: { kind: 'group', group } };
  if (ctx.countries.all) return { selection: { kind: 'all' } };
  return ctx.countries.tags.length ? { selection: { kind: 'countries', countries: ctx.countries.tags.map(countryFromKey) } } : null;
}

function experience(live: Live, key: string): { readonly raw: string | null; readonly reason: string } {
  const value = live?.get(liveKey(key, 'experience'));
  return value === undefined
    ? { raw: null, reason: live?.missing(liveKey(key, 'experience')) || 'Experience was not recorded at this date' }
    : { raw: String(value), reason: '' };
}

function progress(raw: string | null, required: string | null, reason: string, needed: string): { readonly fraction: Value<number>; readonly figure: Value<string> } {
  if (raw === null) return { fraction: notRecorded(reason), figure: notRecorded(reason) };
  const figure = knownValue(experienceWhole(raw, required));
  if (required === null) return { fraction: notRecorded(needed), figure };
  const of = Number(required);
  return { fraction: knownValue(of > 0 ? Math.max(0, Math.min(1, Number(raw) / of)) : 1), figure };
}

const traitName = (trait: LeaderTrait | undefined) => trait?.name ?? trait?.token ?? 'Trait name not recorded';

function traitText(trait: LeaderTrait | undefined, date: readonly [number, number] | null | undefined): string {
  const year = date ? gameDate(date[0])?.year : undefined;
  return year === undefined ? `${traitName(trait)} · date not recorded` : `${traitName(trait)} · ${date?.[1] === 0 ? 'before ' : ''}${year}`;
}

function serviceStartItem(start: LeaderServiceStart | null): TooltipListItem | null {
  if (!start) return null;
  if (start.kind === 'since') return { label: 'Serving since', value: formatDateOnly(start.hour), divider: true };
  if (start.kind === 'before') return { label: 'Serving before', value: formatDateOnly(start.hour), divider: true };
  return { label: 'Service start', missingReason: start.reason, divider: true };
}

function header(readView: Parameters<CardProjection['project']>[0], reply: LeaderHourReply, reference: string, partial: boolean): CardItemRow {
  const hour = readView.hour;
  const role = reply.role!;
  const name = reply.name;
  const owner = role.owner ? cardFlag(readView, role.owner, hour) : null;
  const rank = role.rank ?? `Rank ${role.category}`;
  const skill = reply.skill === null ? 'skill not recorded' : `★ ${reply.skill}${role.maximumSkill === null ? '' : ` / ${role.maximumSkill}`}`;
  const decided = reply.won + reply.lost;
  const counts = `${reply.won} won · ${reply.lost} lost`;
  const record = partial ? counts : `${counts} · ${decided ? formatShare(reply.won / decided) : 'no results'}`;
  const names = role.attributeNames.map(item => item ?? 'Name not recorded');
  const ownerName = owner ? (owner.name.kind === 'known' ? owner.name.value : owner.name.kind === 'pending' ? 'Loading…' : 'Country name not recorded') : 'Owner not recorded';
  const countryItem: TooltipListItem = !owner
    ? { label: 'Country', missingReason: 'Owner not recorded' }
    : owner.name.kind === 'known'
      ? { label: 'Country', value: owner.name.value }
      : owner.name.kind === 'pending'
        ? { label: 'Country', pendingReason: owner.name.reason }
        : { label: 'Country', missingReason: 'reason' in owner.name ? owner.name.reason : 'Country name not recorded' };
  const serviceStart = serviceStartItem(reply.serviceStart);
  const items: TooltipListItem[] = [
    countryItem,
    { label: 'Rank', value: rank },
    reply.skill === null ? { label: 'Skill', missingReason: 'Skill was not recorded at this date' } : { label: 'Skill', value: skill.slice(2) },
    ...names.map((label, i) => ({ label, value: String(reply.components[i]) })),
    { label: 'Battles fought', value: String(reply.fought), divider: true },
    { label: 'Won', value: String(reply.won) },
    { label: 'Lost', value: String(reply.lost) },
    { label: 'Unresolved', value: String(Math.max(0, reply.fought - decided)) },
    ...(partial ? [{ label: PARTIAL, tone: 'muted' as const }] : []),
    ...(serviceStart ? [serviceStart] : []),
  ];
  return {
    kind: 'item',
    key: 'leader',
    subject: reference,
    art: { kind: 'art', art: readView.art({ kind: 'portrait', leader: reference, hour }), ...(name ? { name } : {}) },
    title: name ? knownValue(name) : notRecorded('The name was not recorded'),
    subtitle: `${rank} · ${skill}`,
    ...(owner ? { subtitleFlag: { ...owner.media, name: ownerName } } : {}),
    figure: { value: partial ? partialValue(record, PARTIAL) : knownValue(record), figure: 'Battles' },
    tooltip: { title: name ?? 'Name not recorded', subtitle: ownerName, items },
  };
}

function recordRow(reply: LeaderHourReply, partial: boolean): CardRow {
  const decided = reply.won + reply.lost;
  if (!decided && partial) return { kind: 'list', key: 'record', label: 'Record', items: [], missing: PARTIAL, none: '' };
  if (!decided) return { kind: 'list', key: 'record', label: 'Record', items: [], none: reply.fought ? `${reply.fought} fought, none with a result yet` : 'No battles yet' };
  return {
    kind: 'meter',
    key: 'record',
    label: 'Record',
    ...(partial ? { figure: { value: partialValue(`${reply.won} won · ${reply.lost} lost`, PARTIAL), figure: 'Recorded results' } } : {}),
    bar: partial ? { kind: 'meter', fraction: notRecorded(PARTIAL) } : {
      kind: 'share',
      segments: [
        { share: reply.won / decided, tone: 'positive', label: `${reply.won} won`, shortLabel: String(reply.won) },
        { share: reply.lost / decided, tone: 'negative', label: `${reply.lost} lost`, shortLabel: String(reply.lost), labelAlign: 'right' },
      ],
    },
  };
}

function xpRow(reply: LeaderHourReply, reference: string, live: Live): CardRow {
  const role = reply.role!;
  const top = reply.skill !== null && role.maximumSkill !== null && reply.skill >= role.maximumSkill;
  const xp = experience(live, reference);
  const next = reply.progress?.nextRequiredRaw ?? null;
  const shown = progress(xp.raw, top ? null : next, xp.reason, 'The experience needed for the next skill level is not recorded');
  return {
    kind: 'meter',
    key: 'experience',
    label: 'XP',
    bar: { kind: 'meter', fraction: top && xp.raw !== null ? knownValue(1) : shown.fraction },
    figure: { value: shown.figure, figure: 'Experience' },
    tooltip: {
      title: 'Commander experience',
      items: [
        xp.raw === null ? { label: 'Experience', missingReason: xp.reason } : { label: 'Experience', value: experienceText(xp.raw, null) },
        top ? { label: 'Next skill level', value: 'At the highest skill' }
          : next === null ? { label: 'Next skill level', missingReason: 'The experience it needs is not recorded' }
            : { label: 'Next skill level', value: experienceText(next, null) },
      ],
    },
  };
}

function traitsRow(reply: LeaderHourReply): CardRow {
  if (reply.traits === null) return { kind: 'list', key: 'traits', label: 'Traits', items: [], missing: 'Traits were not recorded at this date', none: '' };
  const held = reply.traits.map(at => reply.traitCatalogue[at]);
  return {
    kind: 'list',
    key: 'traits',
    label: 'Traits',
    items: held.map((trait, i) => traitText(trait, reply.traitDates[i])),
    none: 'No traits',
    tooltip: {
      title: 'Traits',
      items: held.map((trait, i) => {
        const date = reply.traitDates[i];
        return date
          ? { label: traitName(trait), value: `${date[1] === 0 ? 'Held before' : 'Gained'} ${formatDateOnly(date[0])}` }
          : { label: traitName(trait), missingReason: 'Date not recorded' };
      }),
    },
  };
}

function trainingRows(reply: LeaderHourReply, live: Live): CardRow[] {
  const training = reply.progress?.training;
  if (training === null || training === undefined) {
    return [{ kind: 'list', key: 'training', label: 'Training', items: [], missing: 'Trait training was not recorded at this date', none: '' }];
  }
  if (!training.length) return [{ kind: 'list', key: 'training', label: 'Training', items: [], none: 'No traits in training' }];
  return training.map((trait, i) => {
    const xp = experience(live, trait.key);
    const shown = progress(xp.raw, trait.requiredRaw, xp.reason, 'The experience this trait needs is not recorded');
    const name = trait.name ?? trait.token ?? 'Trait name not recorded';
    return {
      kind: 'meter',
      key: `training:${trait.key}`,
      ...(i === 0 ? { label: 'Training' } : {}),
      name,
      bar: { kind: 'meter', fraction: shown.fraction },
      figure: { value: shown.figure, figure: 'Experience' },
      tooltip: {
        title: name,
        items: [
          xp.raw === null ? { label: 'Training experience', missingReason: xp.reason } : { label: 'Training experience', value: experienceText(xp.raw, null) },
          trait.requiredRaw === null ? { label: 'Needed', missingReason: 'The experience this trait needs is not recorded' } : { label: 'Needed', value: experienceText(trait.requiredRaw, null) },
        ],
      },
    };
  });
}

export const dossierProjection: CardProjection<DossierOptions> = registerFamilyProjection({
  family: 'card' as const,
  kind: 'general-career',
  project(readView, options): CardModel {
    const hour = readView.hour;
    const base = { hour, columns: COLUMNS, rowHeight: ROW_HEIGHT };
    const refused = readView.refused('main');
    if (refused) return { ...base, rows: [], empty: refused, emptyKind: 'unavailable' };
    const raw = readView.panel<unknown>('main');
    if (raw === undefined) return { ...base, rows: [] };
    const reply = validateLeaderHour(raw);
    const warnings = reply.warnings.length ? { warnings: reply.warnings } : {};
    const reference = reply.reference;
    if (reference === null) {
      return reply.missing
        ? { ...base, rows: [], empty: reply.missing, emptyKind: 'unavailable', ...warnings }
        : { ...base, rows: [], empty: options.reference === null ? 'No generals yet.' : 'This leader is not serving at this date.', emptyKind: 'empty', ...warnings };
    }
    if (reply.missing || !reply.role) {
      return { ...base, rows: [], empty: reply.missing ?? 'This leader was not recorded at this date.', emptyKind: 'unavailable', ...warnings };
    }
    const live = readView.live('main');
    const partial = reply.battlesPartialFrom !== null && reply.battlesPartialFrom <= hour;
    return {
      ...base,
      rows: [header(readView, reply, reference, partial), recordRow(reply, partial), xpRow(reply, reference, live), traitsRow(reply), ...trainingRows(reply, live)],
      ...(partial ? { notes: [PARTIAL] } : {}),
      ...warnings,
    };
  },
}, DOSSIER_VIEW) as CardProjection<DossierOptions>;

export const dossierDeclaration: CardWidgetDeclaration<DossierOptions> = {
  family: 'card',
  kind: 'general-career',
  label: 'General career',
  subject: 'general',
  emptyCampaign: 'Load a campaign to see a general’s career.',
  icon: 'history',
  countries: { mode: 'multi', allowSlots: true },
  keyByBranch: true,
  unavailable: ctx => (String(ctx.options.leader ?? '') && leaderParams(ctx) === null ? 'The saved general is from another campaign. Choose one in widget settings.' : null),
  sources: ctx => {
    const params = leaderParams(ctx);
    return {
      owner: `general-career-dossier:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${JSON.stringify(params)}`,
      entries: params
        ? [{
            name: 'main',
            source: ctx.track.source,
            branch: ctx.branch.id,
            offset: ctx.track.axisOffset,
            read: {
              shape: 'point' as const,
              channel: 'panel' as const,
              coverage: { mode: 'keyed' as const, keys: ['reference' in params ? params.reference : JSON.stringify(params.selection)] },
              product: 'unit-leader-hour' as const,
              followQuery: { panel: 'unit-leader-hour', branchId: ctx.branch.id, ...params },
              params: { branchId: ctx.branch.id, ...params },
              validate: validateLeaderHour,
            },
          }]
        : [],
    };
  },
  projection: dossierProjection,
  projectionOptions: ctx => {
    const params = leaderParams(ctx);
    return { reference: params && 'reference' in params ? params.reference : null };
  },
};
