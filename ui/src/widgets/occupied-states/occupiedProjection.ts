import { productIconArt } from '../../families/art/provider.ts';
import { formatAmount, formatFixed, formatSigned } from '../../charts/valueAxis.ts';
import { liveKey } from '../../charts/live.ts';
import type { DeclaredCountryFilter, HourRanges, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import {
  declareTableProjection,
  type DeclarativeColumnSpec,
  type TableCellBar,
  type TableColumnSpec,
  type TableProjection,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { newIdentityBoundary, resolveIdentity } from '../../families/table/identityCell.ts';
import { strengthBar, TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notRecorded, partialValue, valueFromLive, zeroValue, type Value } from '../../families/value.ts';
import { COLUMNS, TABLE_BASE } from './definition.ts';
import {
  emptyReason,
  garrisonCell,
  NOT_NAMED,
  occupiedRows,
  STRENGTH,
  validateOccupiedStates,
  type CountryRole,
  type OccupiedRow,
  type OccupiedStatesReply,
} from './occupiedStatesData.ts';

export type OccupiedFilter = DeclaredCountryFilter;

export interface OccupiedProjectionOptions {
  readonly filter: OccupiedFilter;
  readonly role: CountryRole;
  readonly lawAs: string;
  readonly filterAll: boolean;
}

const PERCENT_SCALE = 10_000_000;
const PRIORITIES: Readonly<Record<string, number>> = { state: 0, resistance: 1, occupier: 2, compliance: 3, garrison: 4, law: 5, owner: 6 };

const within = (ranges: HourRanges | undefined, hour: number) => !!ranges?.some(([first, last]) => first <= hour && hour <= last);

export function filterStretch(filter: OccupiedFilter, hour: number): number {
  let low = 0, high = filter.bounds.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (filter.bounds[mid]! <= hour) low = mid + 1;
    else high = mid;
  }
  return low;
}

export const filterNext = (filter: OccupiedFilter, hour: number): number => filter.bounds[filterStretch(filter, hour)] ?? Infinity;

const toBigIntExact = (raw: number | bigint): bigint => (typeof raw === 'bigint' ? raw : BigInt(Math.trunc(raw)));

function strengthFraction(raw: number | bigint): number {
  if (typeof raw === 'bigint' || Math.abs(raw) >= 2 ** 40) {
    return Math.max(0, Math.min(1, Number(strengthBar(toBigIntExact(raw))) / 100000));
  }
  const percent = Math.trunc(raw) * 100;
  const rounded = Math.trunc((percent + (percent < 0 ? -50000 : 50000)) / 100000) * 1000;
  return Math.max(0, Math.min(1, rounded / 100000));
}

export const OCCUPIED_COLUMNS: readonly TableColumnSpec[] = COLUMNS.map(col => ({
  ...col,
  priority: PRIORITIES[col.key] ?? 9,
  pinned: col.key === 'owner' || col.key === 'state',
  cellKind: col.key === 'resistance' || col.key === 'compliance' ? 'track' : col.key === 'state' ? 'item' : undefined,
  trackTone: col.key === 'resistance' ? 'red' : col.key === 'compliance' ? 'grey' : undefined,
  barEligible: col.key === 'garrison',
  numeric: col.key === 'resistance' || col.key === 'compliance' || col.key === 'garrison',
  text: col.key === 'state' || col.key === 'owner' || col.key === 'occupier' || col.key === 'law',
  format: col.key === 'resistance' || col.key === 'compliance' ? 'fixed5Percent' : col.key === 'garrison' ? 'fixed5Amount' : undefined,
  widest: col.key === 'resistance' || col.key === 'compliance' ? ['100%'] : col.key === 'garrison' ? ['999.99'] : undefined,
}));

const lawsCache = new WeakMap<OccupiedStatesReply, ReadonlyMap<string, OccupiedStatesReply['laws'][number]>>();
function lawsOf(reply: OccupiedStatesReply) {
  let map = lawsCache.get(reply);
  if (!map) {
    map = new Map(reply.laws.map(law => [law.token, law]));
    lawsCache.set(reply, map);
  }
  return map;
}

export const occupiedStatesProjection: TableProjection<OccupiedProjectionOptions> = registerFamilyProjection(
  declareTableProjection<OccupiedStatesReply, OccupiedRow, OccupiedProjectionOptions>({
    kind: 'occupied-states',
    validate: validateOccupiedStates,
    nextChange: (readView, options) => filterNext(options.filter, readView.hour),
    columns: (reply, options): readonly DeclarativeColumnSpec<OccupiedRow, OccupiedProjectionOptions>[] => {
      const laws = lawsOf(reply);
      const redBelow = reply.garrisonRedRaw === null ? undefined : BigInt(reply.garrisonRedRaw);
      const byKey = new Map(OCCUPIED_COLUMNS.map(col => [col.key, col]));
      return [
        { ...byKey.get('owner')!, countryField: 'value.owner' },
        { ...byKey.get('state')!, path: 'value.name', whenNull: { kind: 'notRecorded', reasonField: 'value.nameMissing', reason: 'Not recorded' } },
        { ...byKey.get('resistance')!, liveField: 'resistance', maximum: PERCENT_SCALE, country: 'value.occupier' },
        { ...byKey.get('compliance')!, liveField: 'compliance', maximum: PERCENT_SCALE, country: 'value.occupier' },
        {
          ...byKey.get('law')!,
          iconSlot: options.lawAs === 'icon',
          art: (row, readView) => (row.value.law !== null ? productIconArt(readView, laws.get(row.value.law)?.icon, 'The product drew no icon for this law') : undefined),
          status: () => (options.lawAs === 'dot' ? 'dot' : undefined),
          value: row => {
            if (row.value.law === null) return notRecorded(row.value.lawMissing ?? 'Not recorded');
            const law = laws.get(row.value.law);
            if (law?.name && law.name.trim()) return knownValue(law.name);
            return notRecorded(law?.missing ?? row.value.lawMissing ?? 'Occupation law name unavailable');
          },
        },
        { ...byKey.get('occupier')!, countryField: 'value.occupier' },
        {
          ...byKey.get('garrison')!,
          country: 'value.occupier',
          value: (row, live) => {
            if (row.value.garrisonRequiredSign === null) {
              return notRecorded(row.value.garrisonMissing ?? 'Garrison data not recorded');
            }
            if (row.value.garrisonRequiredSign === 0) return zeroValue(0);
            return valueFromLive(live, liveKey(row.key, 'garrison'), { overrideMissing: row.value.garrisonMissing });
          },
          bar: (row, live): TableCellBar | null => {
            const shape = garrisonCell(row.value);
            if (shape.bar === null) return null;
            if (shape.bar === 0n) return { fraction: zeroValue(0), alert: true };
            const strengthVal = valueFromLive(live, liveKey(row.key, STRENGTH));
            if (strengthVal.kind === 'known' || strengthVal.kind === 'zero') {
              const rawBig = toBigIntExact(strengthVal.value);
              const frac = strengthFraction(strengthVal.value);
              return { fraction: frac === 0 ? zeroValue(0) : knownValue(frac), alert: redBelow !== undefined && rawBig < redBelow };
            }
            return { fraction: notRecorded(strengthVal.kind === 'notRecorded' ? strengthVal.reason : reply.strengthMissing) };
          },
        },
      ];
    },
    rows: (reply, _live, hour, options, readView) => {
      const passing = options.filter.passing;
      const unrecorded = options.filter.unrecorded.find(([first, last]) => first <= hour && hour <= last)?.[2] ?? null;
      const view = occupiedRows(reply, passing ? tag => within(passing[tag], hour) : null, options.role, unrecorded);
      const empty = emptyReason(reply, view);
      if (reply.missing !== null) {
        return { rows: [], empty: empty ?? reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      }
      const partial: string[] = [];
      if (!reply.complete) partial.push('Some occupied states are missing.');
      if (!options.filterAll && reply.states.some(state => state[options.role] === null)) {
        partial.push('Some states couldn’t be matched to these countries.');
      }
      const boundary = newIdentityBoundary();
      const rows = view.rows.map(row => ({
        ...row,
        occupiedName: row.value.occupied === null ? notRecorded('Unknown country') : resolveIdentity(readView, 'country', row.value.occupied, { boundary }).value,
      }));
      return {
        rows,
        ...(empty !== null ? { empty, emptyKind: view.membership || view.selectionUnknown || !reply.complete ? 'unavailable' : 'empty' } : {}),
        ...(partial.length ? { titleSummary: 'Partial', titleSummaryPartial: partial.join(' ') } : {}),
        warnings: partial.length ? [...reply.warnings, ...partial] : reply.warnings,
        nextChangeAt: boundary.next,
      };
    },
    rowKey: row => row.key,
    rowName: row => row.value.name ?? `State ${row.state}`,
    tooltipExtra: (row, reply, live): TooltipExtraRow[] => {
      const v = row.value;
      const liveText = (column: string, fmt: (s: string) => string): Value<string> => {
        const val = valueFromLive(live, liveKey(row.key, column));
        if (val.kind === 'known' || val.kind === 'zero') return knownValue(fmt(toBigIntExact(val.value).toString()));
        if (val.kind === 'partial') return partialValue(fmt(toBigIntExact(val.value).toString()), val.reason);
        return val;
      };
      const reqManpower: Value<string> = v.garrisonRequiredSign === null
        ? notRecorded(v.garrisonMissing ?? 'Garrison data not recorded')
        : liveText('requiredManpower', r => formatFixed(BigInt(r), 0, 0));
      let assignedManpower: Value<string | bigint | number>;
      let assignedFormat: 'integer' | undefined;
      if (!v.assignedKnown) {
        assignedManpower = notRecorded(v.garrisonMissing ?? 'Assigned manpower not recorded');
      } else {
        const rawAssigned = valueFromLive(live, liveKey(row.key, 'assignedManpower'));
        if (rawAssigned.kind === 'notRecorded' && rawAssigned.reason === NOT_NAMED) {
          assignedManpower = zeroValue(0);
          assignedFormat = 'integer';
        } else if (rawAssigned.kind === 'known' || rawAssigned.kind === 'zero') {
          assignedManpower = knownValue(formatFixed(toBigIntExact(rawAssigned.value), 0, 0));
        } else if (rawAssigned.kind === 'partial') {
          assignedManpower = partialValue(formatFixed(toBigIntExact(rawAssigned.value), 0, 0), rawAssigned.reason);
        } else {
          assignedManpower = rawAssigned;
        }
      }
      const strengthVal = valueFromLive(live, liveKey(row.key, STRENGTH));
      const strength: Value<string> = strengthVal.kind === 'known' || strengthVal.kind === 'zero'
        ? knownValue(`${formatAmount((toBigIntExact(strengthVal.value) * 100n).toString(), 5, 5)}%`)
        : notRecorded(v.garrisonMissing ?? (strengthVal.kind === 'notRecorded' && strengthVal.reason !== NOT_NAMED ? strengthVal.reason : reply.strengthMissing));
      const extra: TooltipExtraRow[] = [
        { label: 'Resistance speed', value: liveText('resistanceSpeed', r => `${formatSigned(r, 5, 5)} pp/day`), divider: true },
        { label: 'Compliance speed', value: liveText('complianceSpeed', r => `${formatSigned(r, 5, 5)} pp/day`) },
        { label: 'Required manpower', value: reqManpower },
        { label: 'Assigned manpower', value: assignedManpower, ...(assignedFormat ? { format: assignedFormat } : {}) },
        { label: 'Garrison strength', value: strength },
        { label: 'Occupied country', value: row.occupiedName ?? notRecorded('Unknown country') },
      ];
      if (v.garrisonAlert) extra.push({ label: 'Garrison status', value: knownValue('Garrison requirement not met.'), tone: 'error' });
      return extra;
    },
  }),
);

export const occupiedStatesDeclaration: TableWidgetDeclaration<OccupiedProjectionOptions> = {
  family: 'table',
  kind: 'occupied-states',
  label: 'Occupied states',
  subject: 'occupied states',
  emptyCampaign: 'Load a campaign to see occupied states.',
  className: 'occupied-states',
  frameClassName: 'occupied-table-surface',
  countries: { mode: 'filter', content: 'hour' },
  sources: ctx => ({
    owner: `occupied-states:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'all' },
        product: 'occupied-states',
        followQuery: { panel: 'occupied-states', branchId: ctx.branch.id },
        params: { branchId: ctx.branch.id },
        validate: validateOccupiedStates,
      },
    }],
  }),
  projection: occupiedStatesProjection,
  columns: OCCUPIED_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => ({
    filter: ctx.countries.filter,
    role: ctx.options.countryRole === 'occupied' ? 'occupied' : 'occupier',
    lawAs: ctx.settings.columns.law?.as ?? 'text',
    filterAll: ctx.countries.all,
  }),
  jumpToDate: true,
};
