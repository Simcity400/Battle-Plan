import { countryFromKey } from '../../dashboard/countryKey.ts';
import { unresolvedTypeName } from '../../dashboard/equipment.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/plan.ts';
import { urlArtHandle } from '../../families/art/provider.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artKindCell, resolveIdentity } from '../../families/table/identityCell.ts';
import type { TableCellStockpile, TableColumnSpec, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { OFFERED, TABLE_SETTINGS_KEY, type TableBase } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, zeroValue, type Value } from '../../families/value.ts';
import { formatScalar, formatShare, type ValueFormat } from '../../families/valueFormat.ts';
import type { StockFigure, StockTypeRow, StockVariantRow, StockpileArtworkReply, StockpileTypesReply } from '../../generated/dashboard';
import { validateStockpileArtwork } from './stockpileArtwork.ts';

export type StockpileLayout = 'sum' | 'parts';

export interface StockpileProjectionOptions {
  readonly layout: StockpileLayout;
}

const STORED: ValueFormat = { decimals: 5, maxDecimals: 0 };
const RATE_SMALL: ValueFormat = { decimals: 0, maxDecimals: 1, fixedDecimals: true };
const RATE_LARGE: ValueFormat = { decimals: 0, maxDecimals: 0 };
const RATE_CUT = 100n * 100_000n;
const UNIT = 100_000;

const PICTURE: TableColumnSpec = { key: 'picture', label: 'Picture', as: 'plain', cellKind: 'kind', offered: OFFERED.text, priority: 0, sortable: false, hideable: false, pinned: true };
const NAME: TableColumnSpec = { key: 'equipment', label: 'Equipment', as: 'plain', cellKind: 'item', offered: OFFERED.text, priority: 0, text: true, hideable: false, sortable: false, pinned: true, minWidth: 92, grow: 1 };
const NET: TableColumnSpec = { key: 'net', label: 'Short or over', as: 'plain', cellKind: 'stockpileNet', offered: OFFERED.number, priority: 1, sortable: false, hideable: false, headerGroup: { kind: 'shortOver' }, minWidth: 76, grow: 1 };

function sumColumn(ai: string | null): TableColumnSpec {
  return {
    key: 'sum', label: 'Produced − Need = Balance', unit: 'per day', as: 'plain', cellKind: 'stockpileSum', offered: OFFERED.number, priority: 1, sortable: false, hideable: false, numeric: true, minWidth: 174, grow: 0,
    headerGroup: { kind: 'parts', parts: [{ label: 'Produced' }, { label: 'Need', operator: '−', ...(ai ? { notRecordedReason: ai } : {}) }, { label: 'Balance', operator: '=', ...(ai ? { notRecordedReason: ai } : {}) }] },
  };
}

const BALANCE: TableColumnSpec = { key: 'sum', label: 'Balance a day', as: 'plain', cellKind: 'stockpileBalance', offered: OFFERED.number, priority: 1, sortable: false, hideable: false, numeric: true, minWidth: 174, grow: 0 };

export function stockpileColumns(layout: StockpileLayout, ai: string | null): TableColumnSpec[] {
  const figures = layout === 'parts' ? BALANCE : sumColumn(ai);
  return ai ? [PICTURE, NAME, figures] : [PICTURE, NAME, NET, figures];
}

export const STOCKPILE_BASE: TableBase = { density: 'normal', barsBehind: 'off', showRank: false, sort: { column: 'equipment', direction: 'ascending' } };

function figureOf(figure: StockFigure): bigint | null {
  return figure.raw === null ? null : BigInt(figure.raw);
}

function rate(raw: bigint, signed = false): string {
  const magnitude = raw < 0n ? -raw : raw;
  const text = formatScalar(Number(magnitude) / UNIT, magnitude >= RATE_CUT ? RATE_LARGE : RATE_SMALL);
  if (!signed || raw === 0n) return text;
  return `${raw < 0n ? '−' : '+'}${text}`;
}

function stored(figure: StockFigure): Value {
  const raw = figureOf(figure);
  if (raw === null) return notRecorded(figure.missing ?? 'Stock not recorded');
  return raw === 0n ? zeroValue(0n) : knownValue(raw);
}

function storedText(figure: StockFigure): string {
  const raw = figureOf(figure);
  return raw === null ? 'Stored not recorded' : `${formatScalar(raw, STORED)} stored`;
}

