import type { BoundedReadView } from '../../families/contract.ts';
import { projectCountryFlow, scaledAmount } from '../../families/flow/countryFlow.ts';
import { countryFlowPlan, flowPerSide } from '../../families/flow/countryFlowPlan.ts';
import type { FlowWidgetDeclaration } from '../../families/flow/declaration.ts';
import type { FlowProjection } from '../../families/flow/model.ts';
import type { HeatMatrixWidgetDeclaration } from '../../families/heat/matrixDeclaration.ts';
import { NO_CELLS, type HeatMatrixModel, type HeatMatrixProjection, type MatrixAxisEntity, type MatrixCell } from '../../families/heat/matrixModel.ts';
import { resolveEntityFromReadView } from '../../families/heat/scale.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { countryRowPresentation } from '../../families/bars/model.ts';
import type { TableColumnSpec, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import { knownValue, notRecorded, zeroValue, type Value } from '../../families/value.ts';
import { formatCompactScalar, formatScalar, type ValueFormat } from '../../families/valueFormat.ts';
import type { CountryFlowReply } from '../../generated/dashboard';

const EMPTY_OPTIONS: Readonly<Record<string, never>> = {};
const NOT_RECORDED = 'Trade was not recorded at this date.';
const NO_TRADE = 'No trade at this date.';
const SIGNED: ValueFormat = { decimals: 0, maxDecimals: 1, signed: true };
const signedFormat = (decimals: number): ValueFormat => ({ decimals, maxDecimals: 1, signed: true });
const amountFormat = (decimals: number): ValueFormat => ({ decimals, maxDecimals: 1 });
const exactAmountFormat = (decimals: number): ValueFormat => ({ decimals });

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

function signed(amount: string | null, reason: string): Value {
  if (amount === null) return notRecorded(reason);
  const raw = BigInt(amount);
  return raw === 0n ? zeroValue(0n) : knownValue(raw);
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
  const signedCellFormat = signedFormat(reply.decimals);
  const partColumns: TableColumnSpec[] = reply.parts.flatMap((part, at) => used[at] ? [{
    key: `part:${part.key}`,
    label: part.name ?? part.key,
    as: 'plain',
    offered: ['plain'],
    priority: 2 + at,
    numeric: true,
    cellKind: 'shaded',
    format: signedCellFormat,
    widest: ['+999.9'],
    sortable: false,
    hideable: true,
  } satisfies TableColumnSpec] : []);
  const subject = countryRowPresentation(readView, reply.countries[reply.subject]!);
  const rows: TableRow[] = reply.table.map(row => {
    const tag = reply.countries[row.country]!;
    const partner = countryRowPresentation(readView, tag);
    const name: Value = partner.labelMissing ? notRecorded('Country name not recorded at this date') : knownValue(partner.label);
    const icon = partner.media.iconUrl;
    const cells: Record<string, TableRow['cells'][string]> = {
      partner: { value: name, flags: [{ tag, ...(icon ? { icon } : {}), ...(partner.media.art.status === 'pending' ? { pending: true } : {}) }] },
      net: { value: signed(row.net, 'An amount with this partner was not recorded'), format: signedCellFormat },
    };
    reply.parts.forEach((part, at) => {
      if (!used[at]) return;
      const cell = row.cells[at] ?? null;
      const value = signed(cell, 'This deal\'s daily delivery was not recorded');
      cells[`part:${part.key}`] = cell === '0'
        ? { value, blank: true }
        : { value, format: signedCellFormat };
    });
    return { key: tag, name: partner.label, cells, tooltipTitle: `${partner.label} and ${subject.label}` };
  });
  return {
    hour,
    columns: [PARTNER, ...partColumns, NET],
    rows,
    unsorted: true,
    titleSummary: `Per day · + imports into ${subject.label}, − exports from it`,
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
  settings: { kind: 'fixed', base: { density: 'compact', sort: { column: 'net', direction: 'descending' } } },
  projectionOptions: () => EMPTY_OPTIONS,
  jumpToDate: true,
};

export function projectTradeMatrix(readView: BoundedReadView): HeatMatrixModel {
  const hour = readView.hour;
  const blank = (extra: Partial<HeatMatrixModel>): HeatMatrixModel => ({ hour, rows: [], columns: [], cells: NO_CELLS, ...extra });
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
  const shownFmt = amountFormat(reply.decimals);
  const exactFmt = exactAmountFormat(reply.decimals);
  const most = Math.max(1e-9, scaledAmount(reply.largestLink, reply.decimals));
  const present = new Map<number, MatrixCell>();
  for (const [at, link] of cells) {
    if (link.total === null) {
      present.set(at, { kind: 'missing', reason: link.missing ?? NOT_RECORDED });
      continue;
    }
    const raw = BigInt(link.total);
    const value = scaledAmount(link.total, reply.decimals);
    present.set(at, { kind: 'value', intensity: value / most, text: formatCompactScalar(raw, shownFmt).text, audit: formatScalar(raw, exactFmt), value, selectable: true });
  }
  return {
    hour,
    rows,
    columns,
    cells: { present, absent: { kind: 'empty', label: 'No trade', audit: '0' }, self: { kind: 'diagonal', text: '—' } },
    shape: 'wide',
    ramp: 'accent',
    corner: 'Exporter ↓ · importer →',
    legend: {
      gradient: { lowLabel: '0', highLabel: `${formatCompactScalar(BigInt(reply.largestLink), shownFmt).text} per day`, stops: [] },
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
          value: amount.amount === null ? 'Not recorded' : `${formatScalar(BigInt(amount.amount), exactFmt)} per day`,
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
