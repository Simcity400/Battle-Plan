import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/plan.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { knownValue, notRecorded, textValue, zeroValue, type Value } from '../../families/value.ts';
import { formatDateOnly, formatScalar } from '../../families/valueFormat.ts';
import type { DivisionEquipmentReply, DivisionEquipmentView, EquipmentTemplate, EquipmentType, StockpileArtworkReply } from '../../generated/dashboard';
import { urlArtHandle } from '../../families/art/provider.ts';
import { artState, type ArtState } from '../../families/table/identityCell.ts';
import { TEMPLATE_BASE, TEMPLATE_COLUMNS, TEMPLATE_TABLE_KEY, TYPE_BASE, TYPE_COLUMNS, TYPE_TABLE_KEY } from './definition.ts';
import { validateDivisionEquipment } from './divisionEquipmentData.ts';

const NO_OPTIONS: Record<string, never> = {};
const FIXED = 100_000n;
const PERCENT = 'progress' as const;

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
  : `${formatScalar(reply.divisionsShort, 'integer')} ${reply.divisionsShort === 1 ? 'division' : 'divisions'} short`;

const partial = (reply: DivisionEquipmentReply): string | null => reply.unknownDivisions === null
  ? 'Divisions not recorded'
  : reply.unknownDivisions > 0 ? `${formatScalar(reply.unknownDivisions, 'integer')} ${reply.unknownDivisions === 1 ? 'division' : 'divisions'} not recorded` : null;

function bodyState(reply: DivisionEquipmentReply): { readonly empty?: string; readonly emptyKind?: 'empty' | 'unavailable' } | null {
  if (reply.missing !== null) return { empty: reply.missing, emptyKind: 'unavailable' };
  if (reply.divisions === 0) return { empty: 'No divisions at this date', emptyKind: 'empty' };
  return null;
}

const asOf = (reply: DivisionEquipmentReply) => reply.poolHour === null ? undefined : `Equipment as of ${formatDateOnly(reply.poolHour)}`;

const [PICTURE, TYPE, HELD, NEEDED, MISSING, TYPE_FILL, DIVISIONS_SHORT] = TYPE_COLUMNS;

type TypeRow = EquipmentType & { readonly picture: ArtState };

export function validateTypePictures(value: unknown): StockpileArtworkReply {
  const reply = value as StockpileArtworkReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.entries) || !Array.isArray(reply.warnings)
    || !reply.entries.every(entry => entry && typeof entry.key === 'string' && (entry.icon === null || typeof entry.icon === 'string')))
    throw new Error('The equipment pictures reply is malformed.');
  return reply;
}

const TYPE_PREFIX = 'type:';
const pictureCache = new WeakMap<StockpileArtworkReply, ReadonlyMap<string, string>>();
function typePictures(reply: StockpileArtworkReply | undefined): ReadonlyMap<string, string> {
  if (!reply) return new Map();
  let held = pictureCache.get(reply);
  if (!held) {
    const map = new Map<string, string>();
    for (const entry of reply.entries) if (entry.icon && entry.key.startsWith(TYPE_PREFIX)) map.set(entry.key.slice(TYPE_PREFIX.length).toLowerCase(), entry.icon);
    pictureCache.set(reply, held = map);
  }
  return held;
}

function typePicture(type: EquipmentType, pictures: ReadonlyMap<string, string>, readView: BoundedReadView): ArtState {
  return type.archetype === null ? {} : artState(readView, urlArtHandle(pictures.get(type.archetype.toLowerCase())));
}

function typeName(row: EquipmentType): Value<string> {
  return row.unclassified ? knownValue('Unclassified equipment') : textValue(row.name, row.nameMissing ?? 'Equipment name unavailable');
}

