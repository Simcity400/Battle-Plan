import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { BarsDataRow } from '../../families/bars/model.ts';
import type { CasualtyMatrixReply } from '../../generated/dashboard';
import { projectCasualtyMatrix, projectCasualtyOpponents } from './casualtiesBetweenProjection.ts';
import { matrixCellAt } from '../../families/heat/matrixModel.ts';

const HOUR = 60_810_000;
const country = (tag: string, dealt: string | null, taken: string | null) => ({
  tag, factionId: null, factionName: null, factionNameMissing: null, dealt, taken,
  missing: dealt === null || taken === null ? 'A war relation\'s casualties were not recorded at this hour' : null,
});

const reply: CasualtyMatrixReply = {
  countries: [country('GER', null, '100'), country('POL', '100', '900'), country('FRA', '0', '0')],
  order: [0, 1, 2],
  cells: [
    { row: 0, column: 1, dealt: '900', missing: null, incomplete: null },
    { row: 1, column: 0, dealt: '100', missing: null, incomplete: 'Part of this pair\'s history fell in a recording gap; the figure may be low' },
    { row: 0, column: 2, dealt: null, missing: 'A war relation\'s casualties were not recorded at this hour', incomplete: null },
    { row: 2, column: 0, dealt: '0', missing: null, incomplete: null },
  ],
  most: '900',
  largestDealt: '100',
  subject: null,
  opponents: [],
  largestSide: '0',
  missing: null,
  warnings: [],
};

const view = (data: unknown) => createBoundedReadView({ hour: HOUR, panels: new Map([['main', data]]) });

test('casualty matrix: not recorded pairs and totals are hatched, a recorded zero is empty, never a guess', () => {
  const model = projectCasualtyMatrix(view(reply));
  assert.deepEqual(matrixCellAt(model, 0, 0), { kind: 'diagonal', text: '—' });
  const big = matrixCellAt(model, 0, 1);
  assert.equal(big.kind === 'value' && big.text, '900');
  assert.equal(big.kind === 'value' && big.intensity, 1);
  assert.equal(matrixCellAt(model, 0, 2).kind, 'missing');
  assert.deepEqual(matrixCellAt(model, 2, 0), { kind: 'empty', label: '0 casualties', audit: '0' });
  assert.equal(matrixCellAt(model, 1, 2).kind, 'empty');
  assert.equal(model.margin!.cell(0).kind, 'missing');
  const total = model.margin!.cell(1);
  assert.equal(total.kind === 'value' && total.fraction, 1);
  assert.ok(model.legend?.entries?.some(entry => entry.hatched));
});

test('one country by opponent: taken on the left, dealt on the right, missing never zero', () => {
  const opponents: CasualtyMatrixReply = {
    ...reply,
    subject: 0,
    order: [],
    cells: [],
    opponents: [
      { country: 1, dealt: '900', taken: '100', missing: null, incomplete: null },
      { country: 2, dealt: null, taken: null, missing: 'A war relation\'s casualties were not recorded at this hour', incomplete: null },
    ],
    largestSide: '900',
  };
  const model = projectCasualtyOpponents(view(opponents));
  const [poland, france] = model.rows as BarsDataRow[];
  assert.equal(poland!.left!.numeric, 100);
  assert.equal(poland!.right!.numeric, 900);
  assert.equal(france!.left!.numeric, null);
  assert.equal(france!.left!.value.value.kind, 'notRecorded');
  assert.equal(model.scaleMax, 900);
  assert.equal(model.summaryText, undefined, 'a total that is not recorded is not shown');
});
