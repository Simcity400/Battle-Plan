import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, type ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { BoundTrack, CountrySelection, WidgetSpec } from './model';
import type { WidgetCountryChipProps, WidgetPreset, WidgetRuntime } from './definitions';

// Each breakdown's header chip, the option its Pin as widget copy saves the country in,
// and its definition, whose catalogue cards are the unpinned breakdowns. These bodies
// offer no country to choose, so an unpinned one asks for it in the header.
const BREAKDOWNS: readonly [kind: string, chip: string, pin: string][] = [
  ['political-power', 'country-measure/MeasureCountryChip.tsx', 'powerCountry'],
  ['command-power', 'country-measure/MeasureCountryChip.tsx', 'powerCountry'],
  ['stability', 'stability/StabilityCountryChip.tsx', 'stabilityCountry'],
  ['equipment-losses', 'equipment-losses/LossesCountryChip.tsx', 'lossCountry'],
  ['nuclear-arsenal', 'nuclear-arsenal/NuclearCountryChip.tsx', 'drillOriginHour'],
];
// These bodies let you choose a country, so they keep the header even unpinned
// (savedActorHeaders and NavalLossesPin follow a card to a choice).
const CHOOSING: readonly [kind: string, chip: string][] = [
  ['army-size', 'army-size/ArmyCountryChip.tsx'],
  ['supply-health', 'supply-health/SupplyCountryChip.tsx'],
  ['naval-losses', 'naval-losses/NavalLossesCountryChip.tsx'],
];

test('an unpinned breakdown asks for a country unless its body offers one; a pinned one leaves its header to the body', async () => {
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
    for (const [kind, path, pin] of BREAKDOWNS) {
      const { variants, chip } = await load(kind, path);
      // A breakdown's card is one whose Pin as widget copy hands its header to the body.
      const breakdowns = variants.filter(variant => /dash-widget-country-slot/.test(chip({ ...variant.initial.options, [pin]: 0 })));
      assert.ok(breakdowns.length, `${kind} has a catalogue card for a breakdown`);
      for (const { key, initial: { options = {} } } of breakdowns) {
        for (const countries of [top, { kind: 'selected' }, { kind: 'tags', tags: ['opaque-a'] }] as CountrySelection[]) {
          const header = chip(options, countries);
          assert.match(header, />Choose a country</, `${kind} ${key} asks for a country`);
          assert.doesNotMatch(header, /Unavailable|Undated name|dash-widget-country-slot/, `${kind} ${key} names no holder the body has not dated`);
        }
      }
    }
  } finally { await vite.close(); }
});
