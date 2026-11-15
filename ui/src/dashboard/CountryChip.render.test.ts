import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, type ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { BoundTrack, CountrySelection, WidgetSpec } from './model';
import type { WidgetCountryChipProps, WidgetPreset, WidgetRuntime } from './definitions';

// Breakdowns whose body lets you choose a country keep the header for the body even unpinned.
const CHOOSING: readonly [kind: string, chip: string][] = [
  ['army-size', 'army-size/ArmyCountryChip.tsx'],
];

test('a breakdown whose body offers a country leaves its header to the body', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const track = { id: 'track', roleId: 'role', label: 'Campaign', selectedTag: 'opaque-a',
      branch: { id: 'branch', firstHour: 0, lastHour: 100 },
      source: { path: 'campaign.battleplan', metadata: { campaignKey: 'campaign', branches: [{ id: 'branch', firstHour: 0, lastHour: 100 }],
        countries: [{ id: '0', tag: 'opaque-a', name: 'Undated name', observedBranchId: 'branch', observedHour: 0 }] } } } as unknown as BoundTrack;
    const runtime = { tracks: [track], campaignTracks: [track], defaultRoleId: 'role', client: {}, clock: { getHour: () => 0 } } as unknown as WidgetRuntime;
    const top: CountrySelection = { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false };
    const load = async (kind: string, path: string) => {
      const { definition } = await vite.ssrLoadModule(`/ui/src/widgets/${kind}/definition.ts`) as { definition: { variants: readonly WidgetPreset[] } };
      const { default: Chip } = await vite.ssrLoadModule(`/ui/src/widgets/${path}`) as { default: ComponentType<WidgetCountryChipProps> };
      const chip = (options: Record<string, unknown>, countries: CountrySelection = top) => renderToStaticMarkup(createElement(Chip, { runtime, active: false, onSlot() {},
        widget: { id: kind, kind, configVersion: 1, title: kind, campaign: 'default', trackIds: ['track'], time: { mode: 'follow' }, countries, options } as WidgetSpec }));
      return { variants: definition.variants, chip };
    };
    for (const [kind, path] of CHOOSING) {
      const { variants, chip } = await load(kind, path);
      assert.ok(variants.some(variant => /dash-widget-country-slot/.test(chip(variant.initial.options ?? {}))), `${kind}'s breakdown card keeps the body's header`);
    }
  } finally { await vite.close(); }
});
