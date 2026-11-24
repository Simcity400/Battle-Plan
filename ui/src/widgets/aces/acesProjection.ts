import { countryRowPresentation } from '../../families/bars/model.ts';
import { inHourRanges, nextChangeAfter } from '../../families/holders.ts';
import type { DeclaredCountryFilter, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notRecorded, valueFromNullable, type Value } from '../../families/value.ts';
import { formatDateHover, formatDateOnly } from '../../families/valueFormat.ts';
import type { AirAce, AirAcesReply } from '../../generated/dashboard';
import { validateAces } from './acesData.ts';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export interface AcesOptions { readonly filter: DeclaredCountryFilter }

interface Media { readonly icon: string | undefined; readonly pending: boolean }

interface AceRow {
  readonly ace: AirAce;
  readonly portrait: Media;
  readonly flag: Media;
  readonly country: Value<string>;
  readonly killerCountry: Value<string> | null;
}

const NO_COUNTRY: Value<string> = notRecorded('Country not recorded');

function portraitMedia(ace: AirAce, readView: BoundedReadView): Media {
  if (ace.portrait === null) return { icon: undefined, pending: false };
  const art = readView.art({ kind: 'url', url: ace.portrait });
  return art.status === 'ready' ? { icon: art.url, pending: false } : { icon: undefined, pending: art.status === 'pending' };
}

function countryOf(tag: string | null, readView: BoundedReadView): { readonly flag: Media; readonly country: Value<string> } {
  if (tag === null) return { flag: { icon: undefined, pending: false }, country: NO_COUNTRY };
  const shown = countryRowPresentation(readView, tag);
  return {
    flag: { icon: shown.media.kind === 'flag' ? shown.media.iconUrl : undefined, pending: shown.media.kind === 'flag' && shown.media.art.status === 'pending' },
    country: shown.labelMissing ? notRecorded('Country name not recorded') : knownValue(shown.label),
  };
}

export function aceSince(ace: AirAce): Value<string> {
  return ace.firstSeen === null ? notRecorded('First sighting not recorded') : knownValue(formatDateOnly(ace.firstSeen));
}

const wingValue = (ace: AirAce): Value<string> => ace.wing === null
  ? knownValue('No wing')
  : valueFromNullable(ace.wingName, { kind: 'notRecorded', reason: 'Wing name not recorded' });

function aircraftValue(ace: AirAce): Value<string> {
  if (ace.wing === null) return knownValue('None');
  if (ace.aircraft === null) return notRecorded('Aircraft not recorded');
  if (!ace.aircraft.length) return knownValue('None');
  return knownValue(ace.aircraft.map(plane => plane.name ?? 'Aircraft name unavailable').join(' · '));
}

const [PORTRAIT, COUNTRY, ACE, WING, SINCE] = COLUMNS;

const ACE_COLUMNS: readonly DeclarativeColumnSpec<AceRow, AcesOptions>[] = [
  { ...PORTRAIT!, priority: 4, sortable: false, iconSlot: true, icon: row => row.portrait.icon, iconPending: row => row.portrait.pending,
    value: row => row.ace.portrait === null ? notRecorded(row.ace.portraitMissing ?? 'Portrait not recorded') : knownValue(row.ace.name ?? 'Portrait') },
  { ...COUNTRY!, priority: 1, iconSlot: true, icon: row => row.flag.icon, iconPending: row => row.flag.pending, country: row => row.ace.tag, value: row => row.country },
  { ...ACE!, priority: 0, text: true, hideable: false, widest: ['Philipp Schultz'],
    value: row => valueFromNullable(row.ace.name, { kind: 'notRecorded', reason: 'Ace name not recorded' }) },
  { ...WING!, priority: 2, text: true, widest: ['Sturzkampfgeschwader 2'], value: row => wingValue(row.ace) },
  { ...SINCE!, priority: 3, numeric: true, widest: ['30 Sep 1941'], value: row => aceSince(row.ace),
    sortValue: row => valueFromNullable(row.ace.firstSeen, { kind: 'notRecorded', reason: 'First sighting not recorded' }) },
];

