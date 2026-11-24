import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { countryRowPresentation } from '../../families/bars/model.ts';
import { nextChangeAfter } from '../../families/holders.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { DeclaredCountryFilter, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artState } from '../../families/table/identityCell.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableCellIconItem, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, partialValue, textValue, zeroValue, type Value } from '../../families/value.ts';
import type { LeaderKinds, LeaderSelection, SpriteFrame, UnitLeadersReply } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';
import { attributeNames, leaderRows, traitDateText, validateLeaders, withheldLeaderState, withheldLeaders, type LeaderRow } from './generalsData.ts';

export interface GeneralsOptions {
  readonly filter: DeclaredCountryFilter;
  readonly kind: LeaderKinds;
}

interface Media { readonly icon: string | undefined; readonly pending: boolean }

interface GeneralRow {
  readonly leader: LeaderRow;
  readonly name: Value<string>;
  readonly portrait: Media;
  readonly flag: Media;
  readonly country: Value<string>;
  readonly skills: readonly Media[];
  readonly traits: readonly TableCellIconItem[];
}

const NONE: Media = { icon: undefined, pending: false };
const PARTIAL = 'Battle counts are incomplete: some battles were not recorded.';
const KIND_NAVY = 2;
const SKILLS = 5;

function media(readView: BoundedReadView, handle: Parameters<BoundedReadView['art']>[0] | null): Media {
  if (!handle) return NONE;
  const art = artState(readView, handle);
  return { icon: art.icon, pending: art.iconPending === true };
}

const spriteHandle = (frame: SpriteFrame | null | undefined) => frame ? { kind: 'sprite' as const, ...frame } : null;

function passingAt(filter: DeclaredCountryFilter, hour: number): ReadonlySet<string> | null {
  if (!filter.passing) return null;
  const set = new Set<string>();
  for (const [tag, ranges] of Object.entries(filter.passing)) if (ranges.some(([first, last]) => first <= hour && hour <= last)) set.add(tag);
  return set;
}

export function leaderSelection(filter: DeclaredCountryFilter): LeaderSelection {
  return filter.passing === null ? { kind: 'all' } : { kind: 'countries', countries: Object.keys(filter.passing).sort().map(countryFromKey) };
}

const identityKey = (row: LeaderRow) => (row.role.owner === null ? null : `${row.role.owner}:${row.key}`);

function datedName(readView: BoundedReadView, row: LeaderRow, hour: number): Value<string> {
  const key = identityKey(row);
  const recorded = textValue(row.row.name, 'Character name unavailable');
  if (key === null) return recorded;
  const who = readView.who('leader', key, hour);
  if (who.name.status === 'ready' && who.name.value.trim()) return knownValue(who.name.value);
  if (who.name.status === 'pending' || row.row.name !== null) return recorded;
  return notRecorded(who.name.status === 'ready' ? 'Character name unavailable' : who.name.reason);
}

const count = (value: number, partial: boolean): Value<number> =>
  partial ? partialValue(value, PARTIAL) : value === 0 ? zeroValue(0) : knownValue(value);

function shownRows(reply: UnitLeadersReply, hour: number, options: GeneralsOptions): LeaderRow[] {
  return leaderRows(reply, hour, passingAt(options.filter, hour), options.kind);
}

function columnNames(reply: UnitLeadersReply, options: GeneralsOptions): readonly string[] {
  const roles = reply.leaders.filter(row => options.kind === 'all' || (row.role.kind === KIND_NAVY) === (options.kind === 'navy'));
  return [0, 1, 2, 3].map(i => [...new Set(roles.map(row => attributeNames(row.role)[i]!))].join(' / '));
}

const [PORTRAIT, COUNTRY, COMMANDER, SKILL, ATTACK, DEFENSE, THIRD, FOURTH, TRAITS, WON, LOST, WIN_RATE, FOUGHT, UNRESOLVED] = COLUMNS;

function skillColumn(defaults: typeof SKILL, at: number, label: string | undefined, priority: number): DeclarativeColumnSpec<GeneralRow, GeneralsOptions> {
  return {
    ...defaults!, ...(label ? { label } : {}), priority, numeric: true, cellKind: 'skill', format: 'integer', barEligible: false,
    extra: row => ({ ...(row.skills[at]?.icon ? { icon: row.skills[at]!.icon } : {}), ...(row.skills[at]?.pending ? { iconPending: true } : {}) }),
    value: row => {
      const value = at === 0 ? row.leader.row.skill : row.leader.row.components[at - 1] ?? null;
      return value === null ? notRecorded('Skill not recorded') : value === 0 ? zeroValue(0) : knownValue(value);
    },
  };
}

