import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { knownValue, textValue, notApplicable, notRecorded, zeroValue, type Value } from '../../families/value.ts';
import { formatDateOnly, formatScalar } from '../../families/valueFormat.ts';
import type { TrainingDivision, TrainingOrder, TrainingQueueReply } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';

const NO_OPTIONS: Record<string, never> = {};
const ON_DIVISION = 'Shown on each division';
const PERCENT = 'progress' as const;
const FIXED = 100_000n;

const isArray = Array.isArray;
export function validateTrainingQueue(value: unknown): TrainingQueueReply {
  const reply = value as TrainingQueueReply;
  if (!reply || typeof reply.tag !== 'string' || !(reply.missing === null || typeof reply.missing === 'string')
    || !isArray(reply.orders) || !isArray(reply.warnings) || !(reply.inTraining === null || Number.isSafeInteger(reply.inTraining))
    || (reply.readHour !== null && !Number.isInteger(reply.readHour))
    || reply.orders.some(order => typeof order.order !== 'string' || !Number.isInteger(order.amount) || !isArray(order.divisions)
      || !Number.isSafeInteger(order.manpower) || !Number.isSafeInteger(order.manpowerNeeded)
      || order.divisions.some(division => typeof division.trainingRaw !== 'string' || typeof division.maxTrainingRaw !== 'string'
        || !(division.equipmentFillRaw === null || typeof division.equipmentFillRaw === 'string') || !isArray(division.need) || !isArray(division.equipment))))
    throw new Error('Invalid training queue reply');
  return reply;
}

type QueueRow =
  | { readonly kind: 'order'; readonly key: string; readonly order: TrainingOrder; readonly icon: string | undefined; readonly open: boolean; readonly children: readonly QueueRow[] }
  | { readonly kind: 'division'; readonly key: string; readonly division: TrainingDivision };

function share(part: bigint, whole: bigint): Value<number> {
  if (whole <= 0n) return notRecorded('Maximum not recorded');
  if (part <= 0n) return zeroValue(0);
  return knownValue(Number((part > whole ? whole : part) * 1_000_000n / whole) / 10_000);
}

export function trainingShare(division: TrainingDivision): Value<number> {
  const maximum = BigInt(division.maxTrainingRaw);
  return maximum > 0n ? share(BigInt(division.trainingRaw), maximum) : notRecorded('Maximum training not recorded');
}

export function equipmentShare(division: TrainingDivision): Value<number> {
  if (division.equipmentFillRaw !== null) return share(BigInt(division.equipmentFillRaw), FIXED);
  return notRecorded(division.comparisonMissing ?? 'The template needs no equipment that can be measured');
}

const fraction = (value: Value<number>): Value<number> => value.kind === 'known' ? knownValue(value.value / 100) : value;
const count = (value: number) => formatScalar(value, 'integer');
const manpower = (have: number): Value<number> => have === 0 ? zeroValue(0) : knownValue(have);

function orderLine(order: TrainingOrder): string {
  const parts = [`${count(order.divisions.length)} ${order.divisions.length === 1 ? 'division' : 'divisions'}`];
  if (order.locationName?.trim()) parts.push(order.locationName);
  if (order.priorityName?.trim()) parts.push(order.priorityName);
  return parts.join(' · ');
}

const [DIVISION, TRAINING, EQUIPMENT, MANPOWER] = COLUMNS;

const QUEUE_COLUMNS: readonly DeclarativeColumnSpec<QueueRow, Record<string, never>>[] = [
  { ...DIVISION!, priority: 0, text: true, cellKind: 'item', hideable: false, widest: ['Motorized Infantry Division 1941 B'],
    icon: row => row.kind === 'order' ? row.icon : undefined,
    extra: row => (row.kind === 'order' ? { iconSlot: true, subtext: orderLine(row.order) } : {}),
    value: row => row.kind === 'order'
      ? textValue(row.order.templateName, row.order.templateMissing ?? 'Template name unavailable')
      : textValue(row.division.name, row.division.nameMissing ?? 'Division name unavailable') },
  { ...TRAINING!, priority: 1, numeric: true, format: PERCENT, widest: ['100%'],
    value: row => row.kind === 'division' ? trainingShare(row.division) : notApplicable(ON_DIVISION),
    extra: row => (row.kind === 'order' ? { blank: true } : {}),
    bar: row => row.kind === 'division' ? { fraction: fraction(trainingShare(row.division)) } : null },
  { ...EQUIPMENT!, priority: 2, numeric: true, format: PERCENT, widest: ['100%'],
    value: row => row.kind === 'division' ? equipmentShare(row.division) : notApplicable(ON_DIVISION),
    extra: row => (row.kind === 'order' ? { blank: true } : {}),
    bar: row => row.kind === 'division' ? { fraction: fraction(equipmentShare(row.division)) } : null },
  { ...MANPOWER!, priority: 3, numeric: true, cellKind: 'outOf', format: 'integer', widest: ['999,999 / 999,999'],
    value: row => manpower(row.kind === 'order' ? row.order.manpower : row.division.manpower),
    extra: row => ({ secondaryValue: knownValue(count(row.kind === 'order' ? row.order.manpowerNeeded : row.division.manpowerNeeded)) }) },
];

