import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('shared family host handles F1 campaign switch pin clear, F2 leave-then-deactivate hover cleanup, F3/F4 theme & reduced motion, F5 press across frame update, F12 dual placement isolation + typeahead, and the generic FamilyHost doing no React render, bind or paint on quiet, steady and paused frames', async () => {
  await withNativePage('family-host', '/ui/src/families/host/host.dom.fixture.tsx', page => page.stage('family-host', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.hostDom.${expression}`);
    const { cdpSession } = await import(new URL('../../../../tools/e2e/cdp.mjs', import.meta.url).href) as {
      cdpSession(url: string): Promise<{ send(method: string, params?: Record<string, unknown>): Promise<unknown>; close(): void }>;
    };
    const cdp = await cdpSession(page.page);
    try {
      await cdp.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'no-preference' }] });
      const mounted = await act<{ pipeId: number }>('mountPrimary()');
      assert.ok(mounted.pipeId > 0);

      const f1 = await act<{ pinnedBefore: string | null; cleared: boolean }>('testF1PinClearsOnCampaignSwitch()');
      assert.equal(f1.pinnedBefore, 'Germany', 'F1: tooltip was pinned before campaign switch');
      assert.equal(f1.cleared, true, 'F1: switching campaign clears pinned tooltip even when host stays mounted');

      const box = await act<{ left: number; top: number }>('bodyBox()');
      const px = Math.round(box.left + 40);
      const py = Math.round(box.top + 14);
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: px, y: py });
      const f2 = await act<{ tipVisible: boolean; clearedOnDeactivate: boolean }>('testF2LeaveThenDeactivateClearsHover()');
      assert.equal(f2.tipVisible, true, 'F2: hover tooltip appeared before leave');
      assert.equal(f2.clearedOnDeactivate, true, 'F2: deactivating before 100ms leave timer clears tooltip without stranding');

      const f3f4 = await act<{ unrelatedCommits: number; unrelatedBinds: number; bindsAfterToggle: number; boundReduced: boolean }>('testF3UnrelatedAppearanceAndF4ReducedMotion()');
      assert.equal(f3f4.unrelatedCommits, 0, 'F3: unrelated appearance event causes 0 React commits');
      assert.equal(f3f4.unrelatedBinds, 0, 'F3: unrelated appearance event causes 0 pipeline binds');
      assert.equal(f3f4.boundReduced, true, 'F4: toggling app reduced motion while paused updates binding immediately');
      assert.ok(f3f4.bindsAfterToggle > 0, 'F4: reduced motion change triggers rebind');

      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: px, y: py });
      assert.equal(await act<boolean>('waitInspected("GER")'), true, 'F5: GER inspected before press');
      await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: px, y: py, button: 'left', clickCount: 1 });
      await act('testF5FrameUpdateDuringPress()');
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: px, y: py, button: 'left', clickCount: 1 });
      const f5Activated = await act<string[]>('waitActivated()');
      assert.deepEqual(f5Activated, ['GER'], 'F5: updating widget frame mid-press preserves pointerdown and activates row on release');
    } finally {
      cdp.close();
    }

    const generic = await act<{
      firstPicture: boolean;
      mountCommits: number;
      quiet: { hoursAdvanced: number; commits: number; binds: number; paints: number };
      steady: { commits: number; binds: number; paints: number };
      paused: { commits: number; binds: number; paints: number };
      activatedSubject: string | null;
      activatedMatchesInstalled: boolean;
    }>('testGenericHostFrames()');
    assert.equal(generic.firstPicture, true, 'generic host installs its first picture');
    assert.ok(generic.mountCommits > 0, 'the generic host render counter observes mounting');
    assert.ok(generic.quiet.hoursAdvanced > 0, 'quiet playback advanced the clock');
    assert.deepEqual([generic.quiet.commits, generic.quiet.binds, generic.quiet.paints], [0, 0, 0], 'quiet playback ticks: no React render, no bind, no paint');
    assert.ok(generic.steady.paints > 0, 'steady playback frames repaint in the lane');
    assert.deepEqual([generic.steady.commits, generic.steady.binds], [0, 0], 'steady playback frames: no React render, no bind');
    assert.deepEqual([generic.paused.commits, generic.paused.binds, generic.paused.paints], [0, 0, 0], 'paused host settles and sleeps');
    assert.equal(generic.activatedSubject, 'GER', 'Enter activates the focused row from the installed picture');
    assert.equal(generic.activatedMatchesInstalled, true, 'the action carries the installed picture id');

    const still = await act<{ shown: boolean; installs: number; storeUpdates: number; commits: number }>('testGenericStationaryTooltip()');
    assert.equal(still.shown, true, 'the focused row shows its tooltip');
    assert.ok(still.installs >= 20, `twenty same-hour installs happened (${still.installs})`);
    assert.deepEqual([still.storeUpdates, still.commits], [0, 0], `a stationary tooltip across installs: no tooltip-store update, no React render including TooltipLayer (${still.installs} installs)`);

    const statics = await act<{ rounds: number; commits: number; binds: number; staticsSupplied: number; generationKept: boolean; mode: string }>('testStaticPanelsReplaced()');
    assert.ok(!statics.mode.startsWith('failed'), `the static host draws (${statics.mode})`);
    assert.deepEqual([statics.binds, statics.generationKept], [0, true], `replacing only a mounted host's static panels ${statics.rounds} times binds nothing and keeps the binding generation (R4-S4: ${statics.binds} binds)`);
    assert.equal(statics.staticsSupplied, statics.rounds, 'each new panel set is one statics supply');
    assert.equal(statics.commits, statics.rounds, 'and costs one React commit, the one that passed the panels');

    const tuple = await act<{
      blockedMeasured: boolean;
      blockedShown: boolean;
      description: string | null;
      clearedDescription: boolean;
      dateOnly: boolean;
      activatedId: number | null;
      activatedHour: number | null;
      viewId: number | null;
      viewHour: number | null;
    }>('testGenericInstalledTuple()');
    assert.equal(tuple.blockedMeasured, true, 'a blocked host still measures its body, so a cold chart can ask for its first tiles');
    assert.equal(tuple.blockedShown, true, 'the blocked state shows inside the frame');
    assert.equal(tuple.description, 'Four countries ranked by factories', 'the installed description is what the body is described by');
    assert.equal(tuple.clearedDescription, true, 'the description goes when the picture goes');
    assert.equal(tuple.dateOnly, true, 'a date-only install happened without an install notification');
    assert.equal(tuple.activatedId, tuple.viewId, 'Enter after a date-only install carries the picture Page shows now');
    assert.equal(tuple.activatedHour, tuple.viewHour, 'and its hour');

    const f12 = await act<{
      distinctCanvases: boolean;
      distinctPlacements: boolean;
      hasReaderA: boolean;
      hasReaderB: boolean;
      primaryStillInstalls: boolean;
      typeaheadKey: string | null;
    }>('testF12DualPlacementAndTypeahead()');
    assert.equal(f12.distinctCanvases, true, 'F12: co-mounted hosts under one widget key own distinct canvases');
    assert.equal(f12.distinctPlacements, true, 'F12: co-mounted hosts under one widget key own distinct placement IDs');
    assert.equal(f12.hasReaderA, true, 'F12: primary audit reader registered under its own placement');
    assert.equal(f12.hasReaderB, true, 'F12: secondary audit reader registered under its own placement');
    assert.equal(f12.primaryStillInstalls, true, 'F12: unmounting secondary leaves primary installing new frames');
    assert.equal(f12.typeaheadKey, 'ITA', 'F12: table typeahead focuses matching country row');
  }));
});
