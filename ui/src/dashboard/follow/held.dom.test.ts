import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

interface Shown { reading: string; dim: boolean; held: string | null; busy: string | null; tip: string | null; described: boolean; menus: number }
interface Interaction { hovered: Shown; after: Shown }
const show = (value: unknown) => JSON.stringify(value);

test('a paused step past the installed picture keeps it muted and busy, with no tooltip, description or menu of the hour it holds, until the step is read; a held seek reads, never outside', async () => {
  await withNativePage('held', '/ui/src/dashboard/follow/held.dom.fixture.tsx', page => page.stage('held', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.heldFixture.${expression}`);
    const started = await act<{ ok: boolean; shown: Shown }>('start()');
    assert.ok(started.ok, `the followed reading installs at 100: ${show(started.shown)}`);
    assert.deepEqual([started.shown.held, started.shown.busy], [null, null], 'a current picture is neither held nor busy');

    const current = await act<Interaction>('current()');
    assert.ok(current.hovered.tip === 'rows' && current.hovered.described, `a current row shows its tooltip and description: ${show(current.hovered)}`);
    assert.equal(current.after.menus, 2, 'a current row opens its menu by pointer and by keyboard');

    const stepped = await act<{ ok: boolean; hovered: Shown; focused: Shown; after: Shown }>('step()');
    assert.ok(stepped.ok, `a paused step to 101 that is not read yet dims the picture of 100: ${show(stepped.hovered)}`);
    assert.equal(stepped.hovered.reading, 'covered', 'the held picture stays on screen, not cleared and not outside the recording');
    assert.deepEqual([stepped.hovered.held, stepped.hovered.busy], ['stale', 'true'], 'the reader holding a dim picture marks its element muted and busy');
    for (const [name, state] of [['hovered', stepped.hovered], ['focused', stepped.focused]] as const)
      assert.ok(state.tip === null && !state.described, `a ${name} row of a held picture shows no tooltip or description of 100: ${show(state)}`);
    assert.equal(stepped.after.menus, 2, 'a held row opens no menu to act on 100, by pointer or keyboard');

    const released = await act<{ ok: boolean } & Interaction>('release()');
    assert.ok(released.ok, `once 101 is read the picture is current again: ${show(released.hovered)}`);
    assert.deepEqual([released.hovered.held, released.hovered.busy], [null, null], 'and neither muted nor busy');
    assert.ok(released.hovered.tip === 'rows' && released.hovered.described, `its row shows its tooltip and description again: ${show(released.hovered)}`);
    assert.equal(released.after.menus, 4, 'and opens its menu again');

    const sought = await act<{ reading: Shown; settled: Shown }>('heldRead(900)');
    assert.equal(sought.reading.reading, 'pending', `a seek to 900 not read yet shows the read in progress, not a time outside the recording: ${show(sought.reading)}`);
    assert.equal(sought.settled.reading, 'covered', 'and the reading installs when 900 arrives');
  }));
});
