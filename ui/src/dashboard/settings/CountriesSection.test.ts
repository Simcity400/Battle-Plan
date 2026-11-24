import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { mergeViewPreferences } from '../model.ts';
import type { BoundTrack, CountrySelection, DashboardTemplate, WidgetSpec } from '../model';
import type { WidgetRuntime } from '../definitions';
import { countryKey } from '../countryKey.ts';
import { selectedCountryKeys } from '../countrySelection.ts';
import { parseTemplate } from '../storage.ts';

const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
const { CountriesSection, groupKinds } = await vite.ssrLoadModule('/ui/src/dashboard/settings/CountriesSection.tsx').finally(() => vite.close()) as typeof import('./CountriesSection.tsx');

test('faction choices keep their labels without showing backend naming refusals', () => {
  const reply = { factions: [{ id: '42', name: null, nameReason: 'Synthetic native faction refusal', names: [] }] } as unknown as import('../../generated/dashboard').CountryGroupReply;
  const kinds = groupKinds(reply, { max: 5000 });
  assert.deepEqual(kinds[1], { value: 'faction:42', label: 'Unnamed faction' });
  assert.equal(groupKinds(reply, { max: 5000 }, undefined, { '42': { name: 'Readable override' } })[1].label, 'Readable override');
});

test('a faction founded after the hour shown is offered by its own name, never as one of several unnamed factions', () => {
  const later = (id: string, name: string, firstHour: number) => ({ id, name, nameReason: null, names: [{ firstHour, lastHour: 900, name, reason: null }] });
  const reply = { factions: [later('7', 'African Union', 500), later('8', "The People's Front", 800)] } as unknown as import('../../generated/dashboard').CountryGroupReply;
  assert.deepEqual(groupKinds(reply, { max: 5000 }, 100).slice(1, 3).map(kind => kind.label), ['African Union', "The People's Front"]);
  assert.equal(groupKinds(reply, { max: 5000 }, 600)[1].label, 'African Union');
});



test('a fixed single-country picker saves a matching dashboard tag explicitly', () => {
  const tag = 'Opaque mod | country 4999';
  const reference = { selectedTag: tag, source: { metadata: { countries: [], playerCountryId: null } } } as unknown as BoundTrack;
  for (const fixed of [undefined, true]) {
    const widget: WidgetSpec = { id: 'fixed', kind: 'test-fixed', configVersion: 1, title: 'Fixed country', campaign: 'default',
      trackIds: ['main'], countries: { kind: 'tags', tags: [] }, time: { mode: 'campaign' } };
    const template: DashboardTemplate = { version: 1, name: 'Synthetic', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: tag }],
      tracks: [{ id: 'main', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: null }],
      widgets: [widget], layouts: { lg: [{ i: widget.id, x: 0, y: 0, w: 4, h: 6 }] } };
    const changes: CountrySelection[] = [];
    const picker = CountriesSection({ widget, reference, runtime: { client: {} } as WidgetRuntime,
      limits: { max: 1, fixed }, onChange: value => changes.push(value) });
    const choose = picker.props.onChange as (tag: string | null) => void;
    choose(tag);
    assert.deepEqual(changes[0], fixed ? { kind: 'tags', tags: [tag] } : { kind: 'selected' });
    template.widgets[0] = mergeViewPreferences(widget, widget, { ...widget, countries: changes[0] }, new Set(['main']));
    const restored = parseTemplate(JSON.parse(JSON.stringify(template)))!;
    assert.ok(restored);
    assert.deepEqual(selectedCountryKeys(restored.widgets[0].countries, { ...reference, selectedSlot: 41 }),
      [fixed ? tag : countryKey(41)]);
    choose('Another native country');
    choose(null);
    assert.deepEqual(changes.slice(1), [{ kind: 'tags', tags: ['Another native country'] }, { kind: 'selected' }]);
  }
});
