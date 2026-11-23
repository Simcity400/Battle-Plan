import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('shared family host handles F1 campaign switch pin clear, F2 leave-then-deactivate hover cleanup, F3/F4 theme & reduced motion, F5 press across frame update, and F12 dual placement isolation + typeahead', async () => {
  await withNativePage('family-host', '/ui/src/families/host/host.dom.fixture.tsx', page => page.stage('family-host', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.hostDom.${expression}`);
    const mounted = await act<{ pipeId: number }>('mountPrimary()');
    assert.ok(mounted.pipeId > 0);

    const f1 = await act<{ pinnedBefore: string | null; cleared: boolean }>('testF1PinClearsOnCampaignSwitch()');
    assert.equal(f1.pinnedBefore, 'Germany', 'F1: tooltip was pinned before campaign switch');
    assert.equal(f1.cleared, true, 'F1: switching campaign clears pinned tooltip even when host stays mounted');

    const { cdpSession } = await import(new URL('../../../../tools/e2e/cdp.mjs', import.meta.url).href) as {
      cdpSession(url: string): Promise<{ send(method: string, params?: Record<string, unknown>): Promise<unknown>; close(): void }>;
    };
    const cdp = await cdpSession(page.page);
    try {
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