function aceTooltip(row: AceRow): TooltipExtraRow[] {
  const ace = row.ace;
  const rows: TooltipExtraRow[] = [
    ...(ace.callsign ? [{ label: 'Nickname', value: knownValue(`“${ace.callsign}”`) }] : []),
    { label: 'Aircraft', value: aircraftValue(ace) },
    { label: 'Became an ace', value: ace.firstSeen === null ? notRecorded('First sighting not recorded') : knownValue(`by ${formatDateHover(ace.firstSeen)}`), divider: true },
    { label: 'Status', value: knownValue(ace.alive ? 'Alive' : 'Killed') },
  ];
  const death = ace.death;
  if (death) {
    rows.push({ label: 'Killed', value: death.byHour === null || death.lastAlive === null
      ? notRecorded('Date of death not recorded')
      : knownValue(`after ${formatDateOnly(death.lastAlive)}, by ${formatDateOnly(death.byHour)}`) });
    if (death.killerAce) rows.push({ label: 'Killed by', value: valueFromNullable(death.killerName, { kind: 'notRecorded', reason: 'Killer ace name not recorded' }) });
    rows.push({ label: 'Killer country', value: row.killerCountry ?? notRecorded('Killer country not recorded') });
  }
  return rows;
}

export const acesProjection: TableProjection<AcesOptions> = registerTableProjection(
  declareTableProjection<AirAcesReply, AceRow, AcesOptions>({
    kind: 'aces',
    validate: validateAces,
    nextChange: (readView, options) => nextChangeAfter(options.filter.bounds, readView.hour),
    columns: ACE_COLUMNS,
    rows: (reply, _live, hour, options, readView) => {
      if (reply.missing !== null) return { rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      const passing = options.filter.passing;
      const unrecorded = options.filter.unrecorded.find(([first, last]) => first <= hour && hour <= last)?.[2] ?? null;
      if (passing && unrecorded !== null) return { rows: [], empty: unrecorded, emptyKind: 'unavailable', warnings: reply.warnings };
      const rows: AceRow[] = [];
      for (const ace of reply.aces) {
        if (passing && (ace.tag === null || !inHourRanges(passing[ace.tag], hour))) continue;
        const killerTag = ace.death?.killerTag ?? null;
        rows.push({ ace, portrait: portraitMedia(ace, readView), ...countryOf(ace.tag, readView), killerCountry: killerTag === null ? null : countryOf(killerTag, readView).country });
      }
      return { rows, ...(!rows.length ? { empty: 'No aces yet', emptyKind: 'empty' as const } : {}), warnings: reply.warnings };
    },
    rowKey: row => row.ace.id,
    rowName: row => row.ace.name ?? 'Unknown ace',
    tooltipExtra: row => aceTooltip(row),
  }),
);

const filterCache = new WeakMap<DeclaredCountryFilter, AcesOptions>();
export const acesDeclaration: TableWidgetDeclaration<AcesOptions> = {
  family: 'table',
  kind: 'aces',
  label: 'Aces',
  subject: 'aces',
  emptyCampaign: 'Load a campaign to see its aces.',
  icon: 'star',
  countries: { mode: 'filter' },
  sources: ctx => ({
    owner: `aces:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'all' },
        product: 'air-aces',
        followQuery: { panel: 'air-aces', branchId: ctx.branch.id },
        params: { branchId: ctx.branch.id },
        validate: validateAces,
      },
    }],
  }),
  projection: acesProjection,
  columns: ACE_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => {
    let held = filterCache.get(ctx.countries.filter);
    if (!held) filterCache.set(ctx.countries.filter, held = { filter: ctx.countries.filter });
    return held;
  },
  jumpToDate: true,
};
