import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Rebound {
  shown: boolean; secondCommit: number | null; hiddenAtOnce: boolean; sameCanvas: boolean; keptCanvas: boolean; canvases: number; heldSurfaces: number; oldOwnerPresents: number;
  hiddenAfterOldOwner: boolean; revealed: boolean; newOwnerPresents: number; fixed: number | null;
}
interface Due { dueFrame: number | null; reads: { text: string; frame: number | null; phase: string | null }[] }

test('a drawn panel keeps its canvas when its pinned hour changes in the commit after its first, hides it in that commit and shows it again for the new hour alone; a frame listener renders before that frame reads geometry', async t => {
  await withNativePage('drawn-panel', '/ui/src/dashboard/drawnPanel.dom.fixture.tsx', page => page.stage('drawn panel', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.drawnPanel.${expression}`);
    const rebound = await act<Rebound>('rebind(30, 54)');
    t.diagnostic(`pinned 30 then 54: ${JSON.stringify(rebound)}`);
    assert.ok(rebound.shown, 'the panel pinned at hour 30 is shown after one commit');
    assert.equal(rebound.secondCommit, 2, 'pinning hour 54 is the second commit');
    assert.ok(rebound.hiddenAtOnce, 'the commit that pins hour 54 hides the hour-30 picture');
    assert.ok(rebound.sameCanvas && rebound.canvases === 1, 'the panel keeps its one canvas');
    assert.deepEqual([rebound.heldSurfaces, rebound.fixed], [1, 54], 'the new binding goes to the worker as one surface message, held back here');
    assert.ok(rebound.oldOwnerPresents > 0, 'meanwhile the worker presents the hour-30 picture again');
    assert.ok(rebound.hiddenAfterOldOwner, 'and that present does not show it');
    assert.ok(rebound.revealed && rebound.newOwnerPresents > 0 && rebound.keptCanvas, 'the worker presenting hour 54 shows that same canvas again');
    const due = await act<Due>('due(200)');
    t.diagnostic(`frame listener due at hour 200: ${JSON.stringify(due)}`);
    assert.notEqual(due.dueFrame, null, 'the clock reached hour 200 in a frame');
    assert.deepEqual(due.reads.map(read => [read.text, read.frame === due.dueFrame, read.phase]), [['due', true, 'after']],
      'the render a frame listener asks for commits in that frame, and the frame reads geometry once, after it');
  }, 120_000));
});
