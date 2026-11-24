import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { formatDateOnly, formatScalar } from '../../families/valueFormat.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { DeclaredCountryFilter, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { identityCell, newIdentityBoundary } from '../../families/table/identityCell.ts';
import type { TableCell, TableColumnSpec, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notRecorded, valueFromLive, valueNumber, zeroValue, type Value } from '../../families/value.ts';
import { object, opaqueTag } from '../../dashboard/validation.ts';
import type { PoliticsStateReply } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export interface ExilesProjectionOptions {
  readonly filter: DeclaredCountryFilter;
}

const optionalText = (value: unknown) => value === null || typeof value === 'string';

export function validateExiles(value: unknown): PoliticsStateReply {
  if (!object(value) || typeof value.branchId !== 'string' || value.view !== 'exiles' || !Array.isArray(value.exiles)
    || !value.exiles.every(exile => object(exile) && typeof exile.id === 'string' && opaqueTag(exile.tag) && (exile.host === null || opaqueTag(exile.host)))
    || !Array.isArray(value.unavailable) || !value.unavailable.every(entry => object(entry) && typeof entry.label === 'string' && optionalText(entry.reason))
    || !Array.isArray(value.warnings) || value.warnings.some(warning => typeof warning !== 'string'))
    throw new Error('The governments in exile reply is malformed.');
  return value as unknown as PoliticsStateReply;
}

const PRIORITY: Readonly<Record<string, number>> = { country: 0, manpower: 1, legitimacy: 2, host: 3, leaders: 4, air: 5 };
export const EXILE_COLUMNS: readonly TableColumnSpec[] = COLUMNS.map(column => ({
  ...column,
  priority: PRIORITY[column.key] ?? 9,
  numeric: column.key !== 'country' && column.key !== 'host',
  text: column.key === 'country' || column.key === 'host',
  ...(column.key === 'country' ? { pinned: true } : {}),
  ...(column.key === 'legitimacy' ? { cellKind: 'track' as const, trackTone: 'grey' as const, format: { decimals: 0, maxDecimals: 0, percent: true } as const, widest: ['100%'] } : {}),
  ...(column.key === 'manpower' ? { cellKind: 'outOf' as const, barEligible: true, unit: 'available / total', format: 'integer' as const, widest: ['999,999 / 9,999,999'] } : {}),
  ...(column.key === 'air' || column.key === 'leaders' ? { barEligible: true } : {}),
}));

const passes = (filter: DeclaredCountryFilter, tag: string | null, hour: number): boolean =>
  tag !== null && (filter.passing === null || !!filter.passing[tag]?.some(([first, last]) => first <= hour && hour <= last));

const unrecordedAt = (filter: DeclaredCountryFilter, hour: number): string | null =>
  filter.unrecorded.find(([first, last]) => first <= hour && hour <= last)?.[2] ?? null;

function filterNext(filter: DeclaredCountryFilter, hour: number): number {
  for (const bound of filter.bounds) if (bound > hour) return bound;
  return Infinity;
}

const count = (live: LiveValues<number | bigint | string> | null, id: string, cell: string): Value => valueFromLive(live, liveKey(id, cell));

function manpowerCell(available: Value, cap: Value, top: number): TableCell {
  const known = valueNumber(available);
  const total = valueNumber(cap);
  if (known === undefined) return { value: available, bar: null };
  return {
    value: available,
    format: 'integer',
    bar: top > 0 && known > 0 ? { fraction: knownValue(known / top) } : null,
    ...(total !== undefined ? { secondaryValue: knownValue(formatScalar(total, 'integer')) } : {}),
  };
}

function exileExtra(live: LiveValues<number | bigint | string> | null, id: string, hour: number): readonly TooltipExtraRow[] {
  const read = valueNumber(count(live, id, 'readHour'));
  return [
    { label: 'Exile manpower cap', value: count(live, id, 'cap'), format: 'integer', divider: true },
    ...(read !== undefined && read < hour ? [{ label: 'Resources last read', value: knownValue(formatDateOnly(read)), tone: 'muted' as const }] : []),
  ];
}

