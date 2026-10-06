import test from 'node:test';
import assert from 'node:assert/strict';
import { isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { createServer } from 'vite';
import type { CountrySelection, WidgetSpec } from '../model';
import type { WidgetRuntime } from '../definitions';

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
