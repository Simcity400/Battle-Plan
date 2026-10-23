import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { branchCoverage, branchLabels, missingTime } from './diagnosticsData.ts';
import type { ReplayMetadata } from './api';

test('a branch’s gaps sit where they fall in its recorded time, and missing time reads in hours or days', () => {
  const coverage = branchCoverage({ firstHour: 0, lastHour: 99, gaps: [{ firstHour: 10, lastHour: 19, reason: 'lost' }, { firstHour: 90, lastHour: 120, reason: 'end' }] });
  assert.deepEqual(coverage.gaps.map(gap => [gap.left, gap.width]), [[10, 10], [90, 10]], 'a gap past the branch end is cut at the end');
  assert.equal(coverage.missingHours, 41);
  assert.equal(missingTime(1), '1 hour');
  assert.equal(missingTime(47), '47 hours');
  assert.equal(missingTime(24 * 12 + 5), '12 days');
});

test('diagnostics say how complete the recording is in plain words, with the file’s notes folded away', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'diagnostics-fixture', enforce: 'pre',
      transform(_source, id) {
        if (id.replaceAll('\\', '/').endsWith('/shell/diagnosticsSetting.ts')) return 'export const useMapDiagnostics = () => ({ enabled: false, error: null, set() {} });';
      },
    }] });
  try {
    const { default: Diagnostics } = await vite.ssrLoadModule('/ui/src/shell/MapDiagnostics.tsx') as typeof import('./MapDiagnostics');
    const branch = { id: 'b1', sessionId: 's1', firstHour: 43800024, lastHour: 43800024 + 24 * 30, parentId: null, parentHour: null, sampleBoundary: 'hourly', gaps: [] };
    const metadata = { path: 'C:/recordings/GER 1936.battleplan', name: 'GER 1936.battleplan', format: 100, captureMode: 'full', campaignKey: 'c', branches: [branch],
      countries: [], playerCountryId: null, contentVerified: true, partial: null, warnings: ['Provinces.attack not recorded (introduced at format 101)'], revision: '1' } as ReplayMetadata;
    const visible = (value: ReplayMetadata | null) => renderToStaticMarkup(createElement(Diagnostics, { native: null, metadata: value })).replace(/<[^>]*>/g, ' ');
    const complete = visible(metadata);
    assert.match(complete, /GER 1936 /, 'the recording is named without its file extension');
    assert.match(complete, /Complete/);
    assert.match(complete, /Recorded throughout/);
    assert.match(complete, /File notes/);
    assert.doesNotMatch(complete, /Recording format|session|hourly|Branch b1/, 'no format number, session or sampling internals');
    const gapped = visible({ ...metadata, branches: [{ ...branch, gaps: [{ firstHour: branch.firstHour + 24, lastHour: branch.firstHour + 24 * 4 - 1, reason: 'Game paused' }] }] });
    assert.match(gapped, /1 gap/);
    assert.match(gapped, /3 days missing in 1 gap/);
    const stopped = visible({ ...metadata, partial: { sessionId: 's1', reason: 'The game closed.', failedHour: branch.lastHour } });
    assert.match(stopped, /Stopped early/);
    assert.match(stopped, /was saved: The game closed\./);
    const markup = (value: ReplayMetadata, shown: { id: string; choose: (id: string) => void } | null) =>
      renderToStaticMarkup(createElement(Diagnostics, { native: null, metadata: value, branch: shown }));
    assert.doesNotMatch(markup(metadata, { id: 'b1', choose() {} }), /Showing|>Show</, 'one branch has nothing to switch to');
    const two = { ...metadata, branches: [branch, { ...branch, id: 'b2', firstHour: branch.firstHour + 24 * 10 }] };
    const switchable = markup(two, { id: 'b1', choose() {} });
    assert.match(switchable, /Branch 1.*Showing/s, 'the branch being played says so');
    assert.match(switchable, /aria-label="Show Branch 2"/, 'another branch can be played from its row');
    assert.doesNotMatch(markup(two, null), /Showing|aria-label="Show /, 'without a playing campaign there is nothing to switch');
    assert.match(visible(null), /No recording open/);
    assert.match(visible(null), /Technical overlay/);
    assert.match(markup(metadata, null), /data-tooltip="GER 1936.battleplan/);
  } finally { await vite.close(); }
});

test('branches are numbered within their session, as the map settings number them', () => {
  assert.deepEqual([...branchLabels([{ id: 'a', sessionId: 's' }, { id: 'b', sessionId: 's' }]).values()], ['Branch 1', 'Branch 2']);
  assert.deepEqual([...branchLabels([{ id: 'a', sessionId: 's' }, { id: 'b', sessionId: 't' }, { id: 'c', sessionId: 't' }]).values()],
    ['Session 1 · Branch 1', 'Session 2 · Branch 1', 'Session 2 · Branch 2']);
});
