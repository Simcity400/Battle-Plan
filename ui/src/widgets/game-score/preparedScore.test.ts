import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { SCORE_PARTS, FALLBACK_LABELS, gameScoreModel, gameScoreRows } from './gameScoreData.ts';
import type { GameScoreReply, GameScoreSeries } from '../../generated/dashboard';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ScoreRow } from './gameScoreData';

const series = (): GameScoreSeries => Object.fromEntries(SCORE_PARTS.map(part => [part,
  [{ firstHour: 104, lastHour: 104, raw: part === 'total' ? '9007199254740993' : '-100001', readEachHour: true }]])) as GameScoreSeries;
const reply: GameScoreReply = { branchId: '1', scale: 100000, reads: [104], reasons: [], warnings: [], scoreWarnings: [],
  labels: { ...FALLBACK_LABELS, faction: 'Faction' }, hints: {}, multipliers: series(), countries: [{ tag: 'opaque / alias',
    identity: [{ firstHour: 104, lastHour: 104, value: { canonicalSlot: 17, nativeCountry: 29, name: '', nameMissing: null,
      color: [17, 29, 0], colorMissing: null, faction: null, factionMissing: null } }], scores: series() }] };

test('prepared score table and race reuse exact rows and prepared holder continuity', async () => {
  const stretch = JSON.stringify([JSON.stringify([17, 29]), 100]);
  let captured: readonly ScoreRow[] | undefined;
  let parity: (() => { rows: readonly ScoreRow[]; complete: boolean }) | undefined;
  const replacements: Record<string, string> = {
    '/widgets/game-score/usePreparedScore.ts': `export const usePreparedScore = () => globalThis.__scorePrepared;`,
    '/dashboard/countryNamesAt.ts': `export const nameKey = () => ''; export const useCountryFactsRead = () => new Map();`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 104;',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000; export const useChartTokens = () => ({});',
    '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => null;',
    '/charts/table/Table.tsx': `export const CountryCell = () => null; export default props => { globalThis.__scoreRows = props.rows; return null; };`,
    '/charts/race/CountryRaceView.tsx': `export default props => { globalThis.__scoreRace = props.ranked; return null; };`,
    '/dashboard/parityRows.ts': `export const useParityRows = (_root, read) => { globalThis.__scoreParity = read; };`,
  };
  const host = globalThis as typeof globalThis & { __scoreRows?: readonly ScoreRow[];
    __scoreRace?: { stretch: string; label: string; entityKey: string }[]; __scoreParity?: typeof parity;
    __scorePrepared?: { data: { reply: GameScoreReply; stretches: Record<string, string>; approximate: boolean };
      hour: number; pending: boolean; error?: string; retry: () => void } };
  host.__scorePrepared = { data: { reply, stretches: { 'opaque / alias': stretch }, approximate: false }, hour: 104, pending: false, retry() {} };
  const vite = await createServer({ configFile: false, mode: 'parity', optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'prepared-score-fixture', enforce: 'pre',
      transform: (_source, id) => { const key = Object.keys(replacements).find(key => id.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/game-score/GameScoreWidget.tsx');
    const props = { widget: { id: 'score', kind: 'game-score', trackIds: ['main'], countries: { kind: 'all' },
      time: { mode: 'campaign' }, options: {} }, runtime: { tracks: [{ id: 'main', axisOffset: 0,
        source: { id: 'synthetic', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 104 } }],
        clock: { getHour: () => 104 }, client: {} }, active: true, onDiagnostics() {}, setOption() {} } as unknown as WidgetProps;
    renderToStaticMarkup(createElement(Widget, props));
    captured = host.__scoreRows;
    const expected = gameScoreRows(gameScoreModel(reply), ['opaque / alias'], 104, { column: 'land', direction: 'descending' });
    assert.deepEqual(captured, expected.map(row => ({ ...row, stretch })));
    parity = host.__scoreParity;
    assert.equal(parity?.().complete, true);
    assert.deepEqual(parity?.().rows, captured);
    renderToStaticMarkup(createElement(Widget, { ...props, widget: { ...props.widget, options: { view: 'race' } } }));
    assert.equal(host.__scoreRace?.[0].stretch, stretch);
    assert.equal(host.__scoreRace?.[0].entityKey, JSON.stringify([17, 29]));
    renderToStaticMarkup(createElement(Widget, { ...props, widget: { ...props.widget, countries: { kind: 'group', group: { type: 'top', count: 1 }, membership: 'hour', aggregate: false } } }));
    assert.equal(host.__scoreRows?.length, 1, 'Top score selection does not depend on unrelated group history.');
    host.__scorePrepared = { ...host.__scorePrepared, data: { reply, stretches: {}, approximate: true }, error: 'Background preparation is off.' };
    assert.match(renderToStaticMarkup(createElement(Widget, props)), /data-detail="Background preparation is off\."/, 'the stop is said beside the rows');
    assert.deepEqual(host.__scoreRows, expected, 'unprepared rows remain visible when background preparation stops');
    assert.equal(host.__scoreParity?.().complete, false, 'tail values have no prepared parity witness');
    renderToStaticMarkup(createElement(Widget, { ...props, widget: { ...props.widget, options: { view: 'race' } } }));
    assert.equal(host.__scoreRace?.[0].stretch, expected[0].stretch, 'tail race uses only its own hour continuity');
    assert.notEqual(host.__scoreRace?.[0].stretch, stretch);
    host.__scoreRows = undefined;
    host.__scorePrepared = { ...host.__scorePrepared, hour: 100, pending: true, error: 'Cannot read hour 104' };
    assert.match(renderToStaticMarkup(createElement(Widget, props)), /data-detail="Cannot read hour 104"/);
    assert.equal(host.__scoreRows, undefined, 'no table shows another hour in place of the one asked');
    const seekFailed = renderToStaticMarkup(createElement(Widget, { ...props, widget: { ...props.widget, options: { view: 'race' } } }));
    assert.doesNotMatch(seekFailed, /Cannot read hour 104/, 'a failed seek is left to the race state alone, with no notice beside an older hour');
    for (const options of [{}, { view: 'race' }]) {
      host.__scoreRows = undefined;
      host.__scorePrepared = { data: { reply: { ...reply, countries: [] }, stretches: {}, approximate: true }, hour: 104, pending: false,
        error: 'Preparation stopped', retry() {} };
      const html = renderToStaticMarkup(createElement(Widget, { ...props, widget: { ...props.widget, options } }));
      assert.equal(html.match(/data-detail="Preparation stopped"/g)?.length, 1, 'the refresh failure is said with its Retry');
      assert.match(html, />Retry</);
      if (!('view' in options)) {
        assert.ok((html.match(/class="widget-state/g)?.length ?? 0) >= 2, 'the empty state stays beside the failure');
        assert.equal(host.__scoreRows, undefined);
      }
    }
  } finally {
    delete host.__scoreRows; delete host.__scoreRace; delete host.__scoreParity; delete host.__scorePrepared;
    await vite.close();
  }
});
