import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

interface State { mode: string; h?: number; through?: number; dim: boolean; mirror: string[]; held: boolean }

test('a production table on real lanes: a same-layout value change repaints, its pixels, mirror and inspection agree with a fresh table, and limited and missing readings are named', async () => {
  await withNativePage('table', '/ui/src/draw/tables/table.dom.fixture.tsx', page => page.stage('table', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.familyTableFixture.${expression}`);
    type Tip = { hour: number; values: string[] } | null;
    const started = await act<{ ok: boolean; state: State; pixels: string; tooltip: Tip }>('start()');
    assert.ok(started.ok, JSON.stringify(started.state));
    assert.deepEqual(started.state.mirror, ['10', '5'], 'the accessibility mirror shows the installed values');
    assert.deepEqual(started.tooltip, { hour: 10, values: ['10'] }, 'inspection reads the installed picture');
    const changed = await act<{ a: State; b: State; changed: string; fresh: string; tooltip: Tip }>('change()');
    assert.deepEqual([changed.a.h, changed.a.mirror], [11, ['20', '5']], 'the same layout with a new value installs the new value in the mirror');
    assert.notEqual(changed.changed, started.pixels, 'astra 1: the pixels changed, not kept as the same content');
    assert.equal(changed.changed, changed.fresh, 'the repainted table is pixel for pixel a fresh table of the same values');
    assert.deepEqual(changed.tooltip, { hour: 11, values: ['20'] }, 'inspection follows the new picture');
    const limited = await act<State>('limited()');
    assert.deepEqual([limited.through, limited.dim, limited.held], [12, true, true], 'confirmed only through 12: dated so and held');
    const missing = await act<State>('missing()');
    assert.deepEqual(missing.mirror, ['20', 'ITA was not recorded at this hour'], 'a missing value shows its reason, never zero');
  }));
});
