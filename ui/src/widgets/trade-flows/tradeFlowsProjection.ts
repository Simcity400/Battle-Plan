import type { BoundedReadView } from '../../families/contract.ts';
import { projectCountryFlow, scaledAmount } from '../../families/flow/countryFlow.ts';
import { countryFlowPlan, flowPerSide } from '../../families/flow/countryFlowPlan.ts';
import type { FlowWidgetDeclaration } from '../../families/flow/declaration.ts';
import type { FlowProjection } from '../../families/flow/model.ts';
import type { HeatMatrixWidgetDeclaration } from '../../families/heat/matrixDeclaration.ts';
import type { HeatMatrixModel, HeatMatrixProjection, MatrixAxisEntity, MatrixCell } from '../../families/heat/matrixModel.ts';
import { resolveEntityFromReadView } from '../../families/heat/scale.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { resolveCountryFromReadView, type TableColumnSpec, type TableModel, type TableProjection, type TableRow } from '../../families/table/model.ts';
import { knownValue, notRecorded, zeroValue, type Value } from '../../families/value.ts';
import { formatCompactScalar, formatScalar, type ValueFormat } from '../../families/valueFormat.ts';
import type { CountryFlowReply } from '../../generated/dashboard';

const EMPTY_OPTIONS: Readonly<Record<string, never>> = {};
const NOT_RECORDED = 'Trade was not recorded at this date.';
const NO_TRADE = 'No trade at this date.';
const AMOUNT: ValueFormat = { decimals: 0, maxDecimals: 1 };
const SIGNED: ValueFormat = { decimals: 0, maxDecimals: 1, signed: true };

export const tradeFlowProjection: FlowProjection = {
  family: 'flow',
  kind: 'trade-flows',
  project: readView => projectCountryFlow(readView, {
    leftHeader: 'Imports from',
    rightHeader: 'Exports to',
    unitNote: 'per day',
    groupLeftHeader: 'Exporters',
    groupRightHeader: 'Importers',
    notRecorded: NOT_RECORDED,
    none: NO_TRADE,
    others: count => `${count} other ${count === 1 ? 'partner' : 'partners'}`,
    ribbonTitle: (from, to) => `${from} → ${to}`,
  }),
};

export const tradeFlowDeclaration: FlowWidgetDeclaration = {
  family: 'flow',
  kind: 'trade-flows',
  label: 'Trade flows',
  subject: 'trade',
  emptyCampaign: 'Load a campaign to see its trade.',
  icon: 'flows',
  countries: { mode: 'multi', labels: false, colours: false },
  sources: (ctx, view) => countryFlowPlan(ctx, 'trade-flow', { limit: flowPerSide(view) }),
  projection: tradeFlowProjection,
  projectionOptions: () => EMPTY_OPTIONS,
  jumpToDate: true,
};

const PARTNER: TableColumnSpec = { key: 'partner', label: 'Partner', as: 'plain', offered: ['plain'], priority: 0, text: true, cellKind: 'item', pinned: true, sortable: false };
const NET: TableColumnSpec = { key: 'net', label: 'Net', as: 'plain', offered: ['plain'], priority: 1000, numeric: true, format: SIGNED, widest: ['+999.9'], sortable: false, hideable: false };

function signed(amount: string | null, decimals: number, reason: string): Value {
  if (amount === null) return notRecorded(reason);
  const value = scaledAmount(amount, decimals);
  return value === 0 ? zeroValue(0) : knownValue(value);
}