function orderIcon(order: TrainingOrder, readView: BoundedReadView): string | undefined {
  if (order.icon === null) return undefined;
  const art = readView.art({ kind: 'url', url: order.icon });
  return art.status === 'ready' ? art.url : undefined;
}

function orderTooltip(order: TrainingOrder): TooltipExtraRow[] {
  return [
    { label: 'Divisions in training', value: knownValue(count(order.divisions.length)), divider: true },
    { label: 'Ordered', value: knownValue(count(order.amount)) },
    { label: 'Location', value: textValue(order.locationName, order.locationMissing ?? 'Location not recorded') },
    { label: 'Equipment priority', value: textValue(order.priorityName, order.priorityMissing ?? 'Priority not recorded') },
    ...(order.exile ? [{ label: 'Government in exile', value: knownValue(order.exile) }] : []),
    ...(order.closed ? [{ label: 'Closed', value: knownValue('Yes') }] : []),
  ];
}

function divisionTooltip(division: TrainingDivision): TooltipExtraRow[] {
  const rows: TooltipExtraRow[] = [];
  if (division.needMissing !== null) rows.push({ label: 'Equipment', value: notRecorded(division.needMissing), divider: true });
  else {
    division.need.forEach((need, at) => rows.push({
      label: need.name ?? 'Equipment name unavailable',
      value: knownValue(`${count(Number(BigInt(need.heldRaw) / FIXED))} / ${count(Number(BigInt(need.needRaw) / FIXED))}`),
      lookupKey: need.archetype,
      ...(at === 0 ? { divider: true } : {}),
    }));
  }
  if (division.halted) rows.push({ label: 'Halted', value: notRecorded('Halting reason not recorded') });
  return rows;
}

export const trainingQueueProjection: TableProjection<Record<string, never>> = registerTableProjection(
  declareTableProjection<TrainingQueueReply, QueueRow, Record<string, never>>({
    kind: 'training-queue',
    validate: validateTrainingQueue,
    columns: QUEUE_COLUMNS,
    rows: (reply, _live, _hour, _options, readView) => {
      if (reply.missing !== null) return { rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      const rows: QueueRow[] = reply.orders.map((order, at) => ({
        kind: 'order',
        key: `order:${order.order}`,
        order,
        icon: orderIcon(order, readView),
        open: at === 0,
        children: order.divisions.map(division => ({ kind: 'division', key: `line:${division.line}`, division })),
      }));
      return {
        rows,
        ...(!rows.length ? { empty: 'No divisions in training', emptyKind: 'empty' as const } : {}),
        titleSummary: reply.inTraining === null ? null : `${count(reply.inTraining)} in training`,
        warnings: reply.warnings,
      };
    },
    rowKey: row => row.key,
    rowName: row => row.kind === 'order' ? (row.order.templateName ?? 'Template name unavailable') : (row.division.name ?? 'Division name unavailable'),
    children: row => row.kind === 'order' ? row.children : undefined,
    initiallyOpen: row => row.kind === 'order' && row.open,
    tooltipExtra: row => row.kind === 'order' ? orderTooltip(row.order) : divisionTooltip(row.division),
    tooltipNote: (_row, reply) => reply.readHour === null ? undefined : `As of ${formatDateOnly(reply.readHour)}`,
  }),
);

export const trainingQueueDeclaration: TableWidgetDeclaration<Record<string, never>> = {
  family: 'table',
  kind: 'training-queue',
  label: 'Training queue',
  subject: 'training queue',
  emptyCampaign: 'Load a campaign to see the training queue.',
  icon: 'person',
  countries: { mode: 'single', choose: 'Choose a country to see its training queue.' },
  sources: ctx => {
    const params = ctx.countryTag ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) } : null;
    return {
      owner: `training-queue:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
      entries: params
        ? [{
            name: 'main',
            source: ctx.track.source,
            branch: ctx.branch.id,
            offset: ctx.track.axisOffset,
            read: {
              shape: 'point',
              channel: 'panel',
              coverage: { mode: 'keyed', keys: [ctx.countryTag!] },
              product: 'training-queue',
              followQuery: { panel: 'training-queue', ...params },
              params,
              validate: validateTrainingQueue,
            },
          }]
        : [],
    };
  },
  projection: trainingQueueProjection,
  columns: QUEUE_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: () => NO_OPTIONS,
  jumpToDate: true,
};