function generalsColumns(reply: UnitLeadersReply, options: GeneralsOptions): readonly DeclarativeColumnSpec<GeneralRow, GeneralsOptions>[] {
  const names = columnNames(reply, options);
  return [
    { ...PORTRAIT!, priority: 4, sortable: false, cellKind: 'kind', iconSlot: true, icon: row => row.portrait.icon, iconPending: row => row.portrait.pending,
      extra: row => ({ artShape: 'portrait', artName: row.leader.name }),
      value: row => row.name },
    { ...COUNTRY!, priority: 1, iconSlot: true, icon: row => row.flag.icon, iconPending: row => row.flag.pending, country: row => row.leader.role.owner, value: row => row.country },
    { ...COMMANDER!, priority: 0, text: true, hideable: false, cellKind: 'item', widest: ['Erich von Manstein'],
      extra: row => (row.leader.role.rank ? { subtext: row.leader.role.rank } : {}),
      value: row => row.name },
    skillColumn(SKILL, 0, undefined, 1),
    skillColumn(ATTACK, 1, names[0], 2),
    skillColumn(DEFENSE, 2, names[1], 2),
    skillColumn(THIRD, 3, names[2], 3),
    skillColumn(FOURTH, 4, names[3], 3),
    { ...TRAITS!, priority: 5, sortable: false, cellKind: 'icons', minWidth: 136,
      extra: row => ({ iconStripKind: 'traits', icons: row.traits }),
      value: row => row.leader.row.traits === null ? notRecorded('Traits not recorded') : row.traits.length ? knownValue(row.traits.length) : zeroValue(0) },
    { ...WON!, priority: 1, numeric: true, format: 'integer', barEligible: true, widest: ['9,999'],
      value: row => count(row.leader.won ?? 0, row.leader.foughtPartial) },
    { ...LOST!, priority: 2, numeric: true, format: 'integer', barEligible: true, widest: ['9,999'],
      value: row => count(row.leader.lost ?? 0, row.leader.foughtPartial) },
    { ...WIN_RATE!, priority: 1, numeric: true, cellKind: 'winRate', barEligible: false,
      extra: row => ({ secondaryValue: count(row.leader.lost ?? 0, false) }),
      sortValue: row => row.leader.winRate === null ? notApplicable('No battles with known results') : knownValue(row.leader.winRate),
      value: row => count(row.leader.won ?? 0, false) },
    { ...FOUGHT!, priority: 6, numeric: true, format: 'integer', barEligible: true, widest: ['9,999'],
      value: row => count(row.leader.fought, row.leader.foughtPartial) },
    { ...UNRESOLVED!, priority: 6, numeric: true, format: 'integer', barEligible: true, widest: ['9,999'],
      value: row => count(row.leader.unresolved, row.leader.foughtPartial) },
  ];
}

function traitRows(row: LeaderRow): TooltipExtraRow[] {
  if (row.row.traits === null) return [{ label: 'Traits', value: notRecorded('Traits not recorded'), divider: true }];
  return row.traits.map((trait, at) => ({
    label: contentTitle(trait.name, trait.token, 'Trait name unavailable'),
    value: knownValue(traitDateText(row.row.traitDates[at] ?? null)),
    indent: true,
    ...(at === 0 ? { divider: true } : {}),
  }));
}

