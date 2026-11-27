import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

test('the event detail distinguishes a missing firing and an observed choice from a recorded firing', () =>
  withNativePage('event-detail', '/ui/src/widgets/event-choices/EventDetailWindow.dom.fixture.tsx', page => page.stage('event timing', async () => {
    const orphan = await page.evaluate<string>('window.eventDetail.show(null)');
    assert.match(orphan, /Firing date not recorded/);
    assert.match(orphan, /event firing was not recorded/i);
    assert.match(orphan, /Chosen by /);
    assert.doesNotMatch(orphan, /Fired /);
    const unidentified = await page.evaluate<string>('window.eventDetail.show(null, null)');
    assert.match(unidentified, /Chosen by /);
    const recorded = await page.evaluate<string>('window.eventDetail.show(119)');
    assert.match(recorded, /Fired /);
    assert.doesNotMatch(recorded, /Firing date not recorded|Chosen by /);
  })));
