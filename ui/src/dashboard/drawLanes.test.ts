import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DrawLanes } from './drawLanes.ts';
import type { FromWorker, ToWorker } from '../draw/kit';

const lanes = (count: number) => {
  const sent: ToWorker[][] = Array.from({ length: count }, () => []), heard: FromWorker[] = [];
  const draw = new DrawLanes(sent.map(box => ({ postMessage: (message: ToWorker) => { box.push(message); } })), message => heard.push(message));
  return { draw, sent, heard, types: (lane: number) => sent[lane]!.map(message => message.type) };
};
const table = (id: number) => ({ type: 'table', id, owner: 1, width: 1, height: 1, dpr: 1, model: {} } as unknown as ToWorker);

test('each surface draws on the least loaded lane, and what concerns it goes to that lane alone', () => {
  const { draw, types } = lanes(2);
  draw.post(table(1)); draw.post(table(2)); draw.post(table(3));
  assert.deepEqual([types(0), types(1)], [['table', 'table'], ['table']]);
  draw.post({ type: 'tableRead', id: 2, owner: 1, request: 7 });
  draw.post({ type: 'tableScroll', id: 3, owner: 1, scrollLeft: 0 });
  assert.deepEqual([types(0), types(1)], [['table', 'table', 'tableScroll'], ['table', 'tableRead']]);
  draw.post(table(2));
  assert.deepEqual(types(1), ['table', 'tableRead', 'table'], 'a new model for a surface goes where it draws');
  draw.post({ type: 'detach', id: 2 });
  draw.post(table(4));
  assert.deepEqual(types(1), ['table', 'tableRead', 'table', 'detach', 'table'], 'a detached surface frees its lane');
});

const panel = (id: number, cells: number) => ({ type: 'surface', id, owner: 1, width: 1, height: 1, dpr: 1, model: { cells: Array.from({ length: cells }) } } as unknown as ToWorker);
const sized = (id: number, columns: number, rows: number) => ({ type: 'table', id, owner: 1, width: 1, height: rows * 28, dpr: 1,
  model: { columns: Array.from({ length: columns }, () => ({ shown: true })), height: rows * 28, settings: { density: 'comfortable' } } } as unknown as ToWorker);

test('a lane weighs what its surfaces paint, so a large table draws beside fewer panels', () => {
  const { draw, types } = lanes(2);
  draw.post(sized(1, 8, 20)); draw.post(sized(2, 4, 10));
  for (let id = 3; id <= 14; id++) draw.post(panel(id, 20));
  assert.deepEqual([types(0).length, types(1).length], [4, 10], 'a 160-cell table and three panels beside a 40-cell table and nine');
  draw.post({ type: 'detach', id: 1 });
  draw.post(sized(15, 8, 20));
  assert.equal(types(0).at(-1), 'table', 'a table drawn again goes where its cells were freed');
});

test('a table not yet laid out still draws on a lane with no table', () => {
  const { draw, types } = lanes(2);
  draw.post(sized(1, 8, 20));
  for (let id = 2; id <= 9; id++) draw.post(panel(id, 20));
  draw.post(sized(10, 0, 0));
  assert.deepEqual([types(0).includes('table') && types(1).includes('table'), types(1).at(-1)], [true, 'table']);
});

test('a surface drawn again with more cells weighs its new picture', () => {
  const { draw, types } = lanes(2);
  draw.post(panel(1, 10)); draw.post(panel(2, 10));
  draw.post(panel(1, 200));
  draw.post(panel(3, 10)); draw.post(panel(4, 10));
  assert.deepEqual([types(0), types(1)], [['surface', 'surface'], ['surface', 'surface', 'surface']]);
});

test('follows, the clock and options reach every lane; a follow inspection the first alone', () => {
  const { draw, types } = lanes(3);
  draw.post({ type: 'add', id: 9, branch: 'b' });
  draw.post({ type: 'clock', anchor: { at: 0, position: 0, playing: false, speed: 1, lastHour: 0, seek: 0, generation: 0 } } as ToWorker);
  draw.post({ type: 'inspect', id: 9, request: 1 });
  assert.deepEqual([types(0), types(1), types(2)], [['add', 'clock', 'inspect'], ['add', 'clock'], ['add', 'clock']]);
});

test('a picture audit is answered once, from every lane’s records, done when every lane is', () => {
  const { draw, heard } = lanes(2);
  draw.post({ type: 'audit', start: false, limit: 10 });
  draw.receive(0, { type: 'audit', records: [{ t: 'a' } as never], done: true });
  assert.equal(heard.length, 0, 'not before every lane answers');
  draw.receive(1, { type: 'audit', records: [{ t: 'b' } as never], done: false });
  assert.deepEqual(heard, [{ type: 'audit', records: [{ t: 'a' }, { t: 'b' }], done: false }]);
});

test('page-wide news comes from the first lane alone; a surface’s from any', () => {
  const { draw, heard } = lanes(2);
  draw.receive(1, { type: 'displayRate', hz: 120 });
  draw.receive(1, { type: 'structure', updates: [] });
  draw.receive(1, { type: 'tableRows', id: 3, owner: 1, count: 2 });
  draw.receive(0, { type: 'displayRate', hz: 60 });
  assert.deepEqual(heard.map(message => message.type), ['tableRows', 'displayRate']);
});