function stockpileCell(row: StockTypeRow, ai: string | null): TableCellStockpile {
  const produced = figureOf(row.produced);
  const need = figureOf(row.need);
  const balance = figureOf(row.balance);
  return {
    produced: produced === null ? null : rate(produced),
    need: need === null ? null : rate(need),
    balance: balance === null ? null : rate(balance, true),
    short: balance !== null && balance < 0n,
    share: row.share === null ? null : Math.max(-1, Math.min(1, row.share)),
    producedMissing: row.produced.missing,
    needMissing: row.need.missing,
    balanceMissing: row.balance.missing,
    ai: ai !== null,
  };
}

function shareNote(row: StockTypeRow): string | null {
  if (row.share === null) return null;
  const abs = Math.abs(row.share);
  const text = abs <= 1 ? formatShare(abs) : formatScalar(abs * 100, { decimals: 0, maxDecimals: 0, percent: true });
  return row.share < 0 ? `${text} short of need` : `${text} over need`;
}

function tooltipRows(row: StockTypeRow): TooltipExtraRow[] {
  const figure = (label: string, value: StockFigure, signed = false): TooltipExtraRow => {
    const raw = figureOf(value);
    return { label, value: raw === null ? notRecorded(value.missing ?? `${label} not recorded`) : knownValue(rate(raw, signed)) };
  };
  return [
    { label: 'Stored', value: stored(row.stored), format: STORED },
    figure('Produced a day', row.produced),
    figure('Need a day', row.need),
    figure('Balance a day', row.balance, true),
  ];
}

function variantRow(
  readView: BoundedReadView,
  parent: string,
  variant: StockVariantRow,
  duplicateName: boolean,
  duplicateNameOrigin: boolean,
): TableRow {
  const name = variant.name ?? `Variant ${variant.id}`;
  const origin = variant.origin ? resolveIdentity(readView, 'country', variant.origin, { flag: 'small' }) : null;
  const disambiguator = duplicateNameOrigin
    ? (origin ? `${origin.name} · Design #${variant.id}` : `Design #${variant.id}`)
    : duplicateName
      ? (origin ? origin.name : `Design #${variant.id}`)
      : undefined;
  return {
    key: `${parent}/${variant.id}`,
    name,
    cells: {
      equipment: {
        value: knownValue(name),
        ...(disambiguator ? { inlineSubtext: disambiguator } : {}),
        ...(origin?.art ? { art: origin.art } : {}),
        ...(origin?.artKey ? { artKey: origin.artKey } : {}),
        ...(variant.origin ? { country: variant.origin } : {}),
        ...(origin ? { iconSlot: true } : {}),
      },
      net: { value: notApplicable('Shown for each type'), blank: true },
      sum: { value: stored(variant.stored), format: STORED, unitSuffix: 'stored' },
    },
    tooltipExtra: [
      ...(duplicateNameOrigin ? [{ label: 'Design ID', value: knownValue(variant.id) }] : []),
      { label: 'Design origin', value: origin ? origin.value : notRecorded('The design names no origin') },
      { label: 'Stored', value: stored(variant.stored), format: STORED },
      { label: 'Produced a day', value: figureOf(variant.produced) === null ? notRecorded(variant.produced.missing ?? 'Not recorded') : knownValue(rate(figureOf(variant.produced)!)) },
    ],
  };
}

function typeRow(readView: BoundedReadView, row: StockTypeRow, art: ReadonlyMap<string, StockpileArtworkReply['entries'][number]>, ai: string | null): TableRow {
  const name = row.name ?? unresolvedTypeName(row.key);
  const picture = art.get(`type:${row.key.toLowerCase()}`);
  const note = shareNote(row);
  const nameCounts = new Map<string, number>();
  const originCounts = new Map<string, number>();
  for (const variant of row.variants) {
    const label = variant.name ?? `Variant ${variant.id}`;
    nameCounts.set(label, (nameCounts.get(label) ?? 0) + 1);
    const key = `${label}\u0000${variant.origin ?? ''}`;
    originCounts.set(key, (originCounts.get(key) ?? 0) + 1);
  }
  return {
    key: row.key,
    name,
    cells: {
      picture: artKindCell(readView, urlArtHandle(picture?.icon ?? null), 'equipment', name, picture?.reason ?? 'Equipment picture unavailable'),
      equipment: { value: row.name ? knownValue(row.name) : notRecorded(name), subtext: storedText(row.stored) },
      net: { value: row.share === null ? notApplicable(row.balance.missing ?? 'No need to measure against') : knownValue(row.share), stockpile: stockpileCell(row, ai) },
      sum: { value: row.balance.raw === null ? notRecorded(row.balance.missing ?? 'Balance not recorded') : knownValue(BigInt(row.balance.raw)), stockpile: stockpileCell(row, ai) },
    },
    tooltipExtra: tooltipRows(row),
    ...(note ? { tooltipNote: note } : {}),
    ...(row.variants.length
      ? {
          children: row.variants.map(variant => {
            const label = variant.name ?? `Variant ${variant.id}`;
            return variantRow(
              readView,
              row.key,
              variant,
              (nameCounts.get(label) ?? 0) > 1,
              (originCounts.get(`${label}\u0000${variant.origin ?? ''}`) ?? 0) > 1,
            );
          }),
        }
      : {}),
  };
}