export function projectTradeTable(readView: BoundedReadView): TableModel {
  const hour = readView.hour;
  const blank = (extra: Partial<TableModel>): TableModel => ({ hour, columns: [PARTNER, NET], rows: [], unsorted: true, ...extra });
  const refused = readView.refused('main');
  if (refused) return blank({ refused });
  const reply = readView.panel<CountryFlowReply>('main');
  if (!reply) return blank({ empty: NOT_RECORDED, emptyKind: 'unavailable' });
  if (reply.missing) return blank({ empty: NOT_RECORDED, emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings });
  if (reply.subject === null) return blank({ empty: 'Choose one country to see its trade by partner and resource.', emptyKind: 'unavailable' });
  if (!reply.table.length) return blank({ empty: NO_TRADE, warnings: reply.warnings });
  const used = reply.parts.map((_, part) => reply.table.some(row => row.cells[part] !== '0'));
  const partColumns: TableColumnSpec[] = reply.parts.flatMap((part, at) => used[at] ? [{
    key: `part:${part.key}`,
    label: part.name ?? part.key,
    as: 'plain',
    offered: ['plain'],
    priority: 2 + at,
    numeric: true,
    cellKind: 'shaded',
    format: SIGNED,
    widest: ['+999.9'],
    sortable: false,
    hideable: true,
  } satisfies TableColumnSpec] : []);
  const subject = resolveCountryFromReadView(readView, reply.countries[reply.subject]);
  const rows: TableRow[] = reply.table.map(row => {
    const tag = reply.countries[row.country]!;
    const partner = resolveCountryFromReadView(readView, tag);
    const cells: Record<string, TableRow['cells'][string]> = {
      partner: { value: partner.value, flags: [{ tag, ...(partner.icon ? { icon: partner.icon } : {}), ...(partner.iconPending ? { pending: true } : {}) }] },
      net: { value: signed(row.net, reply.decimals, 'An amount with this partner was not recorded'), format: SIGNED },
    };
    reply.parts.forEach((part, at) => {
      if (!used[at]) return;
      const cell = row.cells[at] ?? null;
      const value = signed(cell, reply.decimals, 'This deal\'s daily delivery was not recorded');
      cells[`part:${part.key}`] = cell === '0'
        ? { value, blank: true }
        : { value, format: SIGNED };
    });
    return { key: tag, name: partner.name, cells, tooltipTitle: `${partner.name} and ${subject.name}` };
  });
  return {
    hour,
    columns: [PARTNER, ...partColumns, NET],
    rows,
    unsorted: true,
    titleSummary: `Per day · + imports into ${subject.name}, − exports from it`,
    ...(reply.partial ? { notes: [reply.partial] } : {}),
    warnings: reply.warnings,
  };
}

export const tradeTableProjection: TableProjection = {
  family: 'table',
  kind: 'trade-flows',
  project: readView => projectTradeTable(readView),
};

export const tradeTableDeclaration: TableWidgetDeclaration = {
  family: 'table',
  kind: 'trade-flows',
  label: 'Trade flows',
  subject: 'trade',
  emptyCampaign: 'Load a campaign to see its trade.',
  icon: 'flows',
  countries: { mode: 'single', choose: 'Choose one country to see its trade by partner and resource.' },
  sources: ctx => countryFlowPlan(ctx, 'trade-flow', { table: true }),
  projection: tradeTableProjection,
  columns: [PARTNER],
  settings: { kind: 'fixed', base: { density: 'compact', overflow: 'scroll', sort: { column: 'net', direction: 'descending' } } },
  projectionOptions: () => EMPTY_OPTIONS,
  jumpToDate: true,
};