export const exilesProjection: TableProjection<ExilesProjectionOptions> = registerTableProjection({
  kind: 'governments-in-exile',
  family: 'table',
  nextChange: (readView: BoundedReadView, options: ExilesProjectionOptions) => filterNext(options.filter, readView.hour),
  project(readView, options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) return { columns: EXILE_COLUMNS, rows: [], hour, refused };
    const raw = readView.panel('main');
    if (raw === undefined) return { columns: EXILE_COLUMNS, rows: [], hour };
    const reply = validateExiles(raw);
    const live = readView.live('main') ?? null;
    const status = reply.unavailable.find(entry => entry.label === 'exile status');
    if (status) {
      return { columns: EXILE_COLUMNS, rows: [], hour, empty: 'Not recorded', emptyDetail: status.reason ?? 'Exiles are not in this recording at this date', emptyKind: 'unavailable', warnings: reply.warnings };
    }
    const shown = reply.exiles.filter(exile => passes(options.filter, exile.tag, hour) || passes(options.filter, exile.host, hour));
    let top = 0;
    for (const exile of shown) top = Math.max(top, valueNumber(count(live, exile.id, 'manpower')) ?? 0);
    const boundary = newIdentityBoundary();
    const rows: TableRow[] = shown.map(exile => {
      const manpower = manpowerCell(count(live, exile.id, 'manpower'), count(live, exile.id, 'cap'), top);
      const legitimacy = count(live, exile.id, 'legitimacy');
      const legitimacyShare = valueNumber(legitimacy);
      return {
        key: exile.id,
        name: exile.tag,
        cells: {
          country: identityCell(readView, 'country', exile.tag, { boundary }),
          host: exile.host === null ? { value: notRecorded('The host has no identity at this hour'), iconSlot: true } : identityCell(readView, 'country', exile.host, { boundary }),
          legitimacy: {
            value: legitimacy,
            bar: legitimacyShare === undefined ? null : { fraction: legitimacyShare === 0 ? zeroValue(0) : knownValue(Math.max(0, Math.min(1, legitimacyShare / 100))) },
          },
          manpower,
          air: { value: count(live, exile.id, 'deployedAir'), format: 'integer' },
          leaders: { value: count(live, exile.id, 'leaders'), format: 'integer' },
        },
        tooltipExtra: () => exileExtra(live, exile.id, hour),
      };
    });
    const membership = unrecordedAt(options.filter, hour);
    const warnings = reply.unavailable.filter(entry => entry.label !== 'exile status').map(entry => `${entry.label} not recorded${entry.reason ? `: ${entry.reason}` : ''}`);
    return {
      columns: EXILE_COLUMNS,
      rows,
      hour,
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      ...(rows.length === 0
        ? membership !== null
          ? { empty: 'Not recorded', emptyDetail: membership, emptyKind: 'unavailable' as const }
          : { empty: 'No governments in exile', emptyKind: 'empty' as const }
        : {}),
      warnings: [...reply.warnings, ...warnings],
    };
  },
});

export const exilesDeclaration: TableWidgetDeclaration<ExilesProjectionOptions> = {
  family: 'table',
  kind: 'governments-in-exile',
  label: 'Governments in exile',
  subject: 'governments in exile',
  emptyCampaign: 'Load a campaign to see governments in exile.',
  icon: 'flag',
  countries: { mode: 'filter' },
  sources: ctx => ({
    owner: `governments-in-exile:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'all' },
        product: 'politics-state',
        followQuery: { panel: 'politics-state', branchId: ctx.branch.id, view: 'exiles' },
        params: { branchId: ctx.branch.id, view: 'exiles' },
        validate: validateExiles,
      },
    }],
  }),
  projection: exilesProjection,
  columns: EXILE_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => ({ filter: ctx.countries.filter }),
};
