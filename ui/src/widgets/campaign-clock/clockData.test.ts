import test from 'node:test';
import assert from 'node:assert/strict';
import { clockRequest } from '../../dashboard/widgetRequests.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';

test('the live campaign clock accumulates from the start instead of using a trailing week', () => {
  const track = { branch: { id: '4', firstHour: 100, lastHour: 1000 }, source: { id: 'source', revision: 'r1' } } as BoundTrack;
  const widget = { time: { mode: 'follow' } } as WidgetSpec;
  const request = clockRequest(widget, track, { firstHour: 733, lastHour: 900 });
  assert.deepEqual(request?.params, { branchId: '4', firstHour: 100, lastHour: 900 });
  assert.deepEqual(clockRequest({ ...widget, time: { mode: 'campaign' } }, track, { firstHour: 1000, lastHour: 1000 })?.params,
    { branchId: '4', firstHour: 100, lastHour: 1000 });
  assert.deepEqual(clockRequest({ ...widget, time: { mode: 'pin', windows: {} } }, track, { firstHour: 900, lastHour: 900 })?.params,
    { branchId: '4', firstHour: 100, lastHour: 900 });
});

test('explicit periods clip to recorded branch hours and do not invent outside coverage', () => {
  const track = { branch: { id: '4', firstHour: 100, lastHour: 1000 }, source: { id: 'source', revision: 'r1' } } as BoundTrack;
  const widget = { time: { mode: 'range', firstOffset: 0, lastOffset: 1 } } as WidgetSpec;
  assert.deepEqual(clockRequest(widget, track, { firstHour: 500, lastHour: 1200 })?.params,
    { branchId: '4', firstHour: 500, lastHour: 1000 });
  assert.equal(clockRequest(widget, track, { firstHour: 1200, lastHour: 1300 }), null);
});