export function projectTradeMatrix(readView: BoundedReadView): HeatMatrixModel {
  const hour = readView.hour;
  const blank = (extra: Partial<HeatMatrixModel>): HeatMatrixModel => ({ hour, rows: [], columns: [], cell: () => ({ kind: 'empty' }), ...extra });
  const refused = readView.refused('main');
  if (refused) return blank({ refused });
  const reply = readView.panel<CountryFlowReply>('main');
  if (!reply) return blank({ empty: NOT_RECORDED, emptyKind: 'unavailable' });
  if (reply.missing) return blank({ empty: NOT_RECORDED, emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings });
  if (!reply.links.length) return blank({ empty: NO_TRADE, warnings: reply.warnings });
  const entity = (index: number): MatrixAxisEntity => {
    const tag = reply.countries[index]!;
    const shown = resolveEntityFromReadView(readView, 'country', tag, hour, tag);
    return {
      id: tag,
      label: shown.label,
      shortLabel: tag,
      tag,
      ...(shown.nameMissing ? { nameMissing: shown.nameMissing } : {}),
      ...(shown.color ? { color: shown.color } : {}),
      flag: shown.flag ?? null,
      flagPending: shown.pending === true,
    };
  };
  const rows = reply.senders.flatMap(link => (link.from === null ? [] : [entity(link.from)]));
  const columns = reply.receivers.flatMap(link => (link.to === null ? [] : [entity(link.to)]));
  const rowAt = new Map(rows.map((row, at) => [row.id, at]));
  const columnAt = new Map(columns.map((column, at) => [column.id, at]));
  const width = columns.length;
  const cells = new Map<number, CountryFlowReply['links'][number]>();
  for (const link of reply.links) {
    if (link.from === null || link.to === null) continue;
    const r = rowAt.get(reply.countries[link.from]!);
    const c = columnAt.get(reply.countries[link.to]!);
    if (r !== undefined && c !== undefined) cells.set(r * width + c, link);
  }
  const most = Math.max(1e-9, scaledAmount(reply.largestLink, reply.decimals));
  const cell = (r: number, c: number): MatrixCell => {
    if (rows[r]!.id === columns[c]!.id) return { kind: 'diagonal', text: '—' };
    const link = cells.get(r * width + c);
    if (!link) return { kind: 'empty', label: 'No trade', audit: '0' };
    if (link.total === null) return { kind: 'missing', reason: link.missing ?? NOT_RECORDED };
    const value = scaledAmount(link.total, reply.decimals);
    return { kind: 'value', intensity: value / most, text: formatCompactScalar(value, AMOUNT).text, audit: formatScalar(value, AMOUNT), value, selectable: true };
  };
  return {
    hour,
    rows,
    columns,
    cell,
    shape: 'wide',
    ramp: 'accent',
    corner: 'Exporter ↓ · importer →',
    legend: {
      gradient: { lowLabel: '0', highLabel: `${formatCompactScalar(most, AMOUNT).text} per day`, stops: [] },
      ...(reply.links.some(link => link.total === null) ? { entries: [{ id: 'missing', label: 'Not recorded', color: '', hatched: true }] } : {}),
    },
    tooltip: (row, column, item, r, c) => {
      if (item.kind === 'diagonal') return null;
      const title = `${row.label} → ${column.label}`;
      if (item.kind === 'missing') return { title, hour, missingReason: item.reason };
      const link = cells.get(r * width + c);
      if (!link || link.total === null) return { title, hour, items: [{ label: 'No trade', tone: 'muted' }] };
      return {
        title,
        hour,
        items: link.amounts.map(amount => ({
          label: reply.parts[amount.part]?.name ?? reply.parts[amount.part]?.key ?? '',
          value: amount.amount === null ? 'Not recorded' : `${formatScalar(scaledAmount(amount.amount, reply.decimals), AMOUNT)} per day`,
        })),
      };
    },
    ...(reply.partial ? { notes: [reply.partial] } : {}),
    warnings: reply.warnings,
  };
}

export const tradeMatrixProjection: HeatMatrixProjection = {
  kind: 'trade-flows',
  family: 'heat-matrix',
  project: readView => projectTradeMatrix(readView),
};

export const tradeMatrixDeclaration: HeatMatrixWidgetDeclaration = {
  family: 'heat-matrix',
  kind: 'trade-flows',
  label: 'Trade flows',
  subject: 'trade',
  emptyCampaign: 'Load a campaign to see its trade.',
  icon: 'flows',
  countries: { mode: 'multi', labels: false, colours: false },
  sources: ctx => countryFlowPlan(ctx, 'trade-flow', {}, null),
  projection: tradeMatrixProjection,
  projectionOptions: () => EMPTY_OPTIONS,
  jumpToDate: true,
};
