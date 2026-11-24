import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('table family plays live with zero React commits, zero layout reads on install, pause flush, independent hover/focus tooltips, drag-out-and-back cancel, keyboard mirror and sideways scroll when narrow', async t => {
  await withNativePage('table-family', '/ui/src/families/table/tableFamily.dom.fixture.tsx', page => page.stage('table-family', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.tableFamilyDom.${expression}`);
    const mounted = await act<{ painted: boolean; initialRows: (string | null)[] }>('mount()');
    assert.equal(mounted.painted, true, 'initial table frame painted');
    assert.deepEqual(mounted.initialRows, ['GER', 'FRA', 'ITA', 'ENG'], 'missing ENG sorts last initially');

    const played = await act<{
      playingDuringAdvance: boolean;
      playingBeforePause: boolean;
      beforePauseTop: string | null;
      pendingTimersBeforePause: number;
      pendingTimersAfterPause: number;
      timerFiredBeforePause: number;
      timerFiredAfterPause: number;
      commits: number;
      totalInstallReads: number;
      pauseInstallReads: number;
      installs: number;
      flushedOnPause: boolean;
      wantTop: string;
      finalHour: number | null;
      mirrorRows: { key: string | null; cells: (string | null)[] }[];
    }>('playLive(22)');
    t.diagnostic(`live playback: ${played.commits} commits, ${played.totalInstallReads} play reads, ${played.pauseInstallReads} pause reads, ${played.installs} installs, beforePauseTop=${played.beforePauseTop}, pendingBefore=${played.pendingTimersBeforePause}, pendingAfter=${played.pendingTimersAfterPause}, timerFired=${played.timerFiredBeforePause}/${played.timerFiredAfterPause}, flushedOnPause=${played.flushedOnPause}`);
    assert.equal(played.playingDuringAdvance, true, 'clock.playing was true during frame advance');
    assert.equal(played.playingBeforePause, true, 'clock.playing stayed true until pause');
    assert.equal(played.beforePauseTop, 'GER', 'S8: mirror was still throttled on GER before pause with controlled clock');
    assert.ok(played.pendingTimersBeforePause > 0, 'S8: trailing flush timer was armed and held before pause');
    assert.equal(played.timerFiredBeforePause, 0, 'S8: trailing timer did not fire before pause');
    assert.equal(played.timerFiredAfterPause, 0, 'S8: pause flushed the mirror without the trailing timer firing');
    assert.equal(played.pendingTimersAfterPause, 0, 'S8: pause cancelled the pending trailing timer');
    assert.equal(played.commits, 0, `zero React commits across live playback with unchanged structure: ${played.commits}`);
    assert.equal(played.totalInstallReads, 0, `zero layout/style reads during live playback install: ${played.totalInstallReads}`);
    assert.equal(played.pauseInstallReads, 0, `zero layout/style reads through pause flush: ${played.pauseInstallReads}`);
    assert.ok(played.installs >= 6, `frames actually installed while playing: ${played.installs}`);
    assert.equal(played.flushedOnPause, true, `mirror immediately flushed to ${played.wantTop} on pause`);
    assert.equal(played.mirrorRows.at(-1)?.key, 'ENG', 'notRecorded row stays last');
    assert.ok(played.mirrorRows.at(-1)?.cells.some(c => c?.includes('Not recorded: ENG factories not recorded')), 'mirror speaks missing reason');

    const heldCheck = await act<{ pixelsChanged: boolean; dim: boolean; held: boolean; recovered: boolean; through: number | null; expectedThrough: number }>('verifyRepaintAndHeld()');
    assert.equal(heldCheck.pixelsChanged, true, 'same-layout value change repaints canvas pixels');
    assert.equal(heldCheck.dim, true, 'clock past confirmed hour marks view dim');
    assert.equal(heldCheck.held, true, 'dim view marks frame data-held');
    assert.equal(heldCheck.recovered, true, 'dim clears once fresh hour arrives');
    assert.equal(heldCheck.through, heldCheck.expectedThrough);

    const { cdpSession } = await import(new URL('../../../../tools/e2e/cdp.mjs', import.meta.url).href) as {
      cdpSession(url: string): Promise<{ send(method: string, params?: Record<string, unknown>): Promise<unknown>; close(): void }>;
    };
    const cdp = await cdpSession(page.page);
    try {
      const box = await act<{ left: number; top: number }>('bodyBox()');
      const px = Math.round(box.left + 40);
      const pyRow0 = Math.round(box.top + 14);
      const pyRow1 = Math.round(box.top + 48);
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: px, y: pyRow0 });
      assert.equal(await act<boolean>('waitInspected("GER")'), true, 'hover inspects row GER');

      const reordered = await act<(string | null)[]>('resortUnderPointer()');
      assert.equal(reordered[0], 'FRA', 'FRA moved into row 0 beneath the still pointer');

      await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: px, y: pyRow0, button: 'left', clickCount: 1 });
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: px + 1, y: pyRow0, button: 'left', clickCount: 1 });
      const afterClick1 = await act<string[]>('readActivated()');

      await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: px, y: pyRow0, button: 'left', clickCount: 1 });
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: px + 12, y: pyRow0, button: 'left' });
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: px, y: pyRow0, button: 'left' });
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: px, y: pyRow0, button: 'left', clickCount: 1 });

      const clicked = await act<string[]>('readActivated()');
      assert.deepEqual(clicked, ['FRA'], `a click after a re-sort activates the row its own hit names in the installed picture (FRA), not the cached hover, and drag-out-and-back >4px cancels (afterClick1=${JSON.stringify(afterClick1)})`);

      const focused = await act<{ key: string; ready: boolean }>('focusFirstAndWaitTooltip()');
      assert.equal(focused.ready, true, 'focused row FRA receives its own focusTooltip');
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: px, y: pyRow1 });
      assert.equal(await act<boolean>('waitInspected("ITA")'), true, 'pointer hovers row 1 (ITA) while keyboard stays on FRA');
      const pinned = await act<{ title: string | null; subtitle: string | null; installReads: number }>('pinFocusedRow()');
      assert.equal(pinned.title, 'France', 'P pins keyboard-focused FRA (France), not pointer-hovered ITA (Italy)');
      assert.ok((pinned.subtitle ?? '').startsWith('Pinned at '), `pinned tooltip carries pinned subtitle: ${pinned.subtitle}`);
      assert.equal(pinned.installReads, 0, 'zero layout/style reads during install even with active focus and hover tooltips');

      const overflow = await act<{ tipOverflowed: boolean; commits: number; installReads: number; installs: number }>('testOverflowTooltipDuringPlayback()');
      assert.equal(overflow.tipOverflowed, true, 'M8: fitTooltip genuinely truncated the tooltip');
      assert.ok(overflow.installs >= 3, `M8: multiple installs occurred during playback (${overflow.installs})`);
      assert.equal(overflow.commits, 0, `M8: cached overflowing tooltip causes zero TooltipLayer commits across unchanged installs (got ${overflow.commits})`);
      assert.equal(overflow.installReads, 0, `M8: cached overflowing tooltip causes zero layout reads across unchanged installs (got ${overflow.installReads})`);
    } finally {
      cdp.close();
    }

    const kbd = await act<{ firstKey: string | null; afterDown: string | null; activated: string[]; liveStatus: string; focusReturned: boolean; scrollsSideways: boolean; totalInstallReads: number }>('testKeyboardAndNarrow()');
    assert.ok(kbd.afterDown !== null && kbd.afterDown !== kbd.firstKey, `S8: ArrowDown moved focus from ${kbd.firstKey} to ${kbd.afterDown}`);
    assert.equal(kbd.activated.length, 1, 'Enter activates focused row');
    assert.equal(kbd.focusReturned, true, 'closing a window opened from a row returns focus to that row by its key');
    assert.equal(kbd.scrollsSideways, true, 'a narrow table scrolls sideways and shows the right fade');

    const r5 = await act<{ opened: boolean; menuLabel: string | null; tipShown: boolean; hasMissingState: boolean; hasKnownState: boolean }>('testEmptySpaceContextMenuAndValueStateTooltip()');
    assert.equal(r5.opened, true, 'F9: right-clicking empty space with no inspected or focused row opens context menu');
    assert.ok((r5.menuLabel ?? '').includes('Jump to date (empty@'), `F9: menu item offered with null key on empty space: ${r5.menuLabel}`);
    assert.equal(r5.tipShown, true, 'F8: tooltip renders ValueState elements');
    assert.equal(r5.hasMissingState, true, 'F8: missing tooltip cell renders ValueState missing');
    assert.equal(r5.hasKnownState, true, 'F8: known tooltip cell renders ValueState known');
  }));
});
