import test from 'node:test';
import assert from 'node:assert/strict';
import { isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from './definitions';
import type { PoliticsLedgerReply } from '../generated/dashboard';
import type { LogListProps } from '../charts/log/LogList';
import type { SpendingRow } from './politicsSpending';
import { spendingAt, spendingRows } from './politicsSpending.ts';
import { OptionDraft } from './optionDraft.ts';
import { POLITICS_SPENDING_OPTIONS, NO_SPENDING_CATEGORIES } from './politicsSpendingOptions.ts';

test('rendered shown categories migrate once, preserve legacy rows and limit category totals', async () => {
  const names = ['alpha', 'beta', 'none'];
  const data: PoliticsLedgerReply = { branchId: 'branch', tag: 'actor', decimals: 5, warnings: [], mutations: [], governments: [], daily: [], dailyCosts: [], coverage: [],
    commandHolds: [], ideaTooltips: null, spendingCoverage: [{ readHour: 0, sinceHour: 0, throughHour: 10, political: true, command: true, reason: null }],
    spendCategories: names.map(id => ({ id, name: `Category ${id}`, identified: true })),
    spendItems: names.map(id => ({ id, groupId: id, name: id, action: id, icon: null, tooltipToken: null, character: null, targetTag: null, targetName: null, unknown: null })),
    spending: names.map((id, ordinal) => ({ id, category: id, itemId: id, hour: 1, readHour: 1, sequence: '1', ordinal, countryTag: 'actor', countryName: null,
      currency: 'Political', chargeRaw: '100000', realizedDeltaRaw: '-100000', covered: true, occurrences: 1,
      changes: [{ readHour: 1, chargeRaw: '100000', realizedDeltaRaw: '-100000', covered: true, occurrences: 1 }] })) };
  const state = { data: undefined as PoliticsLedgerReply[] | undefined, slots: [] as unknown[], cursor: 0, effects: [] as (() => unknown)[],
    list: undefined as LogListProps<SpendingRow> | undefined };
  const hooks = {
    useMemo(build: () => unknown, deps: readonly unknown[]) {
      const index = state.cursor++, before = state.slots[index] as { deps: readonly unknown[]; value: unknown } | undefined;
      if (before && before.deps.length === deps.length && deps.every((value, i) => Object.is(value, before.deps[i]))) return before.value;
      const value = build(); state.slots[index] = { deps, value }; return value;
    },
    useCallback(callback: unknown, deps: readonly unknown[]) { return hooks.useMemo(() => callback, deps); },
    useRef(initial: unknown) { const index = state.cursor++; return state.slots[index] ??= { current: initial }; },
    useState(initial: unknown) { const index = state.cursor++; if (!(index in state.slots)) state.slots[index] = initial;
      return [state.slots[index], (value: unknown) => { state.slots[index] = value; }]; },
    useEffect(effect: () => unknown, deps: readonly unknown[]) { hooks.useMemo(() => state.effects.push(effect), deps); },
  };
  const host = globalThis as typeof globalThis & { __spendingMigration?: typeof state; __spendingMigrationHooks?: typeof hooks };
  host.__spendingMigration = state; host.__spendingMigrationHooks = hooks;
  const replacements: Record<string, string> = {
    '/dashboard/usePoliticsSpending.ts': `export const usePoliticsSpending = (_widget, runtime) => ({track: runtime.tracks[0], tags: ['actor'],
      data: globalThis.__spendingMigration.data, ready: true, groupWarnings: [], retry() {}}); export const useSpendingIdeaTooltips = () => undefined;`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 10;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456'], muted: '#888888' });`,
    '/charts/log/LogList.tsx': 'export default props => { globalThis.__spendingMigration.list = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'spending-migration-render', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/dashboard/PoliticsSpendingLog.tsx')) return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__spendingMigrationHooks;');
      },
    }] });
  try {
    const { default: Log } = await vite.ssrLoadModule('/ui/src/dashboard/PoliticsSpendingLog.tsx') as typeof import('./PoliticsSpendingLog.tsx');
    const track = { id: 'track', axisOffset: 0, branch: { id: 'branch', firstHour: 0, lastHour: 10 }, source: { id: 'source', metadata: {} } };
    const props = { currency: 'Political', widget: { id: 'log', time: { mode: 'follow' }, trackIds: ['track'], options: { filterMode: 'hide', categories: 'alpha absent' } },
      runtime: { tracks: [track], clock: { getHour: () => 10 }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps & { currency: 'Political' };
    let stored = props.widget.options;
    let draft = new OptionDraft(stored);
    const writes: string[] = [];
    props.setOptions = values => { writes.push(Object.keys(values).join(' + ')); stored = draft.updateAll(POLITICS_SPENDING_OPTIONS, values)!; };
    let tree: ReactNode;
    const render = () => {
      state.cursor = 0; state.effects = []; tree = Log(props);
      const html = renderToStaticMarkup(tree);
      for (const effect of state.effects) effect();
      return html;
    };
    const ids = () => state.list!.rows!.map(row => row.key);
    render(); assert.deepEqual(writes, [], 'catalogue loading cannot persist a guessed selection');
    state.data = [data];
    const migrated = render();
    assert.deepEqual(ids(), spendingRows(spendingAt([data], 'Political', 10), [], 'alpha absent', 'newest', 'exclude').map(row => row.key));
    assert.deepEqual(writes, ['categories + filterMode'], 'the converted selection and its marker save as one change');
    assert.equal(stored!.filterMode, 'shown'); assert.equal(stored!.categories, 'beta none');
    assert.match(migrated, /Total <b>3<\/b>/);
    assert.match(migrated, /Category beta <b>1<\/b>/);
    assert.match(migrated, /Category none <b>1<\/b>/);
    assert.doesNotMatch(migrated, /Category alpha <b>/);
    render(); state.data = [{ ...data }]; render();
    assert.equal(writes.length, 1, 'pending parent props, renders and equivalent catalogues cannot repeat migration');
    props.widget.options = stored; render(); render(); assert.equal(writes.length, 1);
    const select = (categories: string) => { props.widget.options = { filterMode: 'shown', categories }; draft = new OptionDraft(props.widget.options); return render(); };
    assert.match(select(''), /Category alpha <b>1<\/b>/); assert.equal(ids().length, 3);
    select('alpha beta none'); assert.equal(ids().length, 3);
    const missing = select('absent'); assert.equal(ids().length, 0); assert.match(missing, /Total <b>3<\/b>/); assert.doesNotMatch(missing, /Category \w+ <b>/);
    select(NO_SPENDING_CATEGORIES); assert.equal(ids().length, 0);
    assert.match(render(), /aria-pressed="true"><i[^>]*><\/i>None<\/button>/);
    const findChips = (node: ReactNode): ReactElement<{ onToggle: (value: string) => void }> | undefined => {
      if (Array.isArray(node)) return node.map(findChips).find(Boolean);
      if (!isValidElement<{ children?: ReactNode }>(node)) return undefined;
      if (typeof node.type === 'function' && node.type.name === 'ChipSet') return node as ReactElement<{ onToggle: (value: string) => void }>;
      return findChips(node.props.children);
    };
    findChips(tree)!.props.onToggle('alpha'); props.widget.options = stored; render(); assert.equal(ids().length, 1);
    findChips(tree)!.props.onToggle(NO_SPENDING_CATEGORIES); props.widget.options = stored; render(); assert.equal(ids().length, 0);
    findChips(tree)!.props.onToggle(NO_SPENDING_CATEGORIES); props.widget.options = stored; render(); assert.equal(ids().length, 3);
    const beforeRepair = writes.length;
    select(`${NO_SPENDING_CATEGORIES} beta`); assert.equal(ids().length, 1);
    assert.equal(stored!.categories, 'beta', 'generic settings and body chips converge on the same selection');
    render(); assert.equal(writes.length, beforeRepair + 1, 'selection repair saves once');
    props.widget.options = stored; render();
    assert.doesNotMatch(render(), /aria-pressed="true"><i[^>]*><\/i>None<\/button>/, 'None is exclusive even after generic settings selected a category');
  } finally { await vite.close(); delete host.__spendingMigration; delete host.__spendingMigrationHooks; }
});
