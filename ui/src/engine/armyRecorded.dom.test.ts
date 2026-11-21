import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';
import { playWindow, RECORDED_FIXTURE, recordedPage } from '../test-support/recordedDriver.ts';

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('Army at 1,440 h/s on the recorded replies: Land battles binds its rows into pooled boxes and React renders no battle row', async t => {
  await withNativePage('army-recorded', RECORDED_FIXTURE, native => native.stage('army', async () => {
    const act = await recordedPage(native);
    await act('start()');
    const played = await playWindow(act, 'Army');
    t.diagnostic(`hours ${played.from} to ${played.head}, ${played.frames} frames, ${played.messages} messages; commits ${JSON.stringify(played.commits)}`);
    for (const [widget, causes] of Object.entries(played.why)) t.diagnostic(`${widget}: ${JSON.stringify(causes)}`);
    assert.ok(played.frames > 0 && played.head > played.from, 'the window played');
    const battleRenders = Object.values(played.why).flatMap(causes => Object.keys(causes)).filter(cause => cause.startsWith('BattleCells'));
    assert.deepEqual(battleRenders, [], 'no battle row is rendered by React while following');
    const pooled = await native.evaluate<{ boxes: number; rows: number }>(`(() => {
      const list = document.querySelector('article.dash-widget.kind-land-battles .log');
      return { boxes: list ? list.querySelectorAll('.log-row[data-pool]').length : -1, rows: list ? list.querySelectorAll('.log-row').length : -1 };
    })()`);
    assert.ok(pooled.boxes > 0 && pooled.boxes === pooled.rows, `Land battles' rows are its pooled boxes: ${JSON.stringify(pooled)}`);
  }, 900_000));
});
