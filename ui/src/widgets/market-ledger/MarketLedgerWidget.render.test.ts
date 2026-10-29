import assert from 'node:assert/strict';
import test from 'node:test';
import { act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import type { ProductRequest } from '../../dashboard/client';
import type { LogPage } from '../../charts/log/logPages';
import type { CountryGroupReply, MarketLedgerReply } from '../../generated/dashboard';
import type { LedgerItem } from './ledgerData';

/** Just enough of a document for react-dom's client renderer. */
class FakeNode {
  childNodes: FakeNode[] = [];
  parentNode: FakeNode | null = null;
  attributes = new Map<string, string>();
  style: Record<string, unknown> = {};
  nodeValue: string | null = null;
  readonly nodeType: number; readonly nodeName: string; ownerDocument: FakeNode | null;
  constructor(nodeType: number, nodeName: string, ownerDocument: FakeNode | null = null) {
    this.nodeType = nodeType; this.nodeName = nodeName; this.ownerDocument = ownerDocument;
  }
  get namespaceURI() { return 'http://www.w3.org/1999/xhtml'; }
  get tagName() { return this.nodeName; }
  get firstChild(): FakeNode | null { return this.childNodes[0] ?? null; }
  get lastChild(): FakeNode | null { return this.childNodes.at(-1) ?? null; }
  get nextSibling(): FakeNode | null { const siblings = this.parentNode?.childNodes ?? []; return siblings[siblings.indexOf(this) + 1] ?? null; }
  get isConnected() { return true; }
  get textContent(): string { return this.nodeType === 3 ? this.nodeValue ?? '' : this.childNodes.map(child => child.textContent).join(''); }
  set textContent(text: string) { this.childNodes = []; if (text) this.appendChild(Object.assign(new FakeNode(3, '#text', this.ownerDocument), { nodeValue: text })); }
  appendChild(child: FakeNode) { return this.insertBefore(child, null); }
  insertBefore(child: FakeNode, before: FakeNode | null) {
    child.parentNode?.removeChild(child);
    const index = before ? this.childNodes.indexOf(before) : -1;
    this.childNodes.splice(index < 0 ? this.childNodes.length : index, 0, child);
    child.parentNode = this;
    return child;
  }
  removeChild(child: FakeNode) { this.childNodes.splice(this.childNodes.indexOf(child), 1); child.parentNode = null; return child; }
  setAttribute(name: string, value: string) { this.attributes.set(name, String(value)); }
  removeAttribute(name: string) { this.attributes.delete(name); }
  getAttribute(name: string) { return this.attributes.get(name) ?? null; }
  addEventListener() {}
  removeEventListener() {}
  contains(node: FakeNode | null): boolean { return node === this || this.childNodes.some(child => child.contains(node)); }
  getRootNode() { return this.ownerDocument ?? this; }
}

const PAGE = 256;
interface Fixture {
  prepared: number;
  requests: ProductRequest[];
  product: (request: ProductRequest, validate: (data: unknown) => MarketLedgerReply) => MarketLedgerReply;
  windows: (identity: string | null, request: (window: number) => ProductRequest, wanted: readonly number[], active: boolean,
    validate: (data: unknown) => MarketLedgerReply) => { data: Map<number, MarketLedgerReply>; errors: Map<number, string>; retry(): void };
  groups?: CountryGroupReply;
  log?: { pages: readonly LogPage<LedgerItem>[]; onWantedPages: (keys: readonly string[]) => void };
  state?: { message?: string; label?: string; kind: string };
}
const host = globalThis as typeof globalThis & { __ledger?: Fixture };

/** The prepared ledger as the server pages it: occurrences 1…`prepared` (each at
 * its own hour), newest first. Its cursors are the backend's segment cursors
 * (`chain:firstHour:position`, dot-separated), and anything else is refused as the
 * backend refuses it. Every country's ledger names its total. */
function serve(prepared: number, params: Record<string, unknown>): MarketLedgerReply {
  const every = !(params.tags as string[]).length && !params.members;
  const from = every ? -2147483648 : 0;
  const shown = Math.min(prepared, Number(params.lastHour));
  if (params.before !== undefined && !/^\d+:-?\d+:\d+(\.\d+:-?\d+:\d+)*$/.test(String(params.before))) throw new Error('Invalid market ledger cursor');
  const end = params.before === undefined ? shown : Math.min(Number(String(params.before).split(':')[2]), shown);
  const start = Math.max(0, end - PAGE);
  const rows = Array.from({ length: end - start }, (_, index) => {
    const id = String(end - index);
    return { id, occurrence: id, hour: end - index, kind: 21, mode: 0, contract: '0000000000000009', seller: '1', buyer: '2',
      variant: '0', amountRaw: '100000', totalRaw: '0', reason: null };
  });
  return { branchId: '1', lastHour: prepared, rows, total: every ? String(shown) : null, next: start > 0 ? `7:${from}:${start}` : null, known: [[0, prepared, '1']],
    holders: { '1': [{ hour: 0, tag: 'SEL' }], '2': [{ hour: 0, tag: 'BUY' }] }, variants: {}, warnings: [],
    coverage: { firstHour: 0, lastHour: prepared, complete: prepared >= 1000 }, pending: prepared >= 1000 ? [] : [{ firstHour: prepared + 1, lastHour: 1000 }],
  } as unknown as MarketLedgerReply;
}

function fixture(): Fixture {
  const firsts = new Map<string, MarketLedgerReply>();
  // Like the real windows: a window's reply is kept for its identity until the identity changes.
  const held = new Map<string, Map<number, MarketLedgerReply>>();
  const state: Fixture = {
    prepared: 0, requests: [],
    product(request, validate) {
      state.requests.push(request);
      const key = JSON.stringify([state.prepared, request.params]);
      if (!firsts.has(key)) firsts.set(key, validate(serve(state.prepared, request.params as Record<string, unknown>)));
      return firsts.get(key)!;
    },
    windows(identity, request, wanted, active, validate) {
      if (!active || identity === null) return { data: new Map(), errors: new Map(), retry() {} };
      let pages = held.get(identity);
      if (!pages) held.set(identity, pages = new Map());
      for (const window of wanted) if (!pages.has(window)) {
        const asked = request(window);
        state.requests.push(asked);
        pages.set(window, validate(serve(state.prepared, asked.params as Record<string, unknown>)));
      }
      return { data: new Map(pages), errors: new Map(), retry() {} };
    },
  };
  return state;
}

test('the market ledger keeps its pages whole as preparation advances and asks a group by its dated members', async () => {
  const document = new FakeNode(9, '#document');
  const html = new FakeNode(1, 'HTML', document), body = new FakeNode(1, 'BODY', document);
  html.appendChild(body);
  Object.assign(document, { documentElement: html, body, activeElement: body,
    createElement: (name: string) => new FakeNode(1, name.toUpperCase(), document),
    createElementNS: (_namespace: string, name: string) => new FakeNode(1, name, document),
    createTextNode: (text: string) => Object.assign(new FakeNode(3, '#text', document), { nodeValue: text }) });
  const replaced = ['document', 'window', 'HTMLIFrameElement', 'IS_REACT_ACT_ENVIRONMENT'] as const;
  const globals = globalThis as unknown as Record<string, unknown>;
  const before = new Map(replaced.map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const define = (key: string, value: unknown) => Object.defineProperty(globalThis, key, { configurable: true, writable: true, value });
  define('document', document);
  define('window', globalThis);
  define('HTMLIFrameElement', class {});
  define('IS_REACT_ACT_ENVIRONMENT', true);
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'market-ledger-fixture', enforce: 'pre',
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/products.ts')) return `export function useProduct(_client, request, active, validate) {
          return { data: request && active ? globalThis.__ledger.product(request, validate) : undefined, retry() {} }; }
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`;
        if (path.endsWith('/dashboard/productWindows.ts')) return `export function useProductWindows(_client, identity, request, wanted, active, validate) {
          return globalThis.__ledger.windows(identity, request, wanted, active, validate); }`;
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (path.endsWith('/dashboard/useCountryGroups.ts')) return `export const useCountryGroups = (_runtime, _track, active) =>
          ({ data: active ? globalThis.__ledger.groups : undefined, retry() {} });`;
        if (path.endsWith('/dashboard/countryFlagsAt.ts')) return 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag, known: true }) });';
        if (path.endsWith('/dashboard/playback.ts')) return 'export const usePlaybackHour = clock => clock.getHour();';
        if (path.endsWith('/charts/useChartTokens.ts')) return `export const useChartTokens = () => ({ categories: ['#123', '#456', '#789', '#abc', '#def', '#fed'] });
          export const useViewportHeight = () => 800;`;
        if (path.endsWith('/charts/log/LogList.tsx')) return 'export default props => { globalThis.__ledger.log = props; return null; };';
        if (path.endsWith('/charts/drill/DrillHost.tsx')) return 'export default props => props.main;';
        if (path.endsWith('/dashboard/MarketDrill.tsx')) return 'export default () => null;';
        if (path.endsWith('/components/ChipSet.tsx')) return 'export const ChipSet = () => null;';
        if (path.endsWith('/components/WidgetState.tsx')) return 'export default props => { globalThis.__ledger.state = props; return null; };';
      },
    }] });
  const container = document.appendChild(new FakeNode(1, 'DIV', document));
  const root = createRoot(container as unknown as HTMLElement);
  try {
    const [{ definition }, registry, model, { default: Widget }] = await Promise.all([
      vite.ssrLoadModule('/ui/src/widgets/market-ledger/definition.ts'), vite.ssrLoadModule('/ui/src/dashboard/definitions.ts'),
      vite.ssrLoadModule('/ui/src/dashboard/model.ts'), vite.ssrLoadModule('/ui/src/widgets/market-ledger/MarketLedgerWidget.tsx'),
    ]) as [typeof import('./definition.ts'), typeof import('../../dashboard/definitions.ts'), typeof import('../../dashboard/model.ts'),
      typeof import('./MarketLedgerWidget.tsx')];
    registry.registerWidgets([definition]);
    const branch = { id: '1', sessionId: '1', firstHour: 0, lastHour: 1000, parentId: null, parentHour: null, sampleBoundary: 'Sample origin unknown', gaps: [] };
    const runtime = { tracks: [{ id: 'track', axisOffset: 0, selectedTag: 'SEL', source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'campaign-a', branches: [branch] } }, branch }],
      client: {}, clock: { getHour: () => 1000, getSnapshot: () => ({ playing: false }) } } as unknown as WidgetProps['runtime'];
    const render = (widget: WidgetSpec) => act(async () => {
      root.render(createElement(Widget, { widget, runtime, active: true, countrySlot: {} as HTMLElement, onDiagnostics() {}, setOption() {} } as unknown as WidgetProps));
    });
    // Ask for every page the log shows until no new page opens, as a log scrolled to its end would.
    const scroll = async () => {
      for (let shown = -1; host.__ledger!.log && host.__ledger!.log.pages.length !== shown;) {
        shown = host.__ledger!.log.pages.length;
        const keys = host.__ledger!.log.pages.map(page => page.key);
        await act(async () => { host.__ledger!.log!.onWantedPages(keys); });
      }
    };
    const ids = () => host.__ledger!.log!.pages.flatMap(page => page.rows?.map(row => Number(row.id)) ?? []);
    const widget = model.createWidget('market-ledger', 'track', 'ledger');

    // Progressive preparation: chosen countries page by cursor through 600 occurrences…
    host.__ledger = fixture();
    host.__ledger.prepared = 600;
    widget.countries = { kind: 'tags', tags: ['SEL'] };
    await render(widget);
    await scroll();
    assert.deepEqual(ids(), Array.from({ length: 600 }, (_, index) => 600 - index));
    // …then preparation reaches 700: the first page refreshes, and the pages after it are
    // asked again from its new cursor rather than kept from the old one.
    host.__ledger.prepared = 700;
    await render({ ...widget });
    await scroll();
    assert.deepEqual(ids(), Array.from({ length: 700 }, (_, index) => 700 - index));

    // Every country pages by the cursors the backend returns, not by positions it guesses.
    host.__ledger = fixture();
    host.__ledger.prepared = 600;
    await render({ ...widget, countries: { kind: 'all' } });
    await scroll();
    assert.deepEqual(ids(), Array.from({ length: 600 }, (_, index) => 600 - index));
    assert.ok(host.__ledger.requests.some(request => (request.params as { before?: string }).before === '7:-2147483648:344'));

    // A group asks for its members over their dated memberships, never for every country.
    host.__ledger = fixture();
    host.__ledger.prepared = 900;
    host.__ledger.groups = { branchId: '1', warnings: [], humans: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
      factions: [{ id: 'g', name: 'Synthetic pact', members: [{ tag: 'SEL', firstHour: 100, lastHour: 300 }], incomplete: [] }],
      unobserved: { humans: [], factions: [], wars: [] },
      countryIdentities: [{ tag: 'SEL', identities: [{ firstHour: 0, lastHour: 1000, slot: 1, countryId: 1 }] }],
    } as unknown as CountryGroupReply;
    const group = { ...widget, countries: { kind: 'group', group: { type: 'faction', id: 'g' }, membership: 'hour', aggregate: false } } as WidgetSpec;
    await render(group);
    assert.ok(host.__ledger.requests.length > 0);
    for (const request of host.__ledger.requests) {
      const params = request.params as { tags: string[]; members?: unknown };
      assert.deepEqual(params.tags, []);
      assert.deepEqual(params.members, [{ tag: 'SEL', firstHour: 100, lastHour: 300 }]);
    }
    // A group with no members asks nothing and says so; it is not every country.
    host.__ledger = fixture();
    host.__ledger.prepared = 900;
    host.__ledger.groups = { branchId: '1', warnings: [], humans: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
      factions: [{ id: 'g', name: 'Synthetic pact', members: [], incomplete: [] }], unobserved: { humans: [], factions: [], wars: [] },
      countryIdentities: [] } as unknown as CountryGroupReply;
    await render({ ...group });
    assert.equal(host.__ledger.requests.length, 0);
    assert.equal(host.__ledger.state?.message, 'This group has no members in this campaign.');

    // A date range bounds the ledger by its start as well as its end.
    host.__ledger = fixture();
    host.__ledger.prepared = 900;
    await render({ ...widget, countries: { kind: 'all' }, time: { mode: 'range', firstOffset: 400, lastOffset: 800 } } as WidgetSpec);
    assert.ok(host.__ledger.requests.length > 0);
    for (const request of host.__ledger.requests) assert.deepEqual([(request.params as { firstHour?: number }).firstHour, (request.params as { lastHour: number }).lastHour], [400, 800]);
  } finally {
    await act(async () => { root.unmount(); });
    await vite.close();
    delete host.__ledger;
    for (const [key, descriptor] of before) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor); else delete globals[key];
    }
  }
});
