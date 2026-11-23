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
  }));
});