const TYPE_SPEC: readonly DeclarativeColumnSpec<TypeRow, Record<string, never>>[] = [
  { ...PICTURE!, priority: 6, sortable: false, cellKind: 'kind', iconSlot: true, icon: row => row.picture.icon, iconPending: row => row.picture.iconPending === true,
    extra: row => ({ artShape: 'equipment', ...(row.name ? { artName: row.name } : {}) }),
    value: typeName },
  { ...TYPE!, priority: 0, text: true, cellKind: 'item', hideable: false, widest: ['Infantry Equipment'], value: typeName },
  { ...HELD!, priority: 3, numeric: true, format: 'integer', widest: ['999,999'], value: row => amount(row.heldRaw, 'Holdings not recorded') },
  { ...NEEDED!, priority: 2, numeric: true, format: 'integer', widest: ['999,999'], value: row => amount(row.neededRaw, 'Need not recorded') },
  { ...MISSING!, priority: 1, numeric: true, format: 'integer', widest: ['−999,999'], value: row => missingValue(row.missingRaw),
    extra: row => (row.missingRaw !== null && BigInt(row.missingRaw) / FIXED > 0n ? { shortage: true } : {}),
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

export const equipmentTypeProjection: TableProjection<Record<string, never>> = registerTableProjection(declareTableProjection<DivisionEquipmentReply, TypeRow, Record<string, never>>({
    kind: 'division-equipment',
    validate: validateDivisionEquipment,
    columns: TYPE_SPEC,
    rows: (reply, _live, _hour, _options, readView) => {
      const blocked = bodyState(reply);
      if (blocked) return { rows: [], ...blocked, warnings: reply.warnings };
      const pictures = typePictures(readView.panel<StockpileArtworkReply>('pictures'));
      const rows = reply.types.map(type => ({ ...type, picture: typePicture(type, pictures, readView) }));
      return { rows, titleSummary: summary(reply), titleSummaryPartial: partial(reply), warnings: reply.warnings };
    },
    rowKey: row => row.key,
    rowName: row => row.name ?? 'Equipment name unavailable',
    tooltipExtra: row => typeTooltip(row),
    tooltipNote: (_row, reply) => asOf(reply),
  }), 'equipment');

interface TemplateRow { readonly template: EquipmentTemplate; readonly icon: string | undefined; readonly pending: boolean }

function templateIcon(template: EquipmentTemplate, readView: BoundedReadView): Pick<TemplateRow, 'icon' | 'pending'> {
  if (template.icon === null) return { icon: undefined, pending: false };
  const art = readView.art({ kind: 'url', url: template.icon });
  return art.status === 'ready' ? { icon: art.url, pending: false } : { icon: undefined, pending: art.status === 'pending' };
}

const [KIND, TEMPLATE, DIVISIONS, MANPOWER, STRENGTH, FILL, TYPES_SHORT] = TEMPLATE_COLUMNS;

const TEMPLATE_SPEC: readonly DeclarativeColumnSpec<TemplateRow, Record<string, never>>[] = [
  { ...KIND!, priority: 6, sortable: false, cellKind: 'kind', iconSlot: true, icon: row => row.icon, iconPending: row => row.pending,
    extra: () => ({ artShape: 'counter' }), value: row => textValue(row.template.name, row.template.nameMissing ?? 'Template name unavailable') },
  { ...TEMPLATE!, priority: 0, text: true, cellKind: 'item', hideable: false, widest: ['Infantry Division 1941'],
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

export const equipmentTemplateProjection: TableProjection<Record<string, never>> = registerTableProjection(declareTableProjection<DivisionEquipmentReply, TemplateRow, Record<string, never>>({
    kind: 'division-equipment',
    validate: validateDivisionEquipment,
    columns: TEMPLATE_SPEC,
    rows: (reply, _live, _hour, _options, readView) => {
      const blocked = bodyState(reply);
      if (blocked) return { rows: [], ...blocked, warnings: reply.warnings };
      return {
        rows: reply.templates.map(template => ({ template, ...templateIcon(template, readView) })),
        titleSummary: summary(reply),
        titleSummaryPartial: partial(reply),
        warnings: reply.warnings,
      };
    },
    rowKey: row => row.template.template,
    rowName: row => row.template.name ?? 'Template name unavailable',
    tooltipExtra: row => row.template.unknownDivisions === null || row.template.unknownDivisions > 0
      ? [{ label: 'Divisions not recorded', value: count(row.template.unknownDivisions, 'Divisions not recorded'), divider: true }]
      : undefined,
    tooltipNote: (_row, reply) => asOf(reply),
  }), 'template');

function sources(ctx: FamilyPlanContext, view: DivisionEquipmentView): SourcePlan {
  const params = ctx.countryTag
    ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag), countFor: ctx.options.countFor === 'host' ? 'host' : 'owner', view }
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
        }, ...(view === 'types' ? [{
          name: 'pictures',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'point',
            channel: 'panel',
            coverage: { mode: 'keyed', keys: [ctx.countryTag!] },
            product: 'stockpile-artwork',
            followQuery: { panel: 'stockpile-artwork', branchId: ctx.branch.id, country: params.country, variants: [] },
            params: { branchId: ctx.branch.id, country: params.country, variants: [] },
            validate: validateTypePictures,
          },
        } as const] : [])]
      : [],
  };
}

export const equipmentTypeDeclaration: TableWidgetDeclaration<Record<string, never>> = {
  family: 'table',
  kind: 'division-equipment',
  label: 'Division equipment by equipment',
  subject: 'division equipment',
  emptyCampaign: 'Load a campaign to see division equipment.',
  icon: 'cube',
  countries: { mode: 'single', choose: 'Choose a country to see its division equipment.' },
  sources: ctx => sources(ctx, 'types'),
  projection: equipmentTypeProjection,
  columns: TYPE_SPEC,
  settings: { kind: 'table', key: TYPE_TABLE_KEY, base: TYPE_BASE },
  projectionOptions: () => NO_OPTIONS,
  jumpToDate: true,
};

export const equipmentTemplateDeclaration: TableWidgetDeclaration<Record<string, never>> = {
  ...equipmentTypeDeclaration,
  label: 'Division equipment by template',
  sources: ctx => sources(ctx, 'templates'),
  projection: equipmentTemplateProjection,
  columns: TEMPLATE_SPEC,
  settings: { kind: 'table', key: TEMPLATE_TABLE_KEY, base: TEMPLATE_BASE },
};
