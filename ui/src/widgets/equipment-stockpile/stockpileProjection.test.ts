import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { StockFigure, StockTypeRow, StockpileArtworkReply, StockpileTypesReply } from '../../generated/dashboard';
import { buildColumnLayouts, measureFixedColumnWidths } from '../../families/table/layout.ts';
import { tableSettingsOf } from '../../families/table/settings.ts';
import { STOCKPILE_BASE, stockpileColumns, stockpileProjection } from './stockpileProjection.ts';

const known = (raw: number): StockFigure => ({ raw: String(Math.round(raw * 100_000)), missing: null });
const missing = (why: string): StockFigure => ({ raw: null, missing: why });

function type(key: string, extra: Partial<StockTypeRow>): StockTypeRow {
  return { key, name: key.toUpperCase(), stored: known(0), produced: known(0), need: known(0), balance: known(0), share: null, variants: [], ...extra };
}

const AI = 'The game doesn\'t compute surplus or shortage for AI-controlled countries.';

function reply(rows: StockTypeRow[], needMissing: string | null = null): StockpileTypesReply {
  return { hour: 100, validUntil: 123, decimals: 5, needMissing, rows, warnings: [] };
}

const art: StockpileArtworkReply = {
  tag: 'USA', entries: [{ key: 'type:motorized', variant: '7', name: 'Motorized', icon: 'data:image/png;base64,AA', reason: null }], warnings: [],
} as StockpileArtworkReply;

test('the stockpile table draws the product\'s rows in its order, each figure as the product answers it, until the product\'s next change', () => {
  const rows = [
    type('motorized', { stored: known(2940), produced: known(12), need: known(15.8), balance: known(-3.8), share: -3.8 / 15.8,
      variants: [{ id: '7', name: 'Truck', origin: 'USA', stored: known(2900), produced: known(12) }, { id: '8', name: 'Lorry', origin: null, stored: known(40), produced: known(0) }] }),
    type('light_armor', { stored: known(412), produced: known(3.5), need: known(1.2), balance: known(2.3), share: 2.3 / 1.2 }),
    type('guns', { stored: known(7), need: missing('The game\'s logistics lists this type for none of the country\'s needs at this hour.'), balance: missing('The game\'s logistics lists this type for none of the country\'s needs at this hour.') }),
  ];
  const asked: string[] = [];
  const view = createBoundedReadView({ hour: 100, panels: new Map<string, unknown>([['main', reply(rows)], ['art', art]]), onArtDemand: handle => { if (handle.kind === 'url') asked.push(handle.url); } });
  const model = stockpileProjection.project(view, { layout: 'sum' });
  assert.deepEqual(model.rows.map(row => row.key), ['motorized', 'light_armor', 'guns'], 'the product\'s order, never re-sorted');
  assert.equal(model.unsorted, true);
  assert.equal(model.nextChangeAt, 124, 'the answer stands through the product\'s validUntil');
  const motorized = model.rows[0]!;
  assert.deepEqual(asked, ['data:image/png;base64,AA'], 'the type picture comes from the artwork answer for this type');
  assert.equal(model.rows[1]!.cells.picture!.iconSlot, true, 'a type with no artwork entry keeps its slot for the missing tile');
  assert.equal(motorized.cells.equipment!.subtext, '2,940 stored');
  const sum = motorized.cells.sum!.stockpile!;
  assert.deepEqual([sum.produced, sum.need, sum.balance, sum.short], ['12.0', '15.8', '−3.8', true]);
  assert.ok(Math.abs(motorized.cells.net!.stockpile!.share! + 3.8 / 15.8) < 1e-9);
  assert.equal(model.rows[1]!.cells.net!.stockpile!.share, 1, 'a share above the need is capped at the whole bar');
  assert.equal(model.rows[2]!.cells.sum!.stockpile!.balance, null, 'a balance the product does not know is not drawn as zero');
  assert.equal(motorized.tooltipNote, '24% short of need');
  assert.deepEqual(motorized.children!.map(child => child.name), ['Truck', 'Lorry'], 'variants in the product\'s order');
  assert.equal(motorized.children![0]!.cells.sum!.unitSuffix, 'stored');
  const header = model.columns.find(column => column.key === 'sum')!.headerGroup;
  assert.deepEqual(header?.kind === 'parts' ? header.parts.map(part => [part.operator ?? '', part.label, part.notRecordedReason ?? '']) : null,
    [['', 'Produced', ''], ['−', 'Need', ''], ['=', 'Balance', '']]);
});

test('an AI country hatches Need and Balance in the header and has no net column; the parts layout has no header', () => {
  const rows = [type('infantry', { stored: known(19819), produced: known(623), need: missing(AI), balance: missing(AI) })];
  const view = createBoundedReadView({ hour: 100, panels: new Map<string, unknown>([['main', reply(rows, AI)]]) });
  const model = stockpileProjection.project(view, { layout: 'sum' });
  const header = model.columns.find(column => column.key === 'sum')!.headerGroup;
  assert.deepEqual(header?.kind === 'parts' ? header.parts.map(part => part.notRecordedReason ?? null) : null, [null, AI, AI]);
  const cell = model.rows[0]!.cells.sum!.stockpile!;
  assert.deepEqual([cell.ai, cell.produced, cell.share], [true, '623', null]);
  assert.deepEqual(model.columns.map(column => column.key), ['picture', 'equipment', 'sum']);
  const parts = stockpileProjection.project(view, { layout: 'parts' });
  assert.equal(parts.hideHeader, true);
  assert.equal(parts.columns.find(column => column.key === 'sum')!.cellKind, 'stockpileBalance');
});

test('the Short or over and Produced − Need = Balance headers are as wide as their words', () => {
  const columns = stockpileColumns('sum', null);
  const settings = tableSettingsOf(undefined, columns, STOCKPILE_BASE);
  const widths = measureFixedColumnWidths(buildColumnLayouts(columns, settings), settings, { font: 'sans-serif', fontSize: 13 }, text => text.length * 10);
  assert.ok(widths.net! >= 'ShortOver'.length * 10, `net ${widths.net}`);
  assert.ok(widths.sum! >= 'ProducedNeedBalance−='.length * 10, `sum ${widths.sum}`);
});
