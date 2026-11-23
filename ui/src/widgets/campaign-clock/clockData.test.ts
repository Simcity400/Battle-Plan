import { openGate } from '../../test-support/gated.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import { clockRequest } from '../../dashboard/widgetRequests.ts';
import { clockRows, clockStructure, clockWarnings, OVERLAP_REASON, OVERLAP_WARNING } from './clockData.ts';
import { CLOCK_COLUMNS, CLOCK_TABLE_BASE, campaignClockProjection } from './clockProjection.ts';
import { parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { buildColumnLayouts } from '../../families/table/layout.ts';
import { deriveTableAudit, resolveTableRows } from '../../families/table/model.ts';
import { tableSettingsOf } from '../../families/table/settings.ts';
import { pictureError } from '../../shell/perf.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';

test('the live campaign clock accumulates from the start instead of using a trailing week', () => {
  const track = { branch: { id: '4', firstHour: 100, lastHour: 1000 }, source: { id: 'source', revision: 'r1' } } as BoundTrack;
  const widget = { time: { mode: 'follow' } } as WidgetSpec;
  const request = clockRequest(openGate(100, 1000), widget, track, { firstHour: 733, lastHour: 900 });
  assert.deepEqual(request?.params, { branchId: '4', firstHour: 100, lastHour: 900 });
  assert.deepEqual(clockRequest(openGate(100, 1000), { ...widget, time: { mode: 'campaign' } }, track, { firstHour: 1000, lastHour: 1000 })?.params,
    { branchId: '4', firstHour: 100, lastHour: 1000 });
  assert.deepEqual(clockRequest(openGate(100, 1000), { ...widget, time: { mode: 'pin', windows: {} } }, track, { firstHour: 900, lastHour: 900 })?.params,
    { branchId: '4', firstHour: 100, lastHour: 900 });
  for (const time of [{ mode: 'follow' }, { mode: 'pin', windows: {} }] as const) for (const at of [99, 1001])
    assert.equal(clockRequest(openGate(100, 1000), { ...widget, time }, track, { firstHour: at, lastHour: at }), null, `a ${time.mode} moment at ${at} is outside the recording, never its edge`);
});

test('explicit periods clip to recorded branch hours and do not invent outside coverage', () => {
  const track = { branch: { id: '4', firstHour: 100, lastHour: 1000 }, source: { id: 'source', revision: 'r1' } } as BoundTrack;
  const widget = { time: { mode: 'range', firstOffset: 0, lastOffset: 1 } } as WidgetSpec;
  assert.deepEqual(clockRequest(openGate(100, 1000), widget, track, { firstHour: 500, lastHour: 1200 })?.params,
    { branchId: '4', firstHour: 500, lastHour: 1000 });
  assert.equal(clockRequest(openGate(100, 1000), widget, track, { firstHour: 1200, lastHour: 1300 }), null);
});

test('a followed head arriving complete and classified is a new structure, so its four rows are made', () => {
  const head = { recorded: 7200, playing: 5400, paused: 1800, complete: true };
  assert.notEqual(clockStructure(head), clockStructure(null));
  assert.notEqual(clockStructure({ reason: 'Not recorded at this hour' }), clockStructure(null));
  assert.notEqual(clockStructure(head), clockStructure({ ...head, complete: false }));
  assert.equal(clockStructure(head), clockStructure({ ...head, recorded: 9000 }));
  assert.deepEqual(clockRows(head, clockWarnings(head)).map(row => [row.name, row.seconds, row.text ?? null]), [
    ['Total recorded time', 7200, null], ['Playing time', 5400, null], ['Paused time', 1800, null], ['Coverage', null, 'Complete']]);
});

test('overlapping campaign-clock refusal carries the same warning on followed and exact paths', () => {
  const settings = tableSettingsOf(undefined, CLOCK_COLUMNS, CLOCK_TABLE_BASE);
  const cols = buildColumnLayouts(CLOCK_COLUMNS, settings);
  const exactReply = {
    available: true,
    recordedSeconds: null,
    playingSeconds: null,
    pausedSeconds: null,
    complete: false,
    warnings: [OVERLAP_WARNING],
  };
  const followedAnswer = parseFollowed(message(100, [{ id: 1, answer: { notRecorded: OVERLAP_REASON } }])).answers[0]!.answer;
  const exactModel = campaignClockProjection.project({ answers: new Map(), panels: new Map([['main', exactReply]]), live: new Map() }, 100, {});
  const followedModel = campaignClockProjection.project({ answers: new Map([['main', followedAnswer]]), panels: new Map(), live: new Map() }, 100, {});
  assert.deepEqual(followedModel.warnings, [OVERLAP_WARNING]);
  const exactAudit = deriveTableAudit({ model: exactModel, flatRows: resolveTableRows(exactModel, settings), visibleColumns: cols });
  const followedAudit = deriveTableAudit({ model: followedModel, flatRows: resolveTableRows(followedModel, settings), visibleColumns: cols });
  assert.equal(pictureError('table', followedAudit, { values: new Map(exactAudit.map(r => [r.key, r.value!])), ordered: true }), null);
});
