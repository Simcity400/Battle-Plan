import { productIconArt } from '../../families/art/provider.ts';
import { unavailableArt, type ArtResult } from '../../families/art/types.ts';
import { countryRowPresentation } from '../../families/bars/model.ts';
import { inHourRanges, nextChangeAfter } from '../../families/holders.ts';
import type { DeclaredCountryFilter, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, textValue, notRecorded, type Value } from '../../families/value.ts';
import { formatDateHover, formatDateOnly } from '../../families/valueFormat.ts';
import type { AirAce, AirAcesReply } from '../../generated/dashboard';
import { validateAces } from './acesData.ts';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export interface AcesOptions { readonly filter: DeclaredCountryFilter }

interface AceRow {
  readonly ace: AirAce;
  readonly portrait: ArtResult;
  readonly flag: ArtResult;
  readonly country: Value<string>;
  readonly killerCountry: Value<string> | null;
}

const NO_COUNTRY: Value<string> = notRecorded('Country not recorded');

function portraitMedia(ace: AirAce, readView: BoundedReadView): ArtResult {
  return productIconArt(readView, ace.portrait, ace.portraitMissing ?? 'Portrait not recorded');
}

function countryOf(tag: string | null, readView: BoundedReadView, atHour?: number): { readonly flag: ArtResult; readonly country: Value<string> } {
  if (tag === null) return { flag: unavailableArt('Country not recorded'), country: NO_COUNTRY };
  const shown = countryRowPresentation(readView, tag, undefined, undefined, atHour);
  return {
    flag: shown.media.art,
    country: shown.labelMissing ? notRecorded('Country name not recorded') : textValue(shown.label, 'Country name not recorded'),
  };
}

const wingValue = (ace: AirAce): Value<string> => ace.wing === null
  ? knownValue('No wing')
  : textValue(ace.wingName, 'Wing name not recorded');

function aircraftValue(ace: AirAce): Value<string> {
  if (ace.wing === null) return knownValue('None');
  if (ace.aircraft === null) return notRecorded('Aircraft not recorded');
  if (!ace.aircraft.length) return knownValue('None');
  return knownValue(ace.aircraft.map(plane => plane.name ?? 'Aircraft name unavailable').join(' · '));
}

const [PORTRAIT, COUNTRY, ACE, WING, SINCE] = COLUMNS;

const nickname = (ace: AirAce): string | undefined => ace.callsign?.trim() ? `“${ace.callsign}”` : undefined;

function aircraftLine(ace: AirAce): string | undefined {
  const value = aircraftValue(ace);
  return ace.wing !== null && value.kind === 'known' ? value.value : undefined;
}

const ACE_COLUMNS: readonly DeclarativeColumnSpec<AceRow, AcesOptions>[] = [
  { ...PORTRAIT!, priority: 4, sortable: false, cellKind: 'kind', iconSlot: true, art: row => row.portrait,
    extra: row => ({ artShape: 'portrait', ...(row.ace.name ? { artName: row.ace.name } : {}) }),
    value: row => row.ace.portrait === null ? notRecorded(row.ace.portraitMissing ?? 'Portrait not recorded') : textValue(row.ace.name, 'Ace name not recorded') },
  { ...COUNTRY!, priority: 1, iconSlot: true, art: row => row.flag, country: row => row.ace.tag, value: row => row.country },
  { ...ACE!, priority: 0, text: true, hideable: false, cellKind: 'item', widest: ['The Blond Knight of Germany'],
    extra: row => (nickname(row.ace) ? { subtext: nickname(row.ace) } : {}),
    value: row => textValue(row.ace.name, 'Ace name not recorded') },
  { ...WING!, priority: 2, text: true, widest: ['Sturzkampfgeschwader 2'], value: row => wingValue(row.ace),
    extra: row => (aircraftLine(row.ace) ? { subtext: aircraftLine(row.ace) } : {}) },
  { ...SINCE!, priority: 3, numeric: true, cellKind: 'date', widest: ['by 30 Sep 1941'],
    extra: row => (row.ace.firstSeen === null ? {} : { boundedDate: 'by' }),
    value: row => row.ace.firstSeen === null ? notRecorded('First sighting not recorded') : knownValue(row.ace.firstSeen) },
];

function aceTooltip(row: AceRow): TooltipExtraRow[] {
  const ace = row.ace;
  const rows: TooltipExtraRow[] = [
    ...(ace.callsign ? [{ label: 'Nickname', value: knownValue(`“${ace.callsign}”`) }] : []),
    { label: 'Aircraft', value: aircraftValue(ace) },
    { label: 'Portrait', value: ace.portrait === null ? notRecorded(ace.portraitMissing ?? 'Portrait not recorded') : knownValue('Current installation') },
    { label: 'Became an ace', value: ace.firstSeen === null ? notRecorded('First sighting not recorded') : knownValue(`by ${formatDateHover(ace.firstSeen)}`), divider: true },
    { label: 'Status', value: knownValue(ace.alive ? 'Alive' : 'Killed') },
  ];
  const death = ace.death;
  if (death) {
    rows.push({ label: 'Killed', value: death.byHour === null || death.lastAlive === null
      ? notRecorded('Date of death not recorded')
      : knownValue(`after ${formatDateOnly(death.lastAlive)}, by ${formatDateOnly(death.byHour)}`) });
    if (death.killerAce) rows.push({ label: 'Killed by', value: textValue(death.killerName, 'Killer ace name not recorded') });
    rows.push({ label: 'Killer country', value: row.killerCountry ?? notRecorded('Killer country not recorded') });
  }
  return rows;
}

export const acesProjection: TableProjection<AcesOptions> = registerFamilyProjection(
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
        const seenDead = ace.death?.seenDead ?? null;
        const killerCountry = killerTag === null || seenDead === null ? null : countryOf(killerTag, readView, Math.min(seenDead, hour)).country;
        rows.push({ ace, portrait: portraitMedia(ace, readView), ...countryOf(ace.tag, readView), killerCountry });
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
  icon: 'person',
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