export const generalsProjection: TableProjection<GeneralsOptions> = registerFamilyProjection(
  declareTableProjection<UnitLeadersReply, GeneralRow, GeneralsOptions>({
    kind: 'generals',
    validate: validateLeaders,
    nextChange: (readView, options) => {
      let next = nextChangeAfter(options.filter.bounds, readView.hour);
      const raw = readView.panel('main');
      if (!raw) return next;
      for (const row of shownRows(validateLeaders(raw), readView.hour, options)) {
        const key = identityKey(row);
        if (key !== null) next = Math.min(next, readView.who('leader', key, readView.hour).nextChangeAt);
      }
      return next;
    },
    columns: generalsColumns,
    rowHeightMode: 'media',
    rows: (reply, _live, hour, options, readView) => {
      if (reply.missing !== null) return { rows: [], empty: 'Leaders weren’t recorded at this time.', emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      const unrecorded = options.filter.unrecorded.find(([first, last]) => first <= hour && hour <= last)?.[2] ?? null;
      if (options.filter.passing && unrecorded !== null) return { rows: [], empty: unrecorded, emptyKind: 'unavailable', warnings: reply.warnings };
      const skillArt = (frames: readonly (SpriteFrame | null)[]) => Array.from({ length: SKILLS }, (_, at) => media(readView, spriteHandle(frames[at])));
      const land = skillArt(reply.skillIcons.land);
      const navy = skillArt(reply.skillIcons.navy);
      const traitArt = reply.traits.map(trait => media(readView, trait.icon ? { kind: 'url', url: trait.icon } : null));
      const shown = shownRows(reply, hour, options);
      const rows = shown.map((leader): GeneralRow => {
        const owner = leader.role.owner;
        const presented = owner === null ? null : countryRowPresentation(readView, owner, undefined, undefined, hour);
        const portrait = media(readView, { kind: 'portrait', leader: leader.key, hour: leader.row.firstSeenHour ?? hour });
        return {
          leader,
          name: datedName(readView, leader, hour),
          portrait,
          flag: presented?.media.kind === 'flag' ? { icon: presented.media.iconUrl, pending: presented.media.art.status === 'pending' } : NONE,
          country: presented === null ? notRecorded('Owner not recorded') : presented.labelMissing ? notRecorded('Country name not recorded') : textValue(presented.label, 'Country name not recorded'),
          skills: leader.role.kind === KIND_NAVY ? navy : land,
          traits: (leader.row.traits ?? []).map((at, i) => ({
            name: contentTitle(leader.traits[i]?.name, leader.traits[i]?.token ?? String(at), 'Trait name unavailable'),
            ...(traitArt[at]?.icon ? { url: traitArt[at]!.icon } : {}),
            ...(traitArt[at]?.pending ? { pending: true } : {}),
          })),
        };
      });
      const withheld = withheldLeaders(reply, passingAt(options.filter, hour), options.kind);
      const state = withheld.length ? withheldLeaderState(withheld) : null;
      if (!rows.length) {
        return state
          ? { rows, empty: state.message, emptyDetail: state.detail, emptyKind: 'unavailable', warnings: reply.warnings }
          : { rows, empty: 'No leaders for these countries at this time.', emptyKind: 'empty', warnings: reply.warnings };
      }
      const count = new Set(withheld.map(row => row.reference)).size;
      return { rows, ...(state ? { notes: [`${count} ${count === 1 ? 'leader' : 'leaders'} of these countries can’t be shown; Details lists why.`] } : {}),
        warnings: state ? [...reply.warnings, state.detail] : reply.warnings };
    },
    rowKey: row => row.leader.key,
    rowName: row => row.name.kind === 'known' ? row.name.value : row.leader.name,
    tooltipExtra: row => traitRows(row.leader),
    tooltipNote: row => (row.leader.foughtPartial ? PARTIAL : undefined),
  }),
);

const optionsCache = new WeakMap<DeclaredCountryFilter, Map<LeaderKinds, GeneralsOptions>>();
const kindOf = (value: unknown): LeaderKinds => (value === 'land' || value === 'navy' ? value : 'all');

export const generalsDeclaration: TableWidgetDeclaration<GeneralsOptions> = {
  family: 'table',
  kind: 'generals',
  label: 'Generals leaderboard',
  subject: 'leaders',
  emptyCampaign: 'Load a campaign to see its leaders.',
  icon: 'star',
  countries: { mode: 'filter' },
  sources: ctx => {
    const params = { branchId: ctx.branch.id, selection: leaderSelection(ctx.countries.filter), leaders: kindOf(ctx.options.leaders) };
    return {
      owner: `generals:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      entries: [{
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'population', form: 'all' },
          product: 'unit-leaders',
          followQuery: { panel: 'unit-leaders', ...params },
          params,
          validate: validateLeaders,
        },
      }],
    };
  },
  projection: generalsProjection,
  columns: generalsColumns({ leaders: [], traits: [], withheld: [], missing: null, battlesPartialFrom: null, resultsMissing: '', skillIcons: { land: [], navy: [] }, warnings: [] }, { filter: { passing: null, unrecorded: [], bounds: [] }, kind: 'all' }),
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => {
    const kind = kindOf(ctx.options.leaders);
    let byKind = optionsCache.get(ctx.countries.filter);
    if (!byKind) optionsCache.set(ctx.countries.filter, byKind = new Map());
    let held = byKind.get(kind);
    if (!held) byKind.set(kind, held = { filter: ctx.countries.filter, kind });
    return held;
  },
  jumpToDate: true,
};
