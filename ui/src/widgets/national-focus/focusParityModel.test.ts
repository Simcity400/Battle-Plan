import test from 'node:test';
import assert from 'node:assert/strict';
import { focusParityModel } from './focusParityModel.ts';
import type { Artwork, FocusOrder } from '../../generated/dashboard';

test('canonical model retains every referenced static field and every dated observation in order', () => {
  const metadata: Artwork[] = [
    { token: 'future', name: 'Future', description: 'Future details', baseDurationDays: 5, icon: null },
    { token: 'repeated', name: '', description: 'Static details', baseDurationDays: 7, icon: null },
    { token: 'active', name: 'active', description: null, baseDurationDays: 8, icon: null },
    { token: 'continuous', name: null, description: '', baseDurationDays: null, icon: null },
  ];
  const order: FocusOrder = { completed: [ { ordinal: 0, token: 'repeated', completedByHour: null },
    { ordinal: 1, token: 'repeated', completedByHour: 15 } ], current: 'active', continuous: 'continuous', paused: true };
  const calls: [string, number][] = [];
  const read = (token: string, hour: number) => { calls.push([token, hour]); return {
    artwork: { token, name: hour === 15 ? 'Scoped title' : '', description: hour === 15 ? 'Observed details' : 'Static details', baseDurationDays: 7, icon: 'data:image/png;base64,AA==' },
    pending: false, reason: hour === 15 ? 'Named refusal' : null, descriptionDemanded: hour === 15 }; };
  const model = focusParityModel(order, { progress: -9223372036854775808n, dailyGain: null }, 4, 20, true, metadata, read);
  assert.deepEqual(model.staticText, [metadata[2], metadata[3], metadata[1]]);
  assert.deepEqual(calls, [['repeated', 4], ['repeated', 15], ['active', 20]]);
  assert.deepEqual((model.artwork as any[])[1].art, read('repeated', 15));
  assert.equal((model.activeArt as any).artwork.icon, 'data:image/png;base64,AA==');
  assert.equal((model.current as any).progress.progress, -9223372036854775808n);
  const withOtherFuture = focusParityModel(order, null, 4, 20, true, [...metadata, { ...metadata[0], token: 'another_future' }], read);
  assert.deepEqual(withOtherFuture.staticText, model.staticText, 'only unreferenced future metadata is excluded');
});

test('presentation differences remain visible despite equal order and static metadata', () => {
  const order: FocusOrder = { completed: [{ ordinal: 0, token: 'mod', completedByHour: 7 }], current: null, continuous: null, paused: false };
  const at = (name: string) => () => ({ artwork: { token: 'mod', name, description: null, icon: null, baseDurationDays: null }, pending: false, reason: null, descriptionDemanded: false });
  assert.notDeepEqual(focusParityModel(order, null, 1, 7, false, [], at('A')), focusParityModel(order, null, 1, 7, false, [], at('B')));
});
