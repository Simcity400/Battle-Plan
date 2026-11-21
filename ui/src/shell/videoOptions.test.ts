import test from 'node:test';
import assert from 'node:assert/strict';
import { sameDisplay } from './videoOptions.ts';

test('the maps display alike whatever their diagnostics switch, so Revert stays hidden at the game settings', () => {
  const game = { useGameSettings: true, uiScale: 100, vsync: true, mapUiScale: null, mapFrameLimit: null };
  const saved = { ...game, mapDiagnostics: true };
  assert.equal(sameDisplay(saved, game), true);
  for (const changed of [{ ...saved, uiScale: 125 }, { ...saved, vsync: false }, { ...saved, mapUiScale: 90 }, { ...saved, mapFrameLimit: 60 }, { ...saved, useGameSettings: false }])
    assert.equal(sameDisplay(changed, game), false, JSON.stringify(changed));
  assert.equal(sameDisplay(null, game), false);
});
