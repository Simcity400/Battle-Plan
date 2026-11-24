import assert from 'node:assert/strict';
import test from 'node:test';
import { MATRIX_HEADER, matrixCellAt, matrixKey, matrixKeyed, matrixWindow, revealItem } from './matrix.ts';


test('both axes window to the viewport, with a margin, for 5,000 items', () => {
  const rows = matrixWindow(5000, 24 * 1000, 300, 24);
  assert.deepEqual(rows, { first: 999, end: 1014 });
  assert.deepEqual(matrixWindow(3, 0, 300, 24), { first: 0, end: 3 });
  const layout = { gutter: 100, header: MATRIX_HEADER, cell: 24, scrollLeft: 48, scrollTop: 24 };
  assert.deepEqual(matrixCellAt(100 + 5, MATRIX_HEADER + 5, layout, 10, 10), { row: 1, column: 2 });
  assert.equal(matrixCellAt(50, MATRIX_HEADER + 5, layout, 10, 10), null, 'the gutter is not a cell');
  assert.equal(matrixCellAt(100 + 24 * 20, MATRIX_HEADER + 5, layout, 10, 10), null, 'past the last column');
});

test('keys step cells, keep within the matrix and reveal the target', () => {
  assert.deepEqual(matrixKey('ArrowRight', { row: 0, column: 9 }, 10, 10, 5), { row: 0, column: 9 });
  assert.deepEqual(matrixKey('ArrowDown', { row: 3, column: 4 }, 10, 10, 5), { row: 4, column: 4 });
  assert.deepEqual(matrixKey('End', { row: 3, column: 4 }, 10, 12, 5), { row: 3, column: 11 });
  assert.deepEqual(matrixKey('PageDown', { row: 8, column: 0 }, 10, 10, 5), { row: 9, column: 0 });
  assert.equal(matrixKey('a', { row: 0, column: 0 }, 10, 10, 5), null);
  assert.equal(matrixKey('ArrowDown', { row: 0, column: 0 }, 0, 10, 5), null);
  assert.equal(revealItem(10, 24, 0, 100), 24 * 11 - 100);
  assert.equal(revealItem(1, 24, 100, 100), 24);
  assert.equal(revealItem(5, 24, 100, 100), 100);
});

test('a keyboard cell stays on the matrix when its axes shrink', () => {
  assert.deepEqual(matrixKey('ArrowDown', { row: 2, column: 11 }, 3, 3, 1), { row: 2, column: 2 });
  assert.deepEqual(matrixKey('ArrowRight', { row: 11, column: 0 }, 3, 3, 1), { row: 2, column: 1 });
  const axis = (...ids: string[]) => ids.map(id => ({ id, label: id }));
  const twelve = axis(...Array.from({ length: 12 }, (_, index) => `c${index}`));
  const kept = { row: 2, column: 11, rowId: 'c2', columnId: 'c11' };
  assert.equal(matrixKeyed(kept, twelve, twelve), kept, 'unchanged axes keep the same cell');
  assert.deepEqual(matrixKeyed(kept, axis('c11', 'c5', 'c2'), axis('c11', 'c5', 'c2')), { row: 2, column: 0, rowId: 'c2', columnId: 'c11' },
    'a row and column still shown are followed to their new places');
  assert.deepEqual(matrixKeyed(kept, axis('c0', 'c1', 'c3'), axis('c0', 'c1', 'c3')), { row: 2, column: 2, rowId: 'c3', columnId: 'c3' },
    'a row or column that left gives way to the nearest one still shown');
  assert.equal(matrixKeyed(kept, [], twelve), null, 'an empty axis has no cell');
});

