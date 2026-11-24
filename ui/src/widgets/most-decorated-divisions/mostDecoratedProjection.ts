import { countryRowPresentation } from '../../families/bars/model.ts';
import { inHourRanges, nextChangeAfter } from '../../families/holders.ts';
import type { DeclaredCountryFilter, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, textValue, notRecorded, zeroValue, type Value } from '../../families/value.ts';
import type { CareerRow, CareerTemplate, DivisionCareersReply } from '../../generated/dashboard';
import { validateDivisionCareers } from '../division-service-record/careersData.ts';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export interface DecoratedOptions { readonly filter: DeclaredCountryFilter }

interface Media { readonly icon: string | undefined; readonly pending: boolean }

interface DecoratedRow {
  readonly row: CareerRow;
  readonly template: CareerTemplate | undefined;
  readonly kind: Media;
  readonly flag: Media;
  readonly country: Value<string>;
  readonly medals: readonly TooltipExtraRow[];
}

const NONE: Media = { icon: undefined, pending: false };

function urlMedia(url: string | null | undefined, readView: BoundedReadView): Media {
  if (!url) return NONE;
  const art = readView.art({ kind: 'url', url });
  return art.status === 'ready' ? { icon: art.url, pending: false } : { icon: undefined, pending: art.status === 'pending' };
}

function countryOf(tag: string | null, readView: BoundedReadView): { readonly flag: Media; readonly country: Value<string> } {
  if (tag === null) return { flag: NONE, country: notRecorded('Country not recorded') };
  const shown = countryRowPresentation(readView, tag);
  return {
    flag: { icon: shown.media.kind === 'flag' ? shown.media.iconUrl : undefined, pending: shown.media.kind === 'flag' && shown.media.art.status === 'pending' },
    country: shown.labelMissing ? notRecorded('Country name not recorded') : textValue(shown.label, 'Country name not recorded'),
  };
}

const count = (value: number | null, reason: string): Value<number> =>
  value === null ? notRecorded(reason) : value === 0 ? zeroValue(0) : knownValue(value);

function awards(row: CareerRow, reply: DivisionCareersReply): TooltipExtraRow[] {
  return row.facts.awards.map((award, at) => {
    const medal = reply.medals[award.medal];
    return {
      label: medal?.name ?? 'Medal name unavailable',
      value: knownValue(award.count === 1 ? 'Awarded' : `×${award.count}`),
      ...(medal ? { lookupKey: medal.key } : {}),
      indent: true,
      ...(at === 0 ? { divider: true } : {}),
    };
  });
}

const [KIND, COUNTRY, DIVISION, MEDALS, EXPERIENCE, KILLS] = COLUMNS;

const DECORATED_COLUMNS: readonly DeclarativeColumnSpec<DecoratedRow, DecoratedOptions>[] = [
  { ...KIND!, priority: 5, sortable: false, iconSlot: true, icon: item => item.kind.icon, iconPending: item => item.kind.pending,
    value: item => textValue(item.template?.name ?? null, 'Template name unavailable') },
  { ...COUNTRY!, priority: 1, iconSlot: true, icon: item => item.flag.icon, iconPending: item => item.flag.pending, country: item => item.row.tag, value: item => item.country },
  { ...DIVISION!, priority: 0, text: true, hideable: false, widest: ['175 Guangxi Juntuán'],
    value: item => textValue(item.row.name, item.row.nameMissing ?? 'Division name unavailable') },
  { ...MEDALS!, priority: 2, numeric: true, format: 'integer', widest: ['99'],
    value: item => count(item.row.facts.medals, item.row.facts.historyMissing ?? 'Medals not recorded') },
  { ...EXPERIENCE!, priority: 4, text: true, widest: ['Veterans'],
    value: item => textValue(item.row.facts.levelName, 'Experience not recorded'),
    sortValue: item => item.row.facts.experienceRaw === null ? notRecorded('Experience not recorded') : knownValue(BigInt(item.row.facts.experienceRaw)) },
  { ...KILLS!, priority: 3, numeric: true, format: 'integer', widest: ['999,999'],
    value: item => count(item.row.facts.kills, 'Casualties dealt not recorded') },
];

export const mostDecoratedProjection: TableProjection<DecoratedOptions> = registerTableProjection(
  declareTableProjection<DivisionCareersReply, DecoratedRow, DecoratedOptions>({
    kind: 'most-decorated-divisions',
    validate: validateDivisionCareers,
    nextChange: (readView, options) => nextChangeAfter(options.filter.bounds, readView.hour),
    columns: DECORATED_COLUMNS,
    rows: (reply, _live, hour, options, readView) => {
      if (reply.missing !== null) return { rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      const passing = options.filter.passing;
      const unrecorded = options.filter.unrecorded.find(([first, last]) => first <= hour && hour <= last)?.[2] ?? null;
      if (passing && unrecorded !== null) return { rows: [], empty: unrecorded, emptyKind: 'unavailable', warnings: reply.warnings };
      const templates = new Map(reply.templates.map(template => [template.template, template]));
      const kinds = new Map<string, Media>();
      const rows: DecoratedRow[] = [];
      for (const row of reply.divisions) {
        if (passing && (row.tag === null || !inHourRanges(passing[row.tag], hour))) continue;
        const template = templates.get(row.template);
        let kind = kinds.get(row.template);
        if (!kind) kinds.set(row.template, kind = urlMedia(template?.icon, readView));
        rows.push({ row, template, kind, ...countryOf(row.tag, readView), medals: awards(row, reply) });
      }
      return { rows, ...(!rows.length ? { empty: 'No divisions at this date', emptyKind: 'empty' as const } : {}), warnings: reply.warnings };
    },
    rowKey: item => item.row.division,
    rowName: item => item.row.name ?? 'Division name unavailable',
    tooltipExtra: item => item.medals.length ? item.medals : undefined,
  }),
);

const filterCache = new WeakMap<DeclaredCountryFilter, DecoratedOptions>();
export const mostDecoratedDeclaration: TableWidgetDeclaration<DecoratedOptions> = {
  family: 'table',
  kind: 'most-decorated-divisions',
  label: 'Most decorated divisions',
  subject: 'divisions',
  emptyCampaign: 'Load a campaign to see divisions.',
  icon: 'star',
  countries: { mode: 'filter' },
  sources: ctx => ({
    owner: `most-decorated-divisions:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'all' },
        product: 'division-careers',
        followQuery: { panel: 'division-careers', branchId: ctx.branch.id, countries: null },
        params: { branchId: ctx.branch.id, countries: null },
        validate: validateDivisionCareers,
      },
    }],
  }),
  projection: mostDecoratedProjection,
  columns: DECORATED_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => {
    let held = filterCache.get(ctx.countries.filter);
    if (!held) filterCache.set(ctx.countries.filter, held = { filter: ctx.countries.filter });
    return held;
  },
  jumpToDate: true,
};
