import { productIconArt } from '../../families/art/provider.ts';
import { unavailableArt, type ArtResult } from '../../families/art/types.ts';
import { countryRowPresentation } from '../../families/bars/model.ts';
import { nextChangeAfter } from '../../families/holders.ts';
import type { DeclaredCountryFilter, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableCellIconItem, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import type { BoundedReadView, FamilyViewportRequest } from '../../families/plan.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { CELL_INSET, ROW_HEIGHT, TABLE_SETTINGS_KEY, tableSettingsOf, type TableSettings } from '../../families/table/settings.ts';
import { ICON_STRIP_GAP, MEDAL_ICON_SIZE } from '../../draw/kit.ts';
import { knownValue, textValue, notRecorded, zeroValue, type Value } from '../../families/value.ts';
import type { CareerMedal, CareerRow, CareerTemplate, CountryHours, DecoratedDivisionsReply, DecoratedRank } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export interface DecoratedOptions {
  readonly filter: DeclaredCountryFilter;
  readonly descending: boolean;
}


interface DecoratedRow {
  readonly row: CareerRow;
  readonly position: number;
  readonly template: CareerTemplate | undefined;
  readonly kind: ArtResult;
  readonly flag: ArtResult;
  readonly country: Value<string>;
  readonly strip: readonly TableCellIconItem[];
}

const UNIDENTIFIED = 'Medal not identified';
const PREMEASURE_ROWS = 32;

const RANKS: Readonly<Record<string, DecoratedRank>> = { medals: 'medals', experience: 'experience', casualtiesTaken: 'casualtiesTaken' };

export function decoratedRank(settings: TableSettings): { readonly by: DecoratedRank; readonly descending: boolean } {
  const by = RANKS[settings.sort.column];
  return by ? { by, descending: settings.sort.direction === 'descending' } : { by: 'medals', descending: true };
}

export function visibleRows(view: Pick<FamilyViewportRequest, 'height'>, settings: TableSettings): number {
  return view.height === undefined ? PREMEASURE_ROWS : Math.max(1, Math.ceil(view.height / ROW_HEIGHT[settings.density]));
}

export function countryHours(filter: DeclaredCountryFilter): CountryHours[] | null {
  if (filter.passing === null) return null;
  return Object.entries(filter.passing).flatMap(([tag, ranges]) => ranges.map(([firstHour, lastHour]) => ({ tag, firstHour, lastHour })));
}

export function validateDecoratedDivisions(value: unknown): DecoratedDivisionsReply {
  const reply = value as DecoratedDivisionsReply;
  if (!reply || !Array.isArray(reply.divisions) || !Array.isArray(reply.medals) || !Array.isArray(reply.templates) || !Array.isArray(reply.warnings)
    || reply.divisions.some(row => typeof row.division !== 'string' || !Array.isArray(row.facts.awards)))
    throw new Error('Invalid decorated divisions reply');
  return reply;
}

function countryOf(tag: string | null, readView: BoundedReadView): { readonly flag: ArtResult; readonly country: Value<string> } {
  if (tag === null) return { flag: unavailableArt('Country not recorded'), country: notRecorded('Country not recorded') };
  const shown = countryRowPresentation(readView, tag);
  return {
    flag: shown.media.art,
    country: shown.labelMissing ? notRecorded('Country name not recorded') : textValue(shown.label, 'Country name not recorded'),
  };
}

const count = (value: number | null, reason: string): Value<number> =>
  value === null ? notRecorded(reason) : value === 0 ? zeroValue(0) : knownValue(value);

const medalName = (medal: CareerMedal | undefined, index: number | null) =>
  index === null ? UNIDENTIFIED : medal?.name ?? 'Medal name unavailable';

function awards(row: CareerRow, reply: DecoratedDivisionsReply): TooltipExtraRow[] {
  return row.facts.awards.map((award, at) => {
    const medal = award.medal === null ? undefined : reply.medals[award.medal];
    return {
      label: medalName(medal, award.medal),
      value: knownValue(award.count === 1 ? 'Awarded' : `×${award.count}`),
      ...(medal ? { lookupKey: medal.key } : {}),
      indent: true,
      ...(at === 0 ? { divider: true } : {}),
    };
  });
}

function medalArt(medal: CareerMedal | undefined, readView: BoundedReadView): ArtResult {
  return medal?.icon ? readView.art({ kind: 'sprite', key: medal.icon.key, frame: medal.icon.frame }) : unavailableArt('The recording names no icon for this medal');
}

function medalStrip(row: CareerRow, reply: DecoratedDivisionsReply, art: readonly ArtResult[]): TableCellIconItem[] {
  return row.facts.awards.map(award => ({
    name: medalName(award.medal === null ? undefined : reply.medals[award.medal], award.medal),
    art: award.medal === null ? unavailableArt('The medal is not identified') : art[award.medal]!,
    ...(award.count > 1 ? { count: award.count } : {}),
  }));
}

const [KIND, COUNTRY, DIVISION, MEDALS, EXPERIENCE, CASUALTIES] = COLUMNS;

export const CASUALTIES_TAKEN_NOTE = 'The game’s own count of the men this division has lost, as shown on its history tab in the game. It leaves out the men lost when a division is destroyed and losses from paradrop landings.';

const MEDALS_SHOWN = 4;

const inProductOrder = (item: DecoratedRow, options: DecoratedOptions): Value<number> => knownValue(options.descending ? -item.position : item.position);

const DECORATED_COLUMNS: readonly DeclarativeColumnSpec<DecoratedRow, DecoratedOptions>[] = [
  { ...KIND!, priority: 5, sortable: false, cellKind: 'kind', extra: () => ({ artShape: 'counter' }), iconSlot: true, art: item => item.kind,
    value: item => textValue(item.template?.name ?? null, 'Template name unavailable'), sortValue: (item, _live, _hour, options) => inProductOrder(item, options) },
  { ...COUNTRY!, priority: 1, sortable: false, iconSlot: true, art: item => item.flag, country: item => item.row.tag, value: item => item.country,
    sortValue: (item, _live, _hour, options) => inProductOrder(item, options) },
  { ...DIVISION!, priority: 0, sortable: false, text: true, cellKind: 'item', hideable: false, widest: ['175 Guangxi Juntuán'],
    value: item => textValue(item.row.name, item.row.nameMissing ?? 'Division name unavailable'), sortValue: (item, _live, _hour, options) => inProductOrder(item, options) },
  { ...MEDALS!, priority: 2, numeric: true, cellKind: 'icons', format: 'integer', minWidth: MEDALS_SHOWN * MEDAL_ICON_SIZE + (MEDALS_SHOWN - 1) * ICON_STRIP_GAP + CELL_INSET * 2,
    extra: item => ({ iconStripKind: 'medals', icons: item.strip }),
    value: item => count(item.row.facts.medals, item.row.facts.historyMissing ?? 'Medals not recorded'), sortValue: (item, _live, _hour, options) => inProductOrder(item, options) },
  { ...EXPERIENCE!, priority: 4, text: true, widest: ['Veterans'],
    value: item => textValue(item.row.facts.levelName, 'Experience not recorded'), sortValue: (item, _live, _hour, options) => inProductOrder(item, options) },
  { ...CASUALTIES!, priority: 3, numeric: true, format: 'integer', widest: ['999,999'], hint: CASUALTIES_TAKEN_NOTE,
    value: item => count(item.row.facts.casualtiesTaken, 'Casualties taken not recorded'), sortValue: (item, _live, _hour, options) => inProductOrder(item, options) },
];

export const mostDecoratedProjection: TableProjection<DecoratedOptions> = registerFamilyProjection(
  declareTableProjection<DecoratedDivisionsReply, DecoratedRow, DecoratedOptions>({
    kind: 'most-decorated-divisions',
    validate: validateDecoratedDivisions,
    nextChange: (readView, options) => nextChangeAfter(options.filter.bounds, readView.hour),
    columns: DECORATED_COLUMNS,
    rows: (reply, _live, hour, options, readView) => {
      const warnings = reply.partial === null ? reply.warnings : [reply.partial, ...reply.warnings];
      if (reply.missing !== null) return { rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings };
      const unrecorded = options.filter.unrecorded.find(([first, last]) => first <= hour && hour <= last)?.[2] ?? null;
      if (options.filter.passing && unrecorded !== null) return { rows: [], empty: unrecorded, emptyKind: 'unavailable', warnings };
      const templates = new Map(reply.templates.map(template => [template.template, template]));
      const medalPictures = reply.medals.map(medal => medalArt(medal, readView));
      const kinds = new Map<string, ArtResult>();
      const rows = reply.divisions.map((row, position): DecoratedRow => {
        const template = templates.get(row.template);
        let kind = kinds.get(row.template);
        if (!kind) kinds.set(row.template, kind = productIconArt(readView, template?.icon, 'The product drew no counter for this template'));
        return { row, position, template, kind, ...countryOf(row.tag, readView), strip: medalStrip(row, reply, medalPictures) };
      });
      return { rows, ...(!rows.length ? { empty: 'No divisions at this date', emptyKind: 'empty' as const } : {}), warnings };
    },
    rowKey: item => item.row.division,
    rowName: item => item.row.name ?? 'Division name unavailable',
    tooltipExtra: (item, reply) => (item.row.facts.awards.length ? awards(item.row, reply) : undefined),
  }),
);

const optionsCache = new WeakMap<DeclaredCountryFilter, Map<boolean, DecoratedOptions>>();
export const mostDecoratedDeclaration: TableWidgetDeclaration<DecoratedOptions> = {
  family: 'table',
  kind: 'most-decorated-divisions',
  label: 'Most decorated divisions',
  subject: 'divisions',
  emptyCampaign: 'Load a campaign to see divisions.',
  icon: 'star',
  countries: { mode: 'filter' },
  sources: (ctx, view) => {
    const settings = tableSettingsOf(ctx.options[TABLE_SETTINGS_KEY], COLUMNS, TABLE_BASE);
    const params = {
      branchId: ctx.branch.id,
      countries: countryHours(ctx.countries.filter),
      ...decoratedRank(settings),
      count: visibleRows(view, settings),
    };
    return {
      owner: `most-decorated-divisions:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      entries: [{
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'population', form: 'top' },
          product: 'decorated-divisions',
          followQuery: { panel: 'decorated-divisions', ...params },
          params,
          validate: validateDecoratedDivisions,
        },
      }],
    };
  },
  projection: mostDecoratedProjection,
  columns: DECORATED_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => {
    const descending = ctx.settings.sort.direction === 'descending';
    let byDirection = optionsCache.get(ctx.countries.filter);
    if (!byDirection) optionsCache.set(ctx.countries.filter, byDirection = new Map());
    let held = byDirection.get(descending);
    if (!held) byDirection.set(descending, held = { filter: ctx.countries.filter, descending });
    return held;
  },
  jumpToDate: true,
};
