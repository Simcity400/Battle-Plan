import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('log family plays market-ledger live with zero React commits through pause, zero layout reads on install, Enter menu, persistent chip toolbar, and scroll-back jump pill', async t => {
  await withNativePage('log-family', '/ui/src/families/log/logFamily.dom.fixture.tsx', async page => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.logFamilyDom.${expression}`);
    const mounted = await page.stage('mount', () => act<{
      painted: boolean;
      initialHour: number | null;
      afterFirstHour: number | null;
      expectedFirstHour: number;
      initialTotal: number;
    }>('mount()'), 10_000);
    assert.equal(mounted.painted, true, 'initial log frame painted');
    assert.equal(mounted.initialHour, null, 'model hour is null before first paint');
    assert.equal(mounted.afterFirstHour, mounted.expectedFirstHour, 'model hour matches painted hour');
    assert.equal(mounted.initialTotal, 120);

    const played = await page.stage('playLive', () => act<{
      playingDuringAdvance: boolean;
      playingBeforePause: boolean;
      commitsThroughPause: number;
      playReads: number;
      pauseReads: number;
      playInstalls: number;
      scrollPageFetches: number;
      gapsVisible: boolean;
      flushedOnPause: boolean;
      mirrorTopKey: string | null;
      expectedTopKey: string;
      finalTotal: number;
      expectedTotal: number;
      finalHour: number | null;
      pausedHour: number;
    }>('playLive(12)'), 10_000);
    t.diagnostic(`log playback: ${played.commitsThroughPause} commits through pause, ${played.playReads} play reads, ${played.pauseReads} pause reads, ${played.playInstalls} installs, total=${played.finalTotal}`);
    assert.equal(played.playingDuringAdvance, true, 'clock.playing was true during advance');
    assert.equal(played.playingBeforePause, true, 'clock.playing stayed true until pause');
    assert.equal(played.commitsThroughPause, 0, `F17: zero React commits through pause flush: ${played.commitsThroughPause}`);
    assert.equal(played.playReads, 0, `zero layout/style reads during live playback install: ${played.playReads}`);
    assert.equal(played.pauseReads, 0, `zero layout/style reads through pause flush: ${played.pauseReads}`);
    assert.ok(played.playInstalls >= 6, `frames actually installed while playing: ${played.playInstalls}`);
    assert.equal(played.scrollPageFetches, 0, 'pausing at top fetches nothing');
    assert.equal(played.gapsVisible, true, 'F17: ongoing coverage gap renders in imperative slot with zero React commits');
    assert.equal(played.finalTotal, played.expectedTotal, 'F17: total actually advanced during playback');
    assert.equal(played.flushedOnPause, true, `mirror flushed on pause (top=${played.mirrorTopKey}, want=${played.expectedTopKey})`);
    assert.equal(played.finalHour, played.pausedHour, 'final painted hour matches paused clock hour');

    const scroll = await page.stage('scrollBack', () => act<{
      jumpBefore: string;
      jumpAfter: string;
      jumpUpdateCommits: number;
      scrollAfterJump: number;
    }>('testScrollBackAndJump()'), 10_000);
    assert.equal(scroll.jumpBefore, 'Jump to latest');
    assert.ok(scroll.jumpAfter.includes('new · Jump to latest'), `new count pill shown while scrolled away: ${scroll.jumpAfter}`);
    assert.equal(scroll.jumpUpdateCommits, 0, 'S26: live new-row badge updates without React commits');
    assert.equal(scroll.scrollAfterJump, 0, 'Jump to latest resets scrollTop to top');

    const controls = await page.stage('enterAndChips', () => act<{
      hasContractAction: boolean;
      hasJumpAction: boolean;
      toolbarStillMounted: boolean;
      emptyVisible: boolean;
    }>('testEnterMenuAndLastChipToolbar()'), 10_000);
    assert.equal(controls.hasContractAction, true, 'F13: Enter opens row menu with Show contract');
    assert.equal(controls.hasJumpAction, true, 'F13: Enter opens row menu with Jump to hour');
    assert.equal(controls.toolbarStillMounted, true, 'F12: turning off last chip keeps toolbar mounted');
    assert.equal(controls.emptyVisible, true, 'F12: empty state renders inside body overlay');

    const drill = await page.stage('dualPlacement', () => act<{
      distinctPlacements: boolean;
      mainPipeId: number;
      drillPipeId: number;
      mainStillInstalled: boolean;
    }>('testDualPlacementAndDrillSurvival()'), 10_000);
    assert.equal(drill.distinctPlacements, true, 'S21: drill log under same FollowedWidgetKey gets distinct placement');
    assert.equal(drill.mainStillInstalled, true, 'S21: unmounting drill log leaves main log installing');
  });
});
