import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { DeclaredTableContext, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/plan.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { knownValue, notRecorded, textValue, zeroValue, type Value } from '../../families/value.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { DivisionEquipmentReply, EquipmentTemplate, EquipmentType } from '../../generated/dashboard';
import { TEMPLATE_BASE, TEMPLATE_COLUMNS, TEMPLATE_TABLE_KEY, TYPE_BASE, TYPE_COLUMNS, TYPE_TABLE_KEY } from './definition.ts';
import { validateDivisionEquipment } from './divisionEquipmentData.ts';

const NO_OPTIONS: Record<string, never> = {};
const FIXED = 100_000n;
const PERCENT = { decimals: 0, percent: true, maxDecimals: 0 } as const;

const items = (raw: string): Value<number> => {
  const whole = BigInt(raw) / FIXED;
  return whole === 0n ? zeroValue(0) : knownValue(Number(whole));
};

const amount = (raw: string | null, reason: string): Value<number> => raw === null ? notRecorded(reason) : items(raw);

const count = (value: number | null, reason: string): Value<number> =>
  value === null ? notRecorded(reason) : value === 0 ? zeroValue(0) : knownValue(value);

export function share(part: string | null, whole: string | null, reason: string): Value<number> {
  if (part === null || whole === null) return notRecorded(reason);
  const top = BigInt(whole);
  if (top <= 0n) return notRecorded(reason);
  const have = BigInt(part);
  if (have <= 0n) return zeroValue(0);
  return knownValue(Number((have > top ? top : have) * 1_000_000n / top) / 10_000);
}

const fraction = (value: Value<number>): Value<number> => value.kind === 'known' ? knownValue(value.value / 100) : value;

export function missingValue(raw: string | null): Value<number> {
  if (raw === null) return notRecorded('Shortfall not recorded');
  const whole = BigInt(raw) / FIXED;
  return whole === 0n ? zeroValue(0) : knownValue(-Number(whole));
}

const summary = (reply: DivisionEquipmentReply): string | null => reply.divisionsShort === null
  ? null
  : `${reply.divisionsShort.toLocaleString('en-US')} ${reply.divisionsShort === 1 ? 'division' : 'divisions'} short`;

function bodyState(reply: DivisionEquipmentReply): { readonly empty?: string; readonly emptyKind?: 'empty' | 'unavailable' } | null {
  if (reply.missing !== null) return { empty: reply.missing, emptyKind: 'unavailable' };
  if (reply.divisions === 0) return { empty: 'No divisions at this date', emptyKind: 'empty' };
  return null;
}

const asOf = (reply: DivisionEquipmentReply) => reply.poolHour === null ? undefined : `Equipment as of ${formatDateOnly(reply.poolHour)}`;

const [TYPE, HELD, NEEDED, MISSING, TYPE_FILL, DIVISIONS_SHORT] = TYPE_COLUMNS;

const TYPE_SPEC: readonly DeclarativeColumnSpec<EquipmentType, Record<string, never>>[] = [
  { ...TYPE!, priority: 0, text: true, hideable: false, widest: ['Infantry Equipment'],
    value: row => row.archetype === null ? knownValue('Unclassified equipment') : textValue(row.name, row.nameMissing ?? 'Equipment name unavailable') },
  { ...HELD!, priority: 3, numeric: true, format: 'integer', widest: ['999,999'], value: row => amount(row.heldRaw, 'Holdings not recorded') },
  { ...NEEDED!, priority: 2, numeric: true, format: 'integer', widest: ['999,999'], value: row => amount(row.neededRaw, 'Need not recorded') },
  { ...MISSING!, priority: 1, numeric: true, format: 'integer', widest: ['−999,999'], value: row => missingValue(row.missingRaw),
    cellTone: row => row.missingRaw !== null && BigInt(row.missingRaw) / FIXED > 0n ? 'red' : undefined,
    sortValue: row => row.missingRaw === null ? notRecorded('Shortfall not recorded') : knownValue(BigInt(row.missingRaw)) },
  { ...TYPE_FILL!, priority: 4, numeric: true, format: PERCENT, widest: ['100%'],
    value: row => share(row.filledRaw, row.neededRaw, 'Fill not recorded'),
    bar: row => ({ fraction: fraction(share(row.filledRaw, row.neededRaw, 'Fill not recorded')) }) },
  { ...DIVISIONS_SHORT!, priority: 5, numeric: true, format: 'integer', widest: ['9,999'], value: row => count(row.divisionsShort, 'Divisions short not recorded') },
];

function typeTooltip(row: EquipmentType): TooltipExtraRow[] {
  const rows: TooltipExtraRow[] = [];
  if (row.unknownDivisions === null || row.unknownDivisions > 0) {
    rows.push({ label: 'Divisions not recorded', value: count(row.unknownDivisions, 'Divisions not recorded'), divider: true });
  }
  row.variants.forEach((variant, at) => rows.push({
    label: variant.name ?? 'Design name unavailable', value: amount(variant.heldRaw, 'Holdings not recorded'), format: 'integer', lookupKey: variant.variant,
    indent: true, ...(at === 0 ? { divider: true } : {}),
  }));
  if (row.stockpileRaw !== null) {
    rows.push({ label: row.stockpileHour === null ? 'In stockpile' : `In stockpile (${formatDateOnly(row.stockpileHour)})`, value: items(row.stockpileRaw), format: 'integer', divider: true });
  }
  return rows;
}

export const equipmentTypeProjection: TableProjection<Record<string, never>> = registerTableProjection(
  declareTableProjection<DivisionEquipmentReply, EquipmentType, Record<string, never>>({
    kind: 'division-equipment',
    validate: validateDivisionEquipment,
    columns: TYPE_SPEC,
    rows: reply => {
      const blocked = bodyState(reply);
      if (blocked) return { rows: [], ...blocked, warnings: reply.warnings };
      return { rows: reply.types, summaryPill: summary(reply), warnings: reply.warnings };
    },
    rowKey: row => JSON.stringify([row.archetype]),
    rowName: row => row.name ?? 'Equipment name unavailable',
    tooltipExtra: row => typeTooltip(row),
    tooltipNote: (_row, reply) => asOf(reply),
  }),
  'equipment',
);

interface TemplateRow { readonly template: EquipmentTemplate; readonly icon: string | undefined; readonly pending: boolean }

function templateIcon(template: EquipmentTemplate, readView: BoundedReadView): Pick<TemplateRow, 'icon' | 'pending'> {
  if (template.icon === null) return { icon: undefined, pending: false };
  const art = readView.art({ kind: 'url', url: template.icon });
  return art.status === 'ready' ? { icon: art.url, pending: false } : { icon: undefined, pending: art.status === 'pending' };
}

const [TEMPLATE, DIVISIONS, MANPOWER, STRENGTH, FILL, TYPES_SHORT] = TEMPLATE_COLUMNS;

const TEMPLATE_SPEC: readonly DeclarativeColumnSpec<TemplateRow, Record<string, never>>[] = [
  { ...TEMPLATE!, priority: 0, text: true, hideable: false, widest: ['Infantry Division 1941'], iconSlot: true,
    icon: row => row.icon, iconPending: row => row.pending,
    value: row => textValue(row.template.name, row.template.nameMissing ?? 'Template name unavailable') },
  { ...DIVISIONS!, priority: 1, numeric: true, format: 'integer', widest: ['999'], value: row => count(row.template.divisions, 'Divisions not recorded') },
  { ...MANPOWER!, priority: 3, numeric: true, format: PERCENT, widest: ['100%'],
    value: row => share(row.template.manpower, row.template.manpowerRequired, 'Manpower not recorded'),
    bar: row => ({ fraction: fraction(share(row.template.manpower, row.template.manpowerRequired, 'Manpower not recorded')) }) },
  { ...STRENGTH!, priority: 4, numeric: true, format: PERCENT, widest: ['100%'],
    value: row => share(row.template.hpRaw, row.template.hpMaxRaw, 'Strength not recorded'),
    bar: row => ({ fraction: fraction(share(row.template.hpRaw, row.template.hpMaxRaw, 'Strength not recorded')) }) },
  { ...FILL!, priority: 2, numeric: true, format: PERCENT, widest: ['100%'],
    value: row => share(row.template.heldRaw, row.template.neededRaw, 'Equipment fill not recorded'),
    bar: row => ({ fraction: fraction(share(row.template.heldRaw, row.template.neededRaw, 'Equipment fill not recorded')) }) },
  { ...TYPES_SHORT!, priority: 5, numeric: true, format: 'integer', widest: ['99'], value: row => count(row.template.typesShort, 'Types short not recorded') },
];

export const equipmentTemplateProjection: TableProjection<Record<string, never>> = registerTableProjection(
  declareTableProjection<DivisionEquipmentReply, TemplateRow, Record<string, never>>({
    kind: 'division-equipment',
    validate: validateDivisionEquipment,
    columns: TEMPLATE_SPEC,
    rows: (reply, _live, _hour, _options, readView) => {
      const blocked = bodyState(reply);
      if (blocked) return { rows: [], ...blocked, warnings: reply.warnings };
      return {
        rows: reply.templates.map(template => ({ template, ...templateIcon(template, readView) })),
        summaryPill: summary(reply),
        warnings: reply.warnings,
      };
    },
    rowKey: row => row.template.template,
    rowName: row => row.template.name ?? 'Template name unavailable',
    tooltipExtra: row => row.template.unknownDivisions === null || row.template.unknownDivisions > 0
      ? [{ label: 'Divisions not recorded', value: count(row.template.unknownDivisions, 'Divisions not recorded'), divider: true }]
      : undefined,
    tooltipNote: (_row, reply) => asOf(reply),
  }),
  'template',
);

function sources(ctx: FamilyPlanContext, view: string): SourcePlan {
  const params = ctx.countryTag
    ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag), countFor: ctx.options.countFor === 'host' ? 'host' : 'owner', template: null, division: null }
    : null;
  return {
    owner: `division-equipment:${view}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}:${params?.countFor ?? ''}`,
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
            product: 'division-equipment',
            followQuery: { panel: 'division-equipment', ...params },
            params,
            validate: validateDivisionEquipment,
          },
        }]
      : [],
  };
}

const noOptions = (_ctx: DeclaredTableContext) => NO_OPTIONS;

export const equipmentTypeDeclaration: TableWidgetDeclaration<Record<string, never>> = {
  family: 'table',
  kind: 'division-equipment',
  label: 'Division equipment by type',
  subject: 'division equipment',
  emptyCampaign: 'Load a campaign to see division equipment.',
  icon: 'helmet',
  countries: { mode: 'single', choose: 'Choose a country to see its division equipment.' },
  sources: ctx => sources(ctx, 'equipment'),
  projection: equipmentTypeProjection,
  columns: TYPE_SPEC,
  settings: { kind: 'table', key: TYPE_TABLE_KEY, base: TYPE_BASE },
  projectionOptions: noOptions,
  jumpToDate: true,
};

export const equipmentTemplateDeclaration: TableWidgetDeclaration<Record<string, never>> = {
  ...equipmentTypeDeclaration,
  label: 'Division equipment by template',
  sources: ctx => sources(ctx, 'template'),
  projection: equipmentTemplateProjection,
  columns: TEMPLATE_SPEC,
  settings: { kind: 'table', key: TEMPLATE_TABLE_KEY, base: TEMPLATE_BASE },
};
