import { formatAmount, formatFixed, formatSigned } from '../../charts/valueAxis.ts';
import { liveKey } from '../../charts/live.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { DeclaredCountryFilter, HourRanges, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { lookAt, type CountryLooksMap } from '../../families/table/looks.ts';
import {
  declareTableProjection,
  type DeclarativeColumnSpec,
  type TableCellBar,
  type TableColumnSpec,
  type TableProjection,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { strengthBar, TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, partialValue, valueFromLive, zeroValue, type Value } from '../../families/value.ts';
import { COLUMNS, OPTIONS, TABLE_BASE } from './definition.ts';
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

export type { HourRanges };
export type OccupiedFilter = DeclaredCountryFilter;

export interface OccupiedProjectionOptions {
  readonly filter: OccupiedFilter;
  readonly role: CountryRole;
  readonly labels: CountryLooksMap;
  readonly lawAs: string;
  readonly filterAll: boolean;
}

const PERCENT_SCALE = 10_000_000;
const STATUS_MAP: Readonly<Record<string, 'dot' | 'outline' | 'fill'>> = { dot: 'dot', outline: 'outline', filled: 'fill' };
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

function strengthFraction(raw: number): number {
  if (Math.abs(raw) >= 2 ** 40) return Math.max(0, Math.min(1, Number(strengthBar(BigInt(Math.trunc(raw)))) / 100000));
  const percent = Math.trunc(raw) * 100;
  const rounded = Math.trunc((percent + (percent < 0 ? -50000 : 50000)) / 100000) * 1000;
  return Math.max(0, Math.min(1, rounded / 100000));
}

function countryLabel(tag: string | null, labels: CountryLooksMap, hour: number): Value<string> {
  if (tag === null) return notRecorded('Unknown country');
  const look = lookAt(labels[tag], hour);
  return look && look.name ? knownValue(look.name) : notRecorded(look?.status ?? 'Country name unavailable');
}

export const OCCUPIED_COLUMNS: readonly TableColumnSpec[] = COLUMNS.map(col => ({
  ...col,
  priority: PRIORITIES[col.key] ?? 9,
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

export const occupiedStatesProjection: TableProjection<OccupiedProjectionOptions> = registerTableProjection(
  declareTableProjection<OccupiedStatesReply, OccupiedRow, OccupiedProjectionOptions>({
    kind: 'occupied-states',
    validate: validateOccupiedStates,
    nextChange: (_sources, hour, options) => filterNext(options.filter, hour),
    columns: (reply, options): readonly DeclarativeColumnSpec<OccupiedRow, OccupiedProjectionOptions>[] => {
      const laws = lawsOf(reply);
      const redBelow = reply.garrisonRedRaw === null ? undefined : Number(reply.garrisonRedRaw);
      return [
        { ...OCCUPIED_COLUMNS[0]!, path: 'value.name', whenNull: { kind: 'notRecorded', reasonField: 'value.nameMissing', reason: 'Not recorded' } },
        { ...OCCUPIED_COLUMNS[1]!, countryField: 'value.owner' },
        { ...OCCUPIED_COLUMNS[2]!, countryField: 'value.occupier' },
        { ...OCCUPIED_COLUMNS[3]!, liveField: 'resistance', maximum: PERCENT_SCALE, country: 'value.occupier' },
        { ...OCCUPIED_COLUMNS[4]!, liveField: 'compliance', maximum: PERCENT_SCALE, country: 'value.occupier' },
        {
          ...OCCUPIED_COLUMNS[5]!,
          country: 'value.occupier',
          value: (row, live) => {
            if (row.value.garrisonRequiredSign === null) {
              return row.value.garrisonMissing !== null ? notRecorded(row.value.garrisonMissing) : notApplicable('No garrison requirement');
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
              const rawNum = Number(strengthVal.value);
              const frac = strengthFraction(rawNum);
              return { fraction: frac === 0 ? zeroValue(0) : knownValue(frac), alert: redBelow !== undefined && rawNum < redBelow };
            }
            return { fraction: notRecorded(strengthVal.kind === 'notRecorded' ? strengthVal.reason : reply.strengthMissing) };
          },
        },
        {
          ...OCCUPIED_COLUMNS[6]!,
          iconSlot: options.lawAs === 'icon',
          icon: row => (row.value.law !== null ? laws.get(row.value.law)?.icon ?? undefined : undefined),
          status: () => STATUS_MAP[options.lawAs],
          value: row => {
            if (row.value.law === null) return notRecorded(row.value.lawMissing ?? 'Not recorded');
            const law = laws.get(row.value.law);
            return knownValue(contentTitle(law?.name, row.value.law, 'Occupation law name unavailable'));
          },
        },
      ];
    },
    rows: (reply, _live, hour, options) => {
      const passing = options.filter.passing;
      const unrecorded = options.filter.unrecorded.find(([first, last]) => first <= hour && hour <= last)?.[2] ?? null;
      const view = occupiedRows(reply, passing ? tag => within(passing[tag], hour) : null, options.role, unrecorded);
      const empty = emptyReason(reply, view);
      if (reply.missing !== null) {
        return { rows: [], empty: empty ?? reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      }
      const notes: string[] = [];
      if (!reply.complete) notes.push('Some occupied states are missing.');
      if (!options.filterAll && reply.states.some(state => state[options.role] === null)) {
        notes.push('Some states couldn’t be matched to these countries.');
      }
      return {
        rows: view.rows,
        ...(empty !== null ? { empty, emptyKind: view.membership || view.selectionUnknown || !reply.complete ? 'unavailable' : 'empty' } : {}),
        notes,
        warnings: reply.warnings,
      };
    },
    rowKey: row => row.key,
    rowName: row => row.value.name ?? `State ${row.state}`,
    tooltipExtra: (row, reply, live, hour, options): TooltipExtraRow[] => {
      const v = row.value;
      const liveText = (column: string, fmt: (s: string) => string): Value<string> => {
        const val = valueFromLive(live, liveKey(row.key, column));
        if (val.kind === 'known' || val.kind === 'zero') return knownValue(fmt(BigInt(Math.trunc(Number(val.value))).toString()));
        if (val.kind === 'partial') return partialValue(fmt(BigInt(Math.trunc(Number(val.value))).toString()), val.reason);
        return val;
      };
      const reqManpower: Value<string> = v.garrisonRequiredSign === null
        ? notApplicable('No garrison requirement')
        : liveText('requiredManpower', r => formatFixed(BigInt(r), 0, 0));
      let assignedManpower: Value<string>;
      if (!v.assignedKnown) {
        assignedManpower = notApplicable('Assigned manpower does not apply');
      } else {
        const rawAssigned = valueFromLive(live, liveKey(row.key, 'assignedManpower'));
        if (rawAssigned.kind === 'notRecorded' && rawAssigned.reason === NOT_NAMED) assignedManpower = knownValue('0');
        else if (rawAssigned.kind === 'known' || rawAssigned.kind === 'zero') assignedManpower = knownValue(formatFixed(BigInt(Math.trunc(Number(rawAssigned.value))), 0, 0));
        else if (rawAssigned.kind === 'partial') assignedManpower = partialValue(formatFixed(BigInt(Math.trunc(Number(rawAssigned.value))), 0, 0), rawAssigned.reason);
        else assignedManpower = rawAssigned;
      }
      const strengthVal = valueFromLive(live, liveKey(row.key, STRENGTH));
      const strength: Value<string> = strengthVal.kind === 'known' || strengthVal.kind === 'zero'
        ? knownValue(`${formatAmount((BigInt(Math.trunc(Number(strengthVal.value))) * 100n).toString(), 5, 5)}%`)
        : notRecorded(v.garrisonMissing ?? (strengthVal.kind === 'notRecorded' && strengthVal.reason !== NOT_NAMED ? strengthVal.reason : reply.strengthMissing));
      const extra: TooltipExtraRow[] = [
        { label: 'Resistance speed', value: liveText('resistanceSpeed', r => `${formatSigned(r, 5, 5)} pp/day`), divider: true },
        { label: 'Compliance speed', value: liveText('complianceSpeed', r => `${formatSigned(r, 5, 5)} pp/day`) },
        { label: 'Required manpower', value: reqManpower },
        { label: 'Assigned manpower', value: assignedManpower },
        { label: 'Garrison strength', value: strength },
        { label: 'Occupied country', value: countryLabel(v.occupied, options.labels, hour) },
      ];
      if (v.garrisonAlert) extra.push({ label: 'Garrison status', value: knownValue('Garrison requirement not met.'), tone: 'negative' });
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
  optionFields: OPTIONS,
  countries: { mode: 'filter', labels: true, colours: true },
  source: {
    kind: 'panel',
    product: 'occupied-states',
    params: ctx => ({ branchId: ctx.branch.id }),
    validate: validateOccupiedStates,
  },
  projection: occupiedStatesProjection,
  columns: OCCUPIED_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => ({
    filter: ctx.countries.filter,
    role: ctx.options.countryRole === 'occupied' ? 'occupied' : 'occupier',
    labels: ctx.countries.labels,
    lawAs: ctx.settings.columns.law?.as ?? 'dot',
    filterAll: ctx.countries.all,
  }),
  jumpToDate: true,
};
