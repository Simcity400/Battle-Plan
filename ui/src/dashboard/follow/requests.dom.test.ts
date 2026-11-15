import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('a followed widget makes no request when the hour changes, and the connection sends only the clock anchors', async t => {
  await withNativePage('follow-requests', '/ui/src/dashboard/follow/requests.dom.fixture.tsx', page => page.stage('requests', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.requestsFixture.${expression}`);
    const today = await act<number>('readsWhileTheHourMoves(false, 30)');
    const followed = await act<number>('readsWhileTheHourMoves(true, 30)');
    t.diagnostic(`series-state requests over 30 hour changes: today ${today}, followed ${followed}`);
    assert.ok(today >= 15, `today's path asks again as the hour moves (${today})`);
    assert.equal(followed, 0, 'a followed head is never asked for');
    const connection = await act<{ hours: number; follow: number; anchors: number; other: number }>('connectionWhileTheClockPlays(30)');
    t.diagnostic(`while playing ${connection.hours} hours: ${JSON.stringify(connection)}`);
    assert.ok(connection.hours > 24, 'the clock moved');
    assert.equal(connection.follow, 1);
    assert.equal(connection.other, 0);
    // The follow's own, speed, play and pause: one a change of anchor, none an hour.
    assert.ok(connection.anchors <= 4, `only anchors go, never an hour (${connection.anchors})`);
  }, 120_000));
});
