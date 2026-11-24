import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import type { CareerRow, DivisionCareersReply } from '../../generated/dashboard';
import { mostDecoratedProjection } from './mostDecoratedProjection.ts';

const division = (id: string, extra: Partial<CareerRow['facts']> = {}): CareerRow => ({
  division: id, tag: 'CHI', template: '4', name: `${id}th Division`, nameMissing: null,
  facts: { medals: 2, awards: [{ medal: 0, count: 2 }], historyMissing: null, experienceRaw: '30000', level: 1, levelName: 'Trained', kills: 13671, ...extra },
});

const reply = (divisions: CareerRow[]): DivisionCareersReply => ({
  divisions, medals: [{ key: 'medal_a', name: 'Order of the Cloud' }], templates: [{ template: '4', name: 'Infantry', icon: null }], missing: null, warnings: [],
});

const project = (raw: unknown) => mostDecoratedProjection.project(
  createBoundedReadView({ hour: 61000000, panels: new Map([['main', raw]]) }),
  { filter: { passing: null, unrecorded: [], bounds: [] } },
);

test('a division shows its recorded medals, experience word and casualties dealt; an unread history is missing, a recorded zero stays zero', () => {
  const model = project(reply([division('66'), division('7', { medals: null, historyMissing: 'Medal history was not read', awards: [], kills: 0 })]));
  const [decorated, unread] = model.rows;
  assert.deepEqual([decorated!.cells.medals!.value, decorated!.cells.experience!.value, decorated!.cells.casualtiesTaken!.value], [knownValue(2), knownValue('Trained'), knownValue(13671)]);
  assert.deepEqual(unread!.cells.medals!.value, notRecorded('Medal history was not read'));
  assert.deepEqual(unread!.cells.casualtiesTaken!.value, zeroValue(0));
  const tip = typeof decorated!.tooltipExtra === 'function' ? decorated!.tooltipExtra() : decorated!.tooltipExtra;
  assert.deepEqual(tip?.map(row => [row.label, row.value]), [['Order of the Cloud', knownValue('×2')]]);
});
