import assert from 'node:assert/strict';
import test from 'node:test';
import { validateFocusReply } from './focusData.ts';
import type { FocusReply } from '../../generated/dashboard';

const first = { completed: [{ ordinal: 0, token: 'one', completedByHour: null }], current: 'two', continuous: null, paused: false };
const reply: FocusReply = { history: { segments: [
  { firstHour: 100, lastHour: 123, order: first },
  { firstHour: 125, lastHour: 200, order: { ...first, completed: [...first.completed, { ordinal: 1, token: 'two', completedByHour: 125 }], current: null } },
], progress: [
  { firstHour: 100, lastHour: 110, progressRaw: '6800000' },
  { firstHour: 111, lastHour: 123, progressRaw: '6900000' },
  { firstHour: 125, lastHour: 140, progressRaw: '100000' },
  { firstHour: 141, lastHour: 160, progressRaw: '200000' },
] }, missing: [{ firstHour: 124, lastHour: 124, reason: 'No focus row was recorded for this country' }], artwork: [], continuousStaticText: [], warnings: [] };

test('focus histories reject overlap, future completion bounds and invalid artwork', () => {
  assert.equal(validateFocusReply(reply), reply);
  const invalid = (firstHour: number, lastHour: number, ordinal: number, completedByHour: number | null) => ({ ...reply, history: { progress: [], segments: [{ firstHour, lastHour, order: { ...first, completed: [{ ordinal, token: 'one', completedByHour }] } }] } });
  for (const bad of [invalid(100, 99, 0, null), invalid(100, 100, 3, null), invalid(100, 100, 0, 101), invalid(NaN, 100, 0, null)]) assert.throws(() => validateFocusReply(bad));
  for (const missing of [undefined, [{ firstHour: 5, lastHour: 4, reason: 'x' }], [{ firstHour: 1, lastHour: 2, reason: 3 }], [{ firstHour: 1, lastHour: 4, reason: 'x' }, { firstHour: 3, lastHour: 5, reason: 'y' }]])
    assert.throws(() => validateFocusReply({ ...reply, missing } as unknown as FocusReply));
  assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, segments: [...reply.history.segments, reply.history.segments[0]] } }));
  for (const progressRaw of ['1.5', '', '99999999999999999999']) assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, progress: [{ firstHour: 1, lastHour: 2, progressRaw }] } }));
  assert.throws(() => validateFocusReply({ ...reply, history: { ...reply.history, progress: [...reply.history.progress, reply.history.progress[0]] } }));
  const art = { token: 'one', name: 'One', description: 'Game description', baseDurationDays: 70, icon: null };
  assert.equal(validateFocusReply({ ...reply, artwork: [art] }).artwork[0].baseDurationDays, 70);
  for (const baseDurationDays of [-1, NaN, Infinity]) assert.throws(() => validateFocusReply({ ...reply, artwork: [{ ...art, baseDurationDays }] }));
  for (const icon of ['https://example.com/remote.png', 'data:text/html;base64,YQ==', 'data:image/png;base64,YQ==', `/api/artwork/../${'a'.repeat(32)}.png`]) {
    assert.equal(validateFocusReply(structuredClone({ ...reply, artwork: [{ ...art, icon }] })).artwork[0].icon, null);
  }
  for (const bad of [null, 'one', {}, { ...art, token: 7 }]) {
    assert.throws(() => validateFocusReply({ ...reply, artwork: [bad] } as unknown as FocusReply));
  }
  assert.equal(validateFocusReply({ ...reply, continuousStaticText: [art] }).continuousStaticText[0].name, 'One');
  for (const continuousStaticText of [undefined, [{ ...art, icon: 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png' }], [null]]) {
    assert.throws(() => validateFocusReply({ ...reply, continuousStaticText } as unknown as FocusReply));
  }
});

test('duration metadata retains 5000 modded tokens without a chosen cap', () => {
  const artwork = Array.from({ length: 5000 }, (_, index) => ({ token: `synthetic_focus_${index}`, name: null, description: null, baseDurationDays: 10, icon: null }));
  assert.equal(validateFocusReply({ ...reply, artwork }).artwork.length, artwork.length);
});
