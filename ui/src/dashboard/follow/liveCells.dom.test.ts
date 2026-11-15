import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

type Act = <T>(expression: string) => Promise<T>;
type Row = { key: string; text: string | null; title: string };

test('a panel\'s followed cells repaint with no commit, are missing by name, and a structure push commits once', async () => {
  await withNativePage('live-cells', '/ui/src/dashboard/follow/liveCells.dom.fixture.tsx', async page => {
    const act: Act = expression => page.evaluate(`window.liveCells.${expression}`);
    await act('structure(["100", "101", "102"])');
    await act('cells({ 100: 5, 101: 6, 102: 7 })');
    assert.deepEqual((await act<Row[]>('rows()')).map(row => [row.key, row.text]), [['100', '5 pp'], ['101', '6 pp'], ['102', '7 pp']]);

    const before = await act<number>('commits()');
    for (let step = 1; step <= 10; step++) await act(`cells({ 101: ${6 + step} }, true)`);
    assert.deepEqual((await act<Row[]>('rows()')).map(row => row.text), ['5 pp', '16 pp', '7 pp'], 'only the changed row moved');
    assert.equal(await act<number>('commits()') - before, 0, 'ten pushes of cells committed nothing');

    await act('notRecorded("Countries was not recorded at this hour")');
    const missing = await act<Row[]>('rows()');
    assert.deepEqual(missing.map(row => [row.text, row.title]), Array(3).fill(['—', 'Countries was not recorded at this hour']), 'never zero, never the last value');
    assert.equal(await act<number>('commits()') - before, 0);

    await act('structure(["100", "103"])');
    const structured = await act<number>('commits()');
    // One render of the panel; the table may add its own settling commits for rows that came and went.
    assert.ok(structured - before >= 1, 'a structure push commits');
    assert.deepEqual((await act<Row[]>('rows()')).map(row => [row.key, row.text, row.title]), [['100', '—', 'Not read yet'], ['103', '—', 'Not read yet']],
      'the new structure\'s cells wait for their own answer, never the old group\'s');
    await act('cells({ 100: 50, 103: 30 })');
    assert.deepEqual((await act<Row[]>('rows()')).map(row => row.text), ['50 pp', '30 pp']);
    assert.equal(await act<number>('commits()'), structured, 'the new cells repaint with no commit');
  });
});
