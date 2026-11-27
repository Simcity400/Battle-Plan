import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../../tools/native-page.ts';

interface Result {
  hour: number;
  before: number; count: number; activities: Record<string, unknown>[]; tails: Record<string, unknown>[]; groups: number;
  strip: string; statuses: string[]; partial: number; missing: number;
}

test('the activity request uses the family plan references without reading country groups again and keeps partial and missing holder bins', () =>
  withNativePage('activity-selection', '/ui/src/widgets/event-feed/activityStrip.dom.fixture.tsx', page => page.stage('plan references', async () => {
    const result = await page.evaluate<Result>('window.activityStripFixture.run({kind:"tags",tags:["AAA"]}, false)');
    assert.equal(result.count, 1);
    assert.deepEqual(result.activities[0]?.countries, [7, 'AAA']);
    assert.equal(result.groups, 0);
    assert.equal(result.partial, 1);
    assert.equal(result.missing, 1);
  })));

test('the real log family passes its resolved selection into the strip and names refused selections without requesting activity', () =>
  withNativePage('activity-family', '/ui/src/widgets/event-feed/activityStrip.dom.fixture.tsx', page => page.stage('family selection', async () => {
    const selected = await page.evaluate<Result>('window.activityStripFixture.run({kind:"tags",tags:["AAA"]}, true)');
    assert.deepEqual(selected.activities[0]?.countries, [7, 'AAA']);
    assert.deepEqual(selected.activities[0]?.countries, selected.tails[0]?.countries);
    for (const group of [{ type: 'humans' }, { type: 'faction', id: '31' }, { type: 'war', tag: 'CCC' }]) {
      for (const membership of ['hour', 'ever']) {
        const selection = { kind: 'group', group, membership, aggregate: false };
        const result = await page.evaluate<Result>(`window.activityStripFixture.run(${JSON.stringify(selection)}, true)`);
        assert.deepEqual(result.activities[0]?.group, { ...group, membership });
        assert.deepEqual(result.activities[0]?.group, result.tails[0]?.group);
      }
    }
    for (const selection of [
      { kind: 'group', group: { type: 'top', count: 3 }, membership: 'hour', aggregate: false },
      { kind: 'factions', count: 3, membership: 'hour' },
    ]) {
      const refused = await page.evaluate<Result>(`window.activityStripFixture.run(${JSON.stringify(selection)}, true)`);
      assert.equal(refused.count, 0);
      assert.ok(refused.strip?.trim(), 'the strip states the table refusal instead of showing no activity');
      assert.ok(refused.statuses.filter(reason => reason === refused.strip).length >= 2, JSON.stringify(refused.statuses));
    }
  })));

test('one followed log widget asks for activity at day boundaries over 1000 played hours', async t =>
  withNativePage('activity-count', '/ui/src/widgets/event-feed/activityStrip.dom.fixture.tsx', page => page.stage('1000 hours', async () => {
    const result = await page.evaluate<Result>('window.activityStripFixture.run({kind:"all"}, true, 1000)');
    t.diagnostic(`strip requests over 1000 played hours: ${result.count} (${result.before} initial)`);
    assert.equal(result.before, 1);
    assert.equal(result.hour, 1100);
    assert.equal(result.count, 42);
  }, 180_000)));

test('the family refreshes a followed group strip at unknown-membership boundaries inside a day', () =>
  withNativePage('activity-membership-gap', '/ui/src/widgets/event-feed/activityStrip.dom.fixture.tsx', page => page.stage('membership gap', async () => {
    for (const membership of ['hour', 'ever']) {
      const selection = { kind: 'group', group: { type: 'faction', id: '31' }, membership, aggregate: false };
      const result = await page.evaluate<Result>(`window.activityStripFixture.run(${JSON.stringify(selection)}, true, 16, true)`);
      assert.equal(result.hour, 116);
      assert.deepEqual(result.activities.map(params => params.hour), membership === 'hour' ? [100, 113, 116] : [100, 113]);
      assert.equal(result.partial, 1);
    }
  })));
