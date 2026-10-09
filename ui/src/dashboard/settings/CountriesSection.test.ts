import test from 'node:test';
import assert from 'node:assert/strict';
import { isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { createServer } from 'vite';
import { mergeViewPreferences } from '../model.ts';
import type { BoundTrack, CountrySelection, DashboardTemplate, WidgetSpec } from '../model';
import type { WidgetRuntime } from '../definitions';
import { selectedTags } from '../countrySelection.ts';
import { parseTemplate } from '../storage.ts';

const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
const { CountriesSection } = await vite.ssrLoadModule('/ui/src/dashboard/settings/CountriesSection.tsx').finally(() => vite.close()) as typeof import('./CountriesSection.tsx');

function elements(value: ReactNode): ReactElement<Record<string, unknown>>[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  if (!isValidElement<Record<string, unknown>>(value)) return [];
  return [value, ...elements(value.props.children as ReactNode)];
}

test('removing the last country chip or picker choice follows the dashboard country', () => {
  for (const all of [false, true]) {
    const changes: CountrySelection[] = [];
    const widget = { countries: { kind: 'tags', tags: ['Opaque mod | country'] } } as WidgetSpec;
    const section = CountriesSection({ widget, reference: undefined, runtime: { client: {} } as WidgetRuntime,
      limits: { max: 5_000, all }, onChange: value => changes.push(value) });
    const child = (section.props.children as ReactElement<Record<string, unknown>>[])[1];
    const chips = (child.type as (props: Record<string, unknown>) => ReactElement)(child.props);
    const remove = elements(chips).find(element => element.props['aria-label'] === 'Remove Opaque mod | country')!;
    (remove.props.onClick as () => void)();
    const picker = elements(chips).find(element => element.props.addLabel !== undefined)!;
    (picker.props.onChange as (tag: string) => void)('Opaque mod | country');
    assert.deepEqual(changes, [{ kind: 'selected' }, { kind: 'selected' }]);
  }
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
    assert.deepEqual(selectedTags(restored.widgets[0].countries, { ...reference, selectedTag: 'Another native country' }),
      [fixed ? tag : 'Another native country']);
    choose('Another native country');
    choose(null);
    assert.deepEqual(changes.slice(1), [{ kind: 'tags', tags: ['Another native country'] }, { kind: 'selected' }]);
  }
});