function artIndex(readView: BoundedReadView): ReadonlyMap<string, StockpileArtworkReply['entries'][number]> {
  const reply = readView.panel<StockpileArtworkReply>('art');
  return new Map(reply?.entries.map(entry => [entry.key.toLowerCase(), entry]) ?? []);
}

function project(readView: BoundedReadView, options: StockpileProjectionOptions): TableModel {
  const hour = readView.hour;
  const refused = readView.refused('main');
  const reply = readView.panel<StockpileTypesReply>('main');
  const ai = reply?.needMissing ?? null;
  const columns = stockpileColumns(options.layout, ai);
  const base = { columns, hour, unsorted: true, rowHeightMode: 'media' as const, ...(options.layout === 'parts' ? { hideHeader: true } : {}) };
  if (refused !== undefined) return { ...base, rows: [], refused };
  if (!reply) return { ...base, rows: [] };
  const art = artIndex(readView);
  const rows = reply.rows.map(row => typeRow(readView, row, art, ai));
  const warnings = [...reply.warnings, ...(readView.panel<StockpileArtworkReply>('art')?.warnings ?? [])];
  return {
    ...base,
    rows,
    nextChangeAt: reply.validUntil + 1,
    ...(warnings.length ? { warnings } : {}),
    ...(rows.length ? {} : { empty: 'No equipment stored, made or needed' }),
  };
}

export const stockpileProjection: TableProjection<StockpileProjectionOptions> = registerFamilyProjection<TableProjection<StockpileProjectionOptions>>({ kind: 'equipment-stockpile', family: 'table', project }, 'bars');

export function validateStockpileTypes(raw: unknown): StockpileTypesReply {
  const reply = raw as Partial<StockpileTypesReply> | null;
  if (!reply || typeof reply.hour !== 'number' || typeof reply.validUntil !== 'number' || !Array.isArray(reply.rows) || !Array.isArray(reply.warnings)) {
    throw new Error('Invalid equipment stockpile reply');
  }
  return reply as StockpileTypesReply;
}

function sources(ctx: FamilyPlanContext): SourcePlan {
  const tag = ctx.countryTag;
  const owner = `equipment-stockpile:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${tag ?? ''}`;
  if (!tag) return { owner, entries: [] };
  const country = countryFromKey(tag);
  const params = { branchId: ctx.branch.id, country };
  const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
  return {
    owner,
    entries: [
      {
        name: 'main',
        ...common,
        read: {
          shape: 'point',
          channel: 'followed',
          coverage: { mode: 'keyed', keys: [tag] },
          product: 'equipment-stockpile-types',
          followQuery: null,
          params,
          hourParam: 'hour',
          provenInterval: (data, at) => {
            const reply = data as StockpileTypesReply;
            return reply.hour <= at && at <= reply.validUntil ? [reply.hour, reply.validUntil] : [at, at];
          },
          validate: validateStockpileTypes,
        },
      },
      {
        name: 'art',
        ...common,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'keyed', keys: [tag] },
          product: 'stockpile-artwork',
          followQuery: null,
          params: { ...params, variants: [] },
          hourParam: 'hour',
          validate: validateStockpileArtwork,
        },
      },
    ],
  };
}

export const stockpileDeclaration: TableWidgetDeclaration<StockpileProjectionOptions> = {
  family: 'table',
  kind: 'equipment-stockpile',
  label: 'Equipment stockpile',
  subject: 'equipment stockpile',
  emptyCampaign: 'Load a campaign and choose a country to see its equipment stockpile.',
  icon: 'grid',
  countries: { mode: 'single', choose: 'Choose a country to see its equipment stockpile.' },
  sources,
  projection: stockpileProjection,
  columns: ctx => stockpileColumns(ctx.options.layout === 'parts' ? 'parts' : 'sum', null),
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: STOCKPILE_BASE },
  projectionOptions: ctx => ({ layout: ctx.options.layout === 'parts' ? 'parts' : 'sum' }),
  keyByBranch: true,
};
