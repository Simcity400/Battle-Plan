import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface LayoutMetrics {
  slotOutsideContent: boolean;
  articleHeight: number;
  headerHeight: number;
  captionHeight: number;
  contentHeight: number;
  bodyHeight: number;
  bodyClientWidth: number;
  bodyScrollHeight: number;
  bodyClientHeight: number;
  contentScrollHeight: number;
  contentClientHeight: number;
  scrollerScrollHeight: number;
  scrollerClientHeight: number;
  scrollerClientWidth: number;
}

test('a widget body with a caption lays its content out in the reduced height below header and caption without overflowing or adding a scrollbar', async () => {
  await withNativePage('widget-caption-layout', '/ui/src/dashboard/widgetCaption.dom.fixture.tsx', page => page.stage('caption layout', async () => {
    const plain = await page.evaluate<LayoutMetrics>('window.captionLayout.measure(null)');
    const captioned = await page.evaluate<LayoutMetrics>('window.captionLayout.measure("Share of divisions")');
    const bodyState = await page.evaluate<LayoutMetrics>('window.captionLayout.measure("Share of divisions", true)');

    assert.equal(plain.slotOutsideContent, true);
    assert.equal(captioned.slotOutsideContent, true);
    assert.equal(plain.captionHeight, 0);
    assert.ok(captioned.captionHeight > 0, 'caption takes positive height');
    assert.ok(Number.isInteger(captioned.captionHeight), `caption height ${captioned.captionHeight} is a whole pixel`);
    assert.equal(captioned.contentHeight, plain.contentHeight - captioned.captionHeight, 'content area shrinks by the exact caption height');
    assert.equal(captioned.bodyHeight, captioned.contentHeight, 'a height:100% body fills the reduced content area');
    assert.ok(captioned.contentScrollHeight <= captioned.contentClientHeight, 'content area does not overflow');
    assert.ok(captioned.scrollerScrollHeight <= captioned.scrollerClientHeight, 'self-sized scroller inside the body does not overflow');
    assert.equal(captioned.scrollerClientWidth, captioned.bodyClientWidth, 'no scrollbar narrows the chart body');
    assert.equal(bodyState.captionHeight, 0, 'caption slot hides when the body shows a state');
    assert.equal(bodyState.contentHeight, plain.contentHeight, 'body state reclaims the full content height');

    const chart = await page.evaluate<LayoutMetrics>('window.captionLayout.measure("Manpower lost per month", false, "chart")');
    assert.equal(chart.contentHeight, plain.contentHeight - chart.captionHeight, 'chart content area shrinks by caption height');
    assert.equal(chart.bodyHeight, chart.contentHeight, 'chart body fills reduced content area');
    assert.ok(chart.bodyScrollHeight <= chart.bodyClientHeight, 'chart body does not overflow');

    const table = await page.evaluate<LayoutMetrics>('window.captionLayout.measure("Equipment as of date", false, "table")');
    assert.equal(table.contentHeight, plain.contentHeight - table.captionHeight, 'table content area shrinks by caption height');
    assert.equal(table.bodyHeight, table.contentHeight, 'table body fills reduced content area');
    assert.ok(table.scrollerScrollHeight <= table.scrollerClientHeight, 'table does not overflow');

    const switched = await page.evaluate<{ switchClicks: string[]; previewHasDynamicSwitch: boolean }>('window.captionLayout.testDynamicSwitchFreshness()');
    assert.deepEqual(switched.switchClicks, ['v2:b'], 'useWidgetViewSwitch invokes the latest onChange callback after parent state changes');
    assert.equal(switched.previewHasDynamicSwitch, true, 'catalogue WidgetPreview passes setOption so dynamic view switches register');

    const headerDrop = await page.evaluate<{
      unloadedOscillations: number; triggerWidth: number; loadedStage: number; subjectCompact: boolean;
      stageBeforePlay: number; stageAfterContentGrowWhilePlaying: number; stageAfterPauseFlush: number;
      wideStage: number; stageAfterResizeWhilePlaying: number;
    }>('window.captionLayout.testHeaderDropStability()');
    assert.equal(headerDrop.unloadedOscillations, 0, 'absent subject slot converges without oscillating');
    assert.ok(headerDrop.triggerWidth > 120, `measured real compact view trigger width (${headerDrop.triggerWidth}px), not 84px fallback`);
    assert.ok(headerDrop.loadedStage >= 3 && headerDrop.subjectCompact, 'wide compact view trigger advances header drop to stage 3');
    assert.equal(headerDrop.stageBeforePlay, 0, 'short pill fits in stage 0 before playback');
    assert.equal(headerDrop.stageAfterContentGrowWhilePlaying, headerDrop.stageBeforePlay, 'content-only pill growth stays deferred while playing');
    assert.ok(headerDrop.stageAfterPauseFlush > headerDrop.stageBeforePlay, `deferred content re-measurement flushes once playback pauses (${headerDrop.stageBeforePlay} -> ${headerDrop.stageAfterPauseFlush})`);
    assert.ok(headerDrop.stageAfterResizeWhilePlaying > headerDrop.wideStage, 'container resize updates header drop stage immediately even while playing');

    const reading = await page.evaluate<{
      loadingRole: string | null; loadingAnnouncement: string; announcedDuringError: string; loadingAfterRetryError: string;
      badgeBeforeThreshold: string | null; badgeAfterRerender: string | null; badgeRole: string | null; badgeAnnouncement: string;
      behindBadge: string | null;
      firstEmptyAnnounced: string; clearedOnPicture: string; repeatedEmptyAnnounced: string;
      mapBadgeOutsideSurface: boolean; chooseButtonWhenUnloaded: boolean; chooseButtonWhenLoaded: boolean;
    }>('window.captionLayout.testReadingBadgeAndAnnouncer()');
    assert.equal(reading.loadingRole, null, 'loading state has no duplicate role="status"');
    assert.equal(reading.loadingAnnouncement, 'Loading probe…', 'loading announces via the single frame live region');
    assert.equal(reading.announcedDuringError, '', 'polite region clears when loading exits to error');
    assert.equal(reading.loadingAfterRetryError, 'Loading probe…', 'loading -> error -> Retry -> loading re-announces loading');
    assert.equal(reading.badgeBeforeThreshold, null, 'reading badge waits for the 400 ms boundary');
    assert.match(reading.badgeAfterRerender ?? '', /^Reading /, 'reading badge stays settled across default-campaign re-renders');
    assert.equal(reading.badgeRole, null, 'reading badge has no duplicate role="status"');
    assert.equal(reading.badgeAnnouncement, reading.badgeAfterRerender, 'reading badge announces via the single frame live region');
    assert.match(reading.behindBadge ?? '', /^Reading /, 'map data-held="behind" shows the reading badge during playback');
    assert.equal(reading.firstEmptyAnnounced, 'Nothing here yet.');
    assert.equal(reading.clearedOnPicture, '', 'returning to a picture clears the announcer state');
    assert.equal(reading.repeatedEmptyAnnounced, 'Nothing here yet.', 're-entering the same state announces it again');
    assert.equal(reading.mapBadgeOutsideSurface, true, 'map reading badge sits in visible frame space outside the native map surface');
    assert.equal(reading.chooseButtonWhenUnloaded, false, 'nothing-chosen omits Choose country when no campaign is loaded');
    assert.equal(reading.chooseButtonWhenLoaded, true, 'nothing-chosen offers Choose country when a campaign is loaded');

    const headRetry = await page.evaluate<{
      firstError: { kind: string; reason?: string; detail?: string } | null;
      callsBeforeRetry: number;
      callsAfterRetry: number;
    }>('window.captionLayout.testHeadProductFollowRetry()');
    assert.deepEqual(headRetry.firstError, { kind: 'retryable', reason: 'Desktop panel channel busy.', detail: 'panel follow 503' },
      'useHeadProduct preserves ShownError user reason and technical detail');
    assert.equal(headRetry.callsBeforeRetry, 1);
    assert.equal(headRetry.callsAfterRetry, 2, 'useHeadProduct.retry reissues the failed follow subscription while following');

    const popup = await page.evaluate<{ openBeforeEmpty: boolean; openAfterEmpty: boolean }>('window.captionLayout.testActiveResearchPopupOnEmpty()');
    assert.equal(popup.openBeforeEmpty, true);
    assert.equal(popup.openAfterEmpty, true, 'seeking to an hour with no active research slots keeps an open ResearchWindow mounted');
  }));
});
