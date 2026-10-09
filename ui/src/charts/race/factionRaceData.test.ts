import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { factionRaceRequest, SMOOTH_TOTALS_PRESENTATION } from './factionRaceData.ts';

test('faction requests size all-factions choices from the recorded catalogue, with no thirty-faction ceiling', () => {
  const source = { kind: 'factories', mode: 'free', groups: [] } as const;
  for (const membership of ['hour', 'end', 'ever'] as const) {
    const selection = { kind: 'factions', count: Number.MAX_SAFE_INTEGER, membership } as const;
    assert.deepEqual(factionRaceRequest('branch', selection, { ...source, groups: [] }, 5000, Number.MAX_SAFE_INTEGER),
      { branchId: 'branch', source, membership, top: 5000 });
    assert.equal(factionRaceRequest('branch', { ...selection, count: 42 }, { ...source, groups: [] }, 5000, Number.MAX_SAFE_INTEGER)?.top, 42);
    assert.equal(factionRaceRequest('branch', selection, { ...source, groups: [] }, 0, Number.MAX_SAFE_INTEGER)?.top, 1);
  }
});

test('Territory and Faction strength use the same eased faction view; country Territory stays smooth too', () => {
  assert.equal(SMOOTH_TOTALS_PRESENTATION.exact, false);
  assert.equal(SMOOTH_TOTALS_PRESENTATION.held, false);
  assert.equal(SMOOTH_TOTALS_PRESENTATION.lineColourHours, 'labels');
  assert.equal(SMOOTH_TOTALS_PRESENTATION.raceTooltip, 'hover');
  const territory = readFileSync(new URL('../../widgets/territory/TerritoryWidget.tsx', import.meta.url), 'utf8');
  const strength = readFileSync(new URL('../../widgets/faction-strength/FactionStrengthWidget.tsx', import.meta.url), 'utf8');
  const shared = readFileSync(new URL('./FactionRaceView.tsx', import.meta.url), 'utf8');
  assert.match(territory, /<FactionRaceView/);
  assert.match(territory, /presentation=\{\{ \.\.\.TERRITORY_PRESENTATION/);
  assert.match(strength, /<FactionRaceView/);
  assert.match(shared, /presentation=\{\{ \.\.\.SMOOTH_TOTALS_PRESENTATION/);
  assert.match(shared, /tooltipTags=\{\(hour, ids\) => factionVisibleMemberTags\(model, hour, ids\)\}/);
  for (const source of [territory, strength, shared]) {
    assert.doesNotMatch(source, /tooltipResetKey|mixSamplePixels|raceTooltip:\s*'sampled'/);
  }
});
