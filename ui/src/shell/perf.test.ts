import assert from 'node:assert/strict';
import test from 'node:test';
import { auditInBatches, pictureError } from './perf.ts';

test('audit work stays bounded while retaining every result in input order', async () => {
  let active = 0;
  let peak = 0;
  const visited: number[] = [];
  let started = 0, release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const result = await auditInBatches(Array.from({ length: 19 }, (_, i) => i), 3, async value => {
    active++;
    peak = Math.max(peak, active);
    if (++started === 3) release();
    visited.push(value);
    await gate;
    active--;
    return value * 2;
  });

  assert.equal(peak, 3, `audit only started ${peak} checks at once`);
  assert.deepEqual(result, Array.from({ length: 19 }, (_, i) => i * 2));
  assert.deepEqual([...visited].sort((a, b) => a - b), Array.from({ length: 19 }, (_, i) => i));
});

test('review 5 finding 18: a picture matches its exact read in displayed order, by row identity, and never as two empty lists', async () => {
  const { panelAuditRows } = await import('../dashboard/follow/panelAudit.ts');
  const shown = [{ row: 'B', key: 'B:v', value: '2' }, { row: 'A', key: 'A:v', value: '1' }];
  const exact = new Map([['A:v', '1'], ['B:v', '2']]);
  assert.equal(pictureError('table', shown, { values: exact }), null, 'unordered reads compare by key');
  assert.match(pictureError('table', shown, { values: exact, ordered: true }) ?? '', /order/, 'a table compares in the order it shows');
  const live = panelAuditRows('wargoals', { goals: [], justifications: [] }, new Map([['J1\u0000points', 5]]));
  assert.equal(pictureError('page', live, { values: new Map(live.map(row => [row.key, row.value])) }), null, 'live cells keep their identity');
  const owned = panelAuditRows('wargoals', { goals: [{ handle: 1, actor: 'GER', recipient: 'POL', name: 'Conquer', available: true, expires: null }], justifications: [] }, null);
  assert.equal(owned.length, 1, 'owned goals alone still give rows to compare');
});

test('review 6 D: a picture missing part of its exact read fails, as one goal of two does', () => {
  const exact = new Map([['["goal",1]', '["goal",1]'], ['["goal",2]', '["goal",2]']]);
  assert.match(pictureError('page', [{ row: '["goal",1]', key: '["goal",1]', value: '["goal",1]' }], { values: exact }) ?? '', /not shown/);
});

test('capturePicturesBatch freezes its snapshot on the first batch so post-window frames cannot extend the drain', async () => {
  const { auditing, capturePictures, capturePicturesBatch, recordPicture } = await import('../dashboard/pulled.ts');
  await capturePictures(true);
  assert.equal(auditing(), true);
  const sample = (at: number) => ({ t: 'picture', at, widget: 'w', surface: 'table', placement: 1, owner: 0, renderer: 'page', head: at, hour: at, offset: 0, since: at, through: at, eligibleHour: at, entry: at, parts: [], face: '', drawMs: 0, mode: 'picture', dim: false, reset: 0, binding: 0, inspect: null, tooltip: false }) as never;
  recordPicture(sample(1));
  recordPicture(sample(2));
  recordPicture(sample(3));
  const first = await capturePicturesBatch(2);
  assert.equal(auditing(), false);
  assert.equal(first.done, false);
  assert.deepEqual(first.records.map(r => r.at), [1, 2]);
  recordPicture(sample(4));
  const second = await capturePicturesBatch(2);
  assert.equal(second.done, true);
  assert.deepEqual(second.records.map(r => r.at), [3]);
});
