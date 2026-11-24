import { object, opaqueTag } from '../../dashboard/validation.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { DeclaredCountryFilter, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { identityCell, newIdentityBoundary } from '../../families/table/identityCell.ts';
import type { TableColumnSpec, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notRecorded, partialValue, zeroValue, type Value } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { TerritoryTransfersReply } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export interface TransfersProjectionOptions {
  readonly filter: DeclaredCountryFilter;
}

const count = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;
const integer = (value: unknown) => Number.isSafeInteger(value);
const optionalText = (value: unknown) => value === null || typeof value === 'string';

export function validateTransfers(value: unknown): TerritoryTransfersReply {
  if (!object(value) || typeof value.branchId !== 'string' || !integer(value.through) || !count(value.transfers)
    || !(value.nextChange === null || integer(value.nextChange))
    || !Array.isArray(value.countries) || !value.countries.every(row => object(row) && typeof row.key === 'string' && opaqueTag(row.tag) && integer(row.hour)
      && count(row.gained) && count(row.lost) && (row.net === null || integer(row.net))
      && (row.netVictoryPoints === null || (typeof row.netVictoryPoints === 'string' && /^-?\d+$/.test(row.netVictoryPoints))) && optionalText(row.victoryPointsMissing))
    || !optionalText(value.partial)
    || !Array.isArray(value.warnings) || value.warnings.some(warning => typeof warning !== 'string'))
    throw new Error('The territory transfers reply is malformed.');
  return value as unknown as TerritoryTransfersReply;
}

const PRIORITY: Readonly<Record<string, number>> = { country: 0, net: 1, gained: 2, lost: 3, victoryPoints: 4 };
const SIGNED = { decimals: 0, signed: true } as const;
export const TRANSFER_COLUMNS: readonly TableColumnSpec[] = COLUMNS.map(column => ({
  ...column,
  priority: PRIORITY[column.key] ?? 9,
  numeric: column.key !== 'country',
  text: column.key === 'country',
  ...(column.key === 'country' ? { pinned: true } : {}),
  ...(column.key === 'gained' || column.key === 'lost' ? { barEligible: true } : {}),
  ...(column.key === 'net' || column.key === 'victoryPoints' ? { format: SIGNED, barEligible: false, widest: ['+9,999'] } : {}),
  ...(column.key === 'net' ? { cellKind: 'twoWay' as const } : {}),
  ...(column.key === 'victoryPoints' ? { unit: 'net' } : {}),
}));

const passes = (filter: DeclaredCountryFilter, tag: string, hour: number): boolean =>
  filter.passing === null || !!filter.passing[tag]?.some(([first, last]) => first <= hour && hour <= last);

function filterNext(filter: DeclaredCountryFilter, hour: number): number {
  for (const bound of filter.bounds) if (bound > hour) return bound;
  return Infinity;
}

const whole = (value: number): Value => (value === 0 ? zeroValue(0) : knownValue(value));
const observed = (value: number, partial: string | null): Value => (partial === null ? whole(value) : partialValue(value, partial));

export const transfersProjection: TableProjection<TransfersProjectionOptions> = registerTableProjection({
  kind: 'territory-transfers',
  family: 'table',
  nextChange: (readView: BoundedReadView, options: TransfersProjectionOptions) => filterNext(options.filter, readView.hour),
  project(readView, options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) return { columns: TRANSFER_COLUMNS, rows: [], hour, refused };
    const raw = readView.panel('main');
    if (raw === undefined) return { columns: TRANSFER_COLUMNS, rows: [], hour };
    const reply = validateTransfers(raw);
    const boundary = newIdentityBoundary();
    const rows: TableRow[] = [];
    const shown = reply.countries.filter(country => passes(options.filter, country.tag, hour));
    for (const country of shown) {
      const points = country.netVictoryPoints === null ? null : BigInt(country.netVictoryPoints);
      rows.push({
        key: country.key,
        name: country.tag,
        cells: {
          country: identityCell(readView, 'country', country.tag, { atHour: Math.min(country.hour, hour), boundary }),
          gained: { value: observed(country.gained, reply.partial), format: 'integer' },
          lost: { value: observed(country.lost, reply.partial), format: 'integer' },
          net: { value: country.net === null ? notRecorded(reply.partial ?? 'Net not recorded') : whole(country.net) },
          victoryPoints: { value: points === null ? notRecorded(country.victoryPointsMissing ?? 'Victory points not recorded') : points === 0n ? zeroValue(0n) : knownValue(points) },
        },
      });
    }
    return {
      columns: TRANSFER_COLUMNS,
      rows,
      hour,
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      titleSummary: `${formatScalar(reply.transfers, 'integer')} ${reply.transfers === 1 ? 'transfer' : 'transfers'}`,
      ...(reply.partial !== null ? { titleSummaryPartial: reply.partial } : {}),
      ...(rows.length === 0
        ? reply.partial === null
          ? { empty: 'No territory transfers yet', emptyKind: 'empty' as const }
          : { empty: 'No transfers observed', emptyDetail: reply.partial, emptyKind: 'unavailable' as const }
        : {}),
      warnings: reply.warnings,
    };
  },
});

export const transfersDeclaration: TableWidgetDeclaration<TransfersProjectionOptions> = {
  family: 'table',
  kind: 'territory-transfers',
  label: 'Territory transfers',
  subject: 'territory transfers',
  emptyCampaign: 'Load a campaign to see territory transfers.',
  icon: 'flows',
  countries: { mode: 'filter' },
  sources: ctx => ({
    owner: `territory-transfers:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'total' },
        product: 'territory-transfers',
        followQuery: null,
        params: { branchId: ctx.branch.id },
        hourParam: 'through',
        provenInterval: (data, hour) => {
          const reply = data as TerritoryTransfersReply;
          const last = reply.nextChange === null ? Number.MAX_SAFE_INTEGER : reply.nextChange - 1;
          return reply.through <= hour && hour <= last ? [reply.through, last] : null;
        },
        validate: validateTransfers,
      },
    }],
  }),
  projection: transfersProjection,
  columns: TRANSFER_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => ({ filter: ctx.countries.filter }),
};
