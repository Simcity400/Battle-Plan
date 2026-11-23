import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('log family plays live with zero React commits, zero layout reads on install, honest shownHour, pause flush without fetch, and scroll-back jump pill', async t => {
  await withNativePage('log-family', '/ui/src/families/log/logFamily.dom.fixture.tsx', async page => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.logFamilyDom.${expression}`);
    const mounted = await page.stage('mount', () => act<{
      painted: boolean;
      initialHour: number | null;
      afterFirstHour: number | null;
      expectedFirstHour: number;
      pillsText: string;
    }>('mount()'), 10_000);
    assert.equal(mounted.painted, true, 'initial log frame painted');
    assert.equal(mounted.initialHour, null, 'model hour is null before first paint');
    assert.equal(mounted.afterFirstHour, mounted.expectedFirstHour, 'model hour matches painted hour');
    assert.ok(mounted.pillsText.includes('Created 60'), `facet pills painted via DOM: ${mounted.pillsText}`);

    const played = await page.stage('playLive', () => act<{
      playingDuringAdvance: boolean;
      playingBeforePause: boolean;
      playCommits: number;
      playReads: number;
      pauseReads: number;
      playInstalls: number;
      scrollPageFetches: number;
      pillsAfterPlay: string;
      flushedOnPause: boolean;
      mirrorTopKey: string | null;
      expectedTopKey: string;
      finalHour: number | null;
      pausedHour: number;
    }>('playLive(12)'), 10_000);
    t.diagnostic(`log playback: ${played.playCommits} commits, ${played.playReads} play reads, ${played.pauseReads} pause reads, ${played.playInstalls} installs, scrollFetches=${played.scrollPageFetches}`);
    assert.equal(played.playingDuringAdvance, true, 'clock.playing was true during advance');
    assert.equal(played.playingBeforePause, true, 'clock.playing stayed true until pause');
    assert.equal(played.playCommits, 0, `zero React commits across live playback with changing rows and pills: ${played.playCommits}`);
    assert.equal(played.playReads, 0, `zero layout/style reads during live playback install: ${played.playReads}`);
    assert.equal(played.pauseReads, 0, `zero layout/style reads through pause flush: ${played.pauseReads}`);
    assert.ok(played.playInstalls >= 6, `frames actually installed while playing: ${played.playInstalls}`);
    assert.equal(played.scrollPageFetches, 0, 'pausing at top fetches nothing');
    assert.ok(played.pillsAfterPlay.includes('Created'), `DOM pills updated without React commits: ${played.pillsAfterPlay}`);
    assert.equal(played.flushedOnPause, true, `mirror flushed on pause (top=${played.mirrorTopKey}, want=${played.expectedTopKey})`);
    assert.equal(played.finalHour, played.pausedHour, 'final painted hour matches paused clock hour');

    const scroll = await page.stage('scrollBack', () => act<{
      jumpBefore: string;
      jumpAfter: string;
      scrollAfterJump: number;
    }>('testScrollBackAndJump()'), 10_000);
    assert.equal(scroll.jumpBefore, 'Jump to latest');
    assert.ok(scroll.jumpAfter.includes('new · Jump to latest'), `new count pill shown while scrolled away: ${scroll.jumpAfter}`);
    assert.equal(scroll.scrollAfterJump, 0, 'Jump to latest resets scrollTop to top');
  });
});
