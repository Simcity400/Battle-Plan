import assert from 'node:assert/strict';
import test from 'node:test';
import { campaignToggled, PickerOptions, toggleMapSetting } from './mapPicker.ts';
import type { MapOptions } from '../shell/api';
import type { DashboardClient, DashboardMapView, MapSettingsReply } from './client';

test('a picker choice keeps the map mode and slots, and the fog switch still decides fog', () => {
  const maps = new PickerOptions();
  maps.changed({ viewId: 'm', options: { country: null, fog: false, mapMode: 'strategic_air', mapModeSlots: ['states'] } });
  const shown = maps.merge('m', { country: 'GER', fog: true });
  assert.deepEqual(shown, { country: 'GER', fog: true, mapMode: 'strategic_air', mapModeSlots: ['states'] });
  // Turning fog off in the map settings reaches the map after a picker choice.
  assert.equal(maps.merge('m', { country: 'GER', fog: false })?.fog, false);
});

test('fog clicks flip the view switch in order: two give the original, three the opposite', () => {
  const maps = new PickerOptions();
  let fog = true;
  const stop = maps.onFog('m', () => { fog = !fog; });
  // Two clicks before any echo: the switch is back where it was.
  assert.ok(maps.toggled('m', { setting: 'fog' }));
  assert.ok(maps.toggled('m', { setting: 'fog' }));
  assert.equal(fog, true);
  // A third flips it.
  maps.toggled('m', { setting: 'fog' });
  assert.equal(fog, false);
  // Another view's click and day/night (the host's settings) leave it alone.
  maps.toggled('other', { setting: 'fog' });
  assert.ok(!maps.toggled('m', { setting: 'dayNight' }));
  // An external change shows at once, and the next click flips that value.
  fog = true;
  maps.toggled('m', { setting: 'fog' });
  assert.equal(fog, false);
  stop();
  maps.toggled('m', { setting: 'fog' });
  assert.equal(fog, false);
});

test('the campaign map and the pop-out flip the one campaign fog switch, each click in order', () => {
  let options: MapOptions = { country: null, fog: true };
  // React applies each queued update to the state the previous one left.
  const update = (flip: (options: MapOptions) => MapOptions) => { options = flip(options); };
  // A click on the campaign map, then one on the pop-out, both under campaign-map.
  campaignToggled({ viewId: 'campaign-map', toggle: { setting: 'fog' } }, update, {} as DashboardClient, null);
  campaignToggled({ viewId: 'campaign-map', toggle: { setting: 'fog' } }, update, {} as DashboardClient, null);
  assert.equal(options.fog, true);
  campaignToggled({ viewId: 'campaign-map', toggle: { setting: 'fog' } }, update, {} as DashboardClient, null);
  assert.equal(options.fog, false);
  // A widget's click is its own view's; graphics clicks need a loaded campaign.
  campaignToggled({ viewId: 'w', toggle: { setting: 'fog' } }, update, {} as DashboardClient, null);
  campaignToggled({ viewId: 'campaign-map', toggle: { setting: 'dayNight' } }, update, {} as DashboardClient, null);
  assert.deepEqual(options, { country: null, fog: false });
});

