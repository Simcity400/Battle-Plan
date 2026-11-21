import assert from 'node:assert/strict';
import test from 'node:test';
import { campaignToggled, observedRole, observedSwitches, PickerOptions, toggleMapSetting } from './mapPicker.ts';
import type { CountrySelection, WidgetSpec } from './model';
import type { MapOptions } from '../shell/api';
import type { DashboardClient, DashboardMapView, MapSettingsReply } from './client';

test('a picker choice keeps the map mode and slots, and the fog switch still decides fog', () => {
  const maps = new PickerOptions();
  maps.changed({ viewId: 'm', options: { country: null, fog: false, mapMode: 'strategic_air', mapModeSlots: ['states'] } });
  const shown = maps.merge('m', { country: 'GER', fog: true });
  assert.deepEqual(shown, { country: 'GER', fog: true, mapMode: 'strategic_air', mapModeSlots: ['states'] });
  assert.equal(maps.merge('m', { country: 'GER', fog: false })?.fog, false);
});

test("a Ctrl+click sets the default campaign's country from the campaign map, a widget map's own, never a pinned one", () => {
  const tracks = [{ id: 'main', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: null },
    { id: 'other', roleId: 'comparison', label: 'Other', branchIndex: 0, anchorOffset: null }];
  const map = (id: string, campaign: 'default' | 'tracks', trackId: string, countries: CountrySelection = { kind: 'selected' }): WidgetSpec =>
    ({ id, kind: 'map', configVersion: 1, title: 'Campaign map', campaign, trackIds: [trackId], countries, time: { mode: 'follow' } });
  const followsDefault = map('default-map', 'default', 'main');
  const fixed = map('fixed-map', 'tracks', 'other');
  const pinned = map('pinned-map', 'tracks', 'main', { kind: 'tags', tags: ['GER'] });
  const template = { tracks, widgets: [followsDefault, fixed, pinned] };
  assert.equal(observedRole('campaign-map', template, 'comparison'), 'comparison');
  assert.equal(observedRole('campaign-map', template, null), null);
  assert.equal(observedRole('default-map', template, 'comparison'), 'comparison');
  assert.equal(observedRole('fixed-map', template, 'primary'), 'comparison');
  assert.equal(observedRole('pinned-map', template, 'primary'), null);
  assert.equal(observedRole('gone', template, 'primary'), null);
});

test('fog clicks flip the view switch in order: two give the original, three the opposite', () => {
  const maps = new PickerOptions();
  let fog = true;
  const stop = maps.onFog('m', () => { fog = !fog; });
  assert.ok(maps.toggled('m', { setting: 'fog' }));
  assert.ok(maps.toggled('m', { setting: 'fog' }));
  assert.equal(fog, true);
  maps.toggled('m', { setting: 'fog' });
  assert.equal(fog, false);
  maps.toggled('other', { setting: 'fog' });
  assert.ok(!maps.toggled('m', { setting: 'dayNight' }));
  fog = true;
  maps.toggled('m', { setting: 'fog' });
  assert.equal(fog, false);
  stop();
  maps.toggled('m', { setting: 'fog' });
  assert.equal(fog, false);
});

test('the campaign map and the pop-out flip the one campaign fog switch, each click in order', () => {
  let options: MapOptions = { country: null, fog: true };
  const update = (flip: (options: MapOptions) => MapOptions) => { options = flip(options); };
  campaignToggled({ viewId: 'campaign-map', toggle: { setting: 'fog' } }, update, {} as DashboardClient, null);
  campaignToggled({ viewId: 'campaign-map', toggle: { setting: 'fog' } }, update, {} as DashboardClient, null);
  assert.equal(options.fog, true);
  campaignToggled({ viewId: 'campaign-map', toggle: { setting: 'fog' } }, update, {} as DashboardClient, null);
  assert.equal(options.fog, false);
  campaignToggled({ viewId: 'w', toggle: { setting: 'fog' } }, update, {} as DashboardClient, null);
  campaignToggled({ viewId: 'campaign-map', toggle: { setting: 'dayNight' } }, update, {} as DashboardClient, null);
  assert.deepEqual(options, { country: null, fog: false });
});

test('setting clicks ask the service to flip, in click order, so a reset meanwhile is what they flip', async () => {
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
  const flip = toggleMapSetting(client, view, { setting: 'dayNight' });
  await writing;
  stored = { graphics: { dayNight: null, borders: null }, audio: { voice: 40 } };
  paused = false;
  release();
  await flip;
  assert.deepEqual(stored, { graphics: { dayNight: false, borders: null }, audio: { voice: 40 } });
  await Promise.all([
    toggleMapSetting(client, view, { setting: 'dayNight' }),
    toggleMapSetting(client, view, { setting: 'dayNight' }),
  ]);
  assert.equal(stored.graphics.dayNight, false);
  assert.deepEqual(sent, Array(3).fill('dayNight {"scope":"widget","id":"w"}'));
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

test("each view's Ctrl+click switches are echoed by the latest applied, never moved back by an older one", () => {
  const observe = (viewId: string, sequence: number) => ({ viewId, country: { slot: 1, tag: 'GER', sequence } });
  let observed = observedSwitches({}, observe('campaign-map', 5));
  observed = observedSwitches(observed, observe('widget', 2));
  assert.deepEqual(observed, { 'campaign-map': 5, widget: 2 }, 'each view echoes its own');
  const kept = observedSwitches(observed, observe('campaign-map', 3));
  assert.equal(kept, observed, 'an older switch arriving late changes nothing');
  assert.deepEqual(observedSwitches(observed, observe('campaign-map', 9)), { 'campaign-map': 9, widget: 2 });
});
