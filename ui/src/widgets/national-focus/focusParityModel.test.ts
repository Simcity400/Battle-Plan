import test from 'node:test';
import assert from 'node:assert/strict';
import { focusParityModel } from './focusParityModel.ts';
import type { Artwork, FocusDatabase, FocusOrder } from '../../generated/dashboard';

test('canonical model retains every referenced static field and every dated observation in order', () => {
  const metadata: Artwork[] = [
    { token: 'future', name: 'Future', description: 'Future details', baseDurationDays: 5, icon: null },
    { token: 'repeated', name: '', description: 'Static details', baseDurationDays: 7, icon: null },
    { token: 'active', name: 'active', description: null, baseDurationDays: 8, icon: null },
  ];
  const continuousMetadata: Artwork[] = [
    { token: 'continuous', name: null, description: '', baseDurationDays: null, icon: null },
    { token: 'other_continuous', name: 'Other', description: null, baseDurationDays: null, icon: null },
  ];
  const order: FocusOrder = { completed: [ { ordinal: 0, token: 'repeated', completedByHour: null },
    { ordinal: 1, token: 'repeated', completedByHour: 15 } ], current: 'active', continuous: 'continuous', paused: true };
  const calls: [FocusDatabase, string, number][] = [];
  const read = (database: FocusDatabase, token: string, hour: number) => { calls.push([database, token, hour]); return {
    artwork: { token, name: hour === 15 ? 'Scoped title' : '', description: hour === 15 ? 'Observed details' : 'Static details', baseDurationDays: 7, icon: 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png' },
    pending: false, reason: hour === 15 ? 'Named refusal' : null, descriptionDemanded: hour === 15 }; };
  const model = focusParityModel(order, { progress: -9223372036854775808n, dailyGain: null }, 4, 20, true, metadata, continuousMetadata, read);
  assert.deepEqual(model.staticText, [metadata[2], metadata[1]]);
  assert.deepEqual(model.continuousStaticText, [continuousMetadata[0]]);
  assert.deepEqual(calls, [['national', 'repeated', 4], ['national', 'repeated', 15], ['national', 'active', 20]]);
  assert.deepEqual((model.artwork as any[])[1].art, read('national', 'repeated', 15));
  assert.equal((model.activeArt as any).artwork.icon, 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png');
  assert.equal((model.current as any).progress.progress, -9223372036854775808n);
  const withOtherFuture = focusParityModel(order, null, 4, 20, true, [...metadata, { ...metadata[0], token: 'another_future' }], continuousMetadata, read);
  assert.deepEqual(withOtherFuture.staticText, model.staticText, 'only unreferenced future metadata is excluded');
});

test('the active continuous focus is read from its own database, apart from a national focus of the same id', () => {
  const order: FocusOrder = { completed: [{ ordinal: 0, token: 'shared', completedByHour: 20 }], current: null, continuous: 'shared', paused: false };
  const national: Artwork = { token: 'shared', name: 'National title', description: null, baseDurationDays: 7, icon: null };
  const continuous: Artwork = { token: 'shared', name: 'Continuous title', description: null, baseDurationDays: null, icon: null };
  const read = (database: FocusDatabase) => ({ artwork: database === 'national' ? national : continuous, pending: false, reason: null, descriptionDemanded: false });
  const model = focusParityModel(order, null, 10, 20, true, [national], [continuous], read);
  assert.deepEqual(model.staticText, [national]);
  assert.deepEqual(model.continuousStaticText, [continuous]);
  assert.equal((model.artwork as any[])[0].art.artwork.name, 'National title');
  assert.equal((model.activeArt as any).artwork.name, 'Continuous title');
});

test('presentation differences remain visible despite equal order and static metadata', () => {
  const order: FocusOrder = { completed: [{ ordinal: 0, token: 'mod', completedByHour: 7 }], current: null, continuous: null, paused: false };
  const at = (name: string) => () => ({ artwork: { token: 'mod', name, description: null, icon: null, baseDurationDays: null }, pending: false, reason: null, descriptionDemanded: false });
  assert.notDeepEqual(focusParityModel(order, null, 1, 7, false, [], [], at('A')), focusParityModel(order, null, 1, 7, false, [], [], at('B')));
});