test('setting clicks ask the service to flip, in click order, so a reset meanwhile is what they flip', async () => {
  // The service's store: a flip reads the value when it runs and writes only that choice.
  let stored: { graphics: { dayNight: boolean | null; borders: boolean | null }; audio: { voice: number | null } } =
    { graphics: { dayNight: null, borders: false }, audio: { voice: 40 } };
  const inherited = true;
  const sent: string[] = [];
  let release: () => void = () => undefined;
  let started: () => void = () => undefined;
  const writing = new Promise<void>(resolve => { started = resolve; });
  let paused = true;
  const client = {
    async flipMapSetting(_source: string, _revision: string, setting: 'dayNight' | 'alliedPlans', _signal: AbortSignal, target: unknown) {
      sent.push(`${setting} ${JSON.stringify(target)}`);
      started();
      if (paused) await new Promise<void>((resolve) => { release = resolve; });
      const current = stored.graphics.dayNight ?? inherited;
      stored = { ...stored, graphics: { ...stored.graphics, dayNight: !current } };
      return {} as MapSettingsReply;
    },
    async mapSettings() { throw new Error('a picker click never writes the whole settings'); },
  } as unknown as DashboardClient;
  const view = { id: 'w', sourceId: 's', revision: 'r' } as DashboardMapView;
  // A flip is held; a reset saves meanwhile, clearing the widget's overrides.
  const flip = toggleMapSetting(client, view, { setting: 'dayNight' });
  await writing;
  stored = { graphics: { dayNight: null, borders: null }, audio: { voice: 40 } };
  paused = false;
  release();
  await flip;
  // The flip applied to the value after the reset, and the reset's borders and the audio stand.
  assert.deepEqual(stored, { graphics: { dayNight: false, borders: null }, audio: { voice: 40 } });
  // Quick clicks are flips sent one after another: two give the value they started from.
  await Promise.all([
    toggleMapSetting(client, view, { setting: 'dayNight' }),
    toggleMapSetting(client, view, { setting: 'dayNight' }),
  ]);
  assert.equal(stored.graphics.dayNight, false);
  assert.deepEqual(sent, Array(3).fill('dayNight {"scope":"widget","id":"w"}'));
  // Fog is the view's own switch, never a settings flip.
  await toggleMapSetting(client, view, { setting: 'fog' });
  assert.equal(sent.length, 3);
});


test('campaign and pop-out graphics clicks flip campaign settings in order and preserve other choices', async () => {
  const source = { id: 'campaign-source', revision: 'campaign-revision' };
  const sent: { sourceId: string; revision: string; setting: string; target: unknown }[] = [];
  const stored = { dayNight: true, showAlliedPlans: false, borders: false, voice: 40 };
  let started: () => void = () => undefined;
  let release: () => void = () => undefined;
  const writing = new Promise<void>(resolve => { started = resolve; });
  const held = new Promise<void>(resolve => { release = resolve; });
  const client = {
    async flipMapSetting(sourceId: string, revision: string, setting: 'dayNight' | 'alliedPlans', _signal: AbortSignal, target: unknown) {
      sent.push({ sourceId, revision, setting, target });
      if (sent.length === 1) { started(); await held; }
      if (setting === 'dayNight') stored.dayNight = !stored.dayNight;
      else stored.showAlliedPlans = !stored.showAlliedPlans;
      return {} as MapSettingsReply;
    },
    async mapSettings() { throw new Error('a picker click never writes the whole settings'); },
  } as unknown as DashboardClient;
  const update = () => { throw new Error('graphics clicks must not change fog'); };
  const click = (setting: 'dayNight' | 'alliedPlans') => campaignToggled({ viewId: 'campaign-map', toggle: { setting } }, update, client, source);
  const first = click('dayNight');
  await writing;
  const second = click('alliedPlans');
  const third = click('dayNight');
  assert.equal(sent.length, 1, 'later clicks wait for the first host write');
  release();
  await Promise.all([first, second, third]);
  assert.deepEqual(sent, ['dayNight', 'alliedPlans', 'dayNight'].map(setting => ({ sourceId: source.id, revision: source.revision, setting, target: { scope: 'campaign' } })));
  assert.deepEqual(stored, { dayNight: true, showAlliedPlans: true, borders: false, voice: 40 });
  await campaignToggled({ viewId: 'widget', toggle: { setting: 'dayNight' } }, update, client, source);
  await campaignToggled({ viewId: 'campaign-map', toggle: { setting: 'dayNight' } }, update, client, null);
  assert.equal(sent.length, 3, 'unrelated views and unloaded campaigns write nothing');
});

test('a failed campaign graphics flip does not discard the next click', async () => {
  let writes = 0;
  const client = { async flipMapSetting() { if (++writes === 1) throw new Error('save failed'); return {} as MapSettingsReply; } } as unknown as DashboardClient;
  const source = { id: 'retry-source', revision: 'r' };
  const click = () => campaignToggled({ viewId: 'campaign-map', toggle: { setting: 'dayNight' } }, () => undefined, client, source);
  const first = click();
  const next = click();
  await assert.rejects(first, /save failed/);
  await next;
  assert.equal(writes, 2);
});
