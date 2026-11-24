import { countryFromKey } from '../../dashboard/countryKey.ts';
import { int, object, opaqueTag } from '../../dashboard/validation.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { DeclaredCountryFilter, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { identityCell, newIdentityBoundary } from '../../families/table/identityCell.ts';
import type { TableCell, TableColumnSpec, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { ElectionEntry, NextElectionsReply } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export interface ElectionsProjectionOptions {
  readonly filter: DeclaredCountryFilter;
}

const HOURS_A_DAY = 24;

export function validateElections(value: unknown): NextElectionsReply {
  const reply = value as Record<string, unknown>;
  const ok = object(reply) && typeof reply.branchId === 'string' && typeof reply.recorded === 'boolean'
    && int(reply.unknown) && reply.unknown >= 0
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.rows) && reply.rows.every(entry => object(entry)
      && typeof entry.key === 'string' && entry.key.length > 0 && opaqueTag(entry.tag)
      && (entry.interval === null || int(entry.interval))
      && (entry.next === null || int(entry.next))
      && (entry.last === null || int(entry.last)));
  if (!ok) throw new Error('The next elections reply is malformed.');
  return value as NextElectionsReply;
}

const PRIORITY: Readonly<Record<string, number>> = { country: 0, days: 1, election: 2, term: 3 };
export const ELECTION_COLUMNS: readonly TableColumnSpec[] = COLUMNS.map(column => ({
  ...column,
  priority: PRIORITY[column.key] ?? 9,
  numeric: column.key !== 'country',
  text: column.key === 'country',
  ...(column.key === 'country' ? { pinned: true } : {}),
  ...(column.key === 'term' ? { cellKind: 'track' as const, trackTone: 'grey' as const, widest: ['100%'] } : {}),
  ...(column.key === 'days' ? { barEligible: false, format: 'integer' as const, widest: ['9,999'] } : {}),
  ...(column.key === 'election' ? { cellKind: 'date' as const, widest: ['30 Sep'] } : {}),
}));

const passes = (filter: DeclaredCountryFilter, tag: string, hour: number): boolean =>
  filter.passing === null || !!filter.passing[tag]?.some(([first, last]) => first <= hour && hour <= last);

const unrecordedAt = (filter: DeclaredCountryFilter, hour: number): string | null =>
  filter.unrecorded.find(([first, last]) => first <= hour && hour <= last)?.[2] ?? null;

function filterNext(filter: DeclaredCountryFilter, hour: number): number {
  for (const bound of filter.bounds) if (bound > hour) return bound;
  return Infinity;
}

const day = (hour: number) => Math.floor(hour / HOURS_A_DAY);
const nextDay = (hour: number) => (day(hour) + 1) * HOURS_A_DAY;

function termCell(entry: ElectionEntry, hour: number): TableCell {
  if (entry.next === null || entry.last === null || entry.next <= entry.last) {
    return { value: notRecorded('The term’s start or the next election is not recorded'), bar: null };
  }
  const share = Math.max(0, Math.min(1, (day(hour) * HOURS_A_DAY - entry.last) / (entry.next - entry.last)));
  return {
    value: knownValue(`${Math.floor(share * 100)}%`),
    sortValue: share === 0 ? zeroValue(0) : knownValue(share),
    bar: { fraction: share === 0 ? zeroValue(0) : knownValue(share) },
  };
}

function electionExtra(entry: ElectionEntry): readonly TooltipExtraRow[] {
  return [{
    label: 'Election interval',
    value: entry.interval === null
      ? notRecorded('Not recorded')
      : knownValue(`Every ${formatScalar(entry.interval, 'integer')} ${entry.interval === 1 ? 'month' : 'months'}`),
    divider: true,
  }];
}

export const electionsProjection: TableProjection<ElectionsProjectionOptions> = registerTableProjection({
  kind: 'next-elections',
  family: 'table',
  nextChange: (readView: BoundedReadView, options: ElectionsProjectionOptions) => Math.min(filterNext(options.filter, readView.hour), nextDay(readView.hour)),
  project(readView, options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) return { columns: ELECTION_COLUMNS, rows: [], hour, refused };
    const raw = readView.panel('main');
    if (raw === undefined) return { columns: ELECTION_COLUMNS, rows: [], hour };
    const reply = validateElections(raw);
    if (!reply.recorded) {
      return { columns: ELECTION_COLUMNS, rows: [], hour, empty: 'Not recorded', emptyDetail: 'Politics are not in this recording at this date', emptyKind: 'unavailable', warnings: reply.warnings };
    }
    const boundary = newIdentityBoundary();
    const rows: TableRow[] = [];
    for (const entry of reply.rows) {
      if (!passes(options.filter, entry.tag, hour)) continue;
      const days = entry.next === null ? null : Math.max(0, day(entry.next) - day(hour));
      rows.push({
        key: entry.key,
        name: entry.tag,
        cells: {
          country: identityCell(readView, 'country', entry.tag, { boundary }),
          term: termCell(entry, hour),
          days: { value: days === null ? notRecorded('The next election is not recorded') : days === 0 ? zeroValue(0) : knownValue(days) },
          election: { value: entry.next === null ? notRecorded('The next election is not recorded') : knownValue(entry.next) },
        },
        tooltipExtra: () => electionExtra(entry),
      });
    }
    const membership = unrecordedAt(options.filter, hour);
    const unknown = reply.unknown ? `${reply.unknown} countries' election schedule or eligibility was not recorded at this date` : null;
    return {
      columns: ELECTION_COLUMNS,
      rows,
      hour,
      ...(unknown !== null ? { populationPartial: unknown } : {}),
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      ...(rows.length === 0
        ? membership !== null
          ? { empty: 'Not recorded', emptyDetail: membership, emptyKind: 'unavailable' as const }
          : reply.unknown
            ? { empty: 'Not recorded', emptyDetail: 'Election eligibility is not in this recording at this date', emptyKind: 'unavailable' as const }
            : { empty: 'No elections due', emptyKind: 'empty' as const }
        : {}),
      warnings: unknown !== null ? [...reply.warnings, unknown] : reply.warnings,
    };
  },
});

export const electionsDeclaration: TableWidgetDeclaration<ElectionsProjectionOptions> = {
  family: 'table',
  kind: 'next-elections',
  label: 'Next elections',
  subject: 'elections',
  emptyCampaign: 'Load a campaign to see elections.',
  icon: 'ballot',
  countries: { mode: 'filter' },
  sources: ctx => {
    const chosen = !ctx.countries.all && ctx.countries.top === null ? ctx.countries.tags : null;
    const params = { branchId: ctx.branch.id, ...(chosen !== null ? { countries: chosen.map(countryFromKey) } : {}) };
    return {
      owner: `next-elections:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      entries: [{
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: chosen !== null ? { mode: 'keyed', keys: chosen } : { mode: 'population', form: 'all' },
          product: 'next-elections',
          followQuery: { panel: 'next-elections', ...params },
          params,
          validate: validateElections,
        },
      }],
    };
  },
  projection: electionsProjection,
  columns: ELECTION_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => ({ filter: ctx.countries.filter }),
};
