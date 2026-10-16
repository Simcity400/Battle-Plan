import type { DashboardClient, DashboardSource, ProductReply, ProductRequest } from './client';
import type { WidgetRuntime } from './definitions';
import { widgetDefinition, widgetDefinitions } from './definitions';
import { createWidget, GRID_COLUMNS } from './model';
import type { DashboardTemplate, WidgetSpec } from './model';
import { parseTemplate } from './storage';
import { validOption } from './options';

export interface ParityHost {
  client: DashboardClient;
  template: DashboardTemplate;
  runtime: WidgetRuntime;
  setTemplate: (template: DashboardTemplate) => void;
  openCampaign: (path: string) => Promise<string | null>;
}

/** Object field order is not a choice; array order and every supplied value are. */
function sameChoices(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (!left || !right || typeof left !== 'object' || typeof right !== 'object') return false;
  if (Array.isArray(left) || Array.isArray(right)) return Array.isArray(left) && Array.isArray(right)
    && left.length === right.length && left.every((value, index) => sameChoices(value, right[index]));
  const a = left as Record<string, unknown>, b = right as Record<string, unknown>;
  return Object.keys(a).length === Object.keys(b).length && Object.keys(a).every(key => Object.hasOwn(b, key) && sameChoices(a[key], b[key]));
}

/** Preserve exact recorded operands and full model rows while omitting runtime
 * callbacks, React ownership and DOM handles. Serialization happens on capture. */
export function parityJson(value: unknown, ancestors = new Set<object>()): unknown {
  if (typeof value === 'bigint') return { bigint: value.toString() };
  if (typeof value === 'number' && !Number.isFinite(value)) return { number: String(value) };
  if (typeof value === 'function' || typeof value === 'symbol' || value === undefined) return undefined;
  if (value === null || typeof value !== 'object') return value;
  if (value instanceof Element) return undefined;
  if (ancestors.has(value)) return undefined;
  const next = new Set(ancestors); next.add(value);
  if (Array.isArray(value)) return value.map(item => parityJson(item, next));
  if (value instanceof Map) return [...value].map(([key, item]) => [parityJson(key, next), parityJson(item, next)]);
  if (value instanceof Set) return [...value].map(item => parityJson(item, next));
  if (ArrayBuffer.isView(value)) return Array.from(value as unknown as ArrayLike<number>);
  const result: Record<string, unknown> = {};
  for (const [key, item] of Object.entries(value)) {
    if (key === '_owner' || key === '_store' || key === 'ref') continue;
    const serialized = parityJson(item, next);
    if (serialized !== undefined) result[key] = serialized;
  }
  return result;
}

export async function prepareParityLists(root: Element, timeout: number) {
  const providers = () => [...(window.__hoi4ParityLists ?? [])]
    .filter(([element]) => element.isConnected && (element === root || root.contains(element))).map(([, provider]) => provider);
  const selected = providers().filter(provider => provider.prepare);
  for (const provider of selected) provider.prepare!();
  const began = performance.now();
  do {
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    const current = new Map(providers().map(provider => [provider.id, provider]));
    let ready = true;
    for (const selectedProvider of selected) {
      const provider = current.get(selectedProvider.id);
      if (!provider) throw new Error('Prepared list unmounted');
      const error = provider.error?.();
      if (error) throw new Error(error);
      if (!provider.ready?.()) ready = false;
    }
    if (ready) return;
  } while (performance.now() - began < timeout);
  throw new Error(`Complete rows did not prepare within ${timeout}ms`);
}

/** Parity builds track the actual transport for readiness. */
export function installParityFixture(host: () => ParityHost): () => void {
  const client = host().client;
  const product = client.product;
  const openSource = client.openSource;
  const preparedQuery = client.preparedQuery;
  const preparationStatus = client.preparationStatus;
  const prepareSource = client.prepareSource;
  const pending = new Map<number, { product: string; sourceId?: string; params?: unknown }>();
  let sequence = 0;
  const track = async <T>(request: { product: string; sourceId?: string; params?: unknown }, run: () => Promise<T>): Promise<T> => {
    const id = ++sequence; pending.set(id, request);
    try { return await run(); } finally { pending.delete(id); }
  };
  const wrappedProduct: DashboardClient['product'] = <T>(request: ProductRequest, signal: AbortSignal) =>
    track(request, () => product.call(client, request, signal) as Promise<ProductReply<T>>);
  const wrappedOpen: DashboardClient['openSource'] = (path, signal) => track({ product: 'open-source' }, () => openSource.call(client, path, signal));
  client.product = wrappedProduct;
  client.openSource = wrappedOpen;
  const wrappedPrepared: DashboardClient['preparedQuery'] = preparedQuery && (<T>(source: DashboardSource, product: string, params: unknown, options: { signal: AbortSignal }) =>
    track({ product, sourceId: source.id, params }, () => preparedQuery.call(client, source, product, params, options) as Promise<T>));
  const wrappedStatus: DashboardClient['preparationStatus'] = preparationStatus && ((request, signal) =>
    track({ product: 'preparation-status', sourceId: request.sourceId }, () => preparationStatus.call(client, request, signal)));
  const wrappedPrepare: DashboardClient['prepareSource'] = prepareSource && ((request, signal) =>
    track({ product: 'prepare-source', sourceId: request.sourceId }, () => prepareSource.call(client, request, signal)));
  if (wrappedPrepared) client.preparedQuery = wrappedPrepared;
  if (wrappedStatus) client.preparationStatus = wrappedStatus;
  if (wrappedPrepare) client.prepareSource = wrappedPrepare;
  const fixture = {
    capabilities: () => ({ scenarioVersion: 1 }),
    registry: () => widgetDefinitions().map(({ kind, title }) => ({ kind, title })),
    open: (path: string) => host().openCampaign(path),
    blank() { const current = host(); current.setTemplate({ ...current.template, widgets: [], layouts: {} }); },
    isolate(kind: string, settings: Partial<Pick<WidgetSpec, 'countries' | 'time' | 'options'>> & { height?: number; hour?: number } = {}) {
      const current = host(), track = current.runtime.tracks.find(track => track.roleId === current.runtime.defaultRoleId);
      const widget = createWidget(kind, track?.id ?? current.template.tracks[0]?.id ?? 'main', `parity-${kind}`);
      if (settings.countries !== undefined) widget.countries = settings.countries;
      if (settings.time !== undefined) widget.time = settings.time;
      if (settings.options !== undefined) {
        for (const [key, value] of Object.entries(settings.options)) {
          const field = widgetDefinition(kind)?.options?.find(field => field.key === key);
          if (!field || !validOption(field, value)) throw new Error(`Invalid parity option ${kind}.${key}`);
        }
        widget.options = { ...widget.options, ...settings.options };
      }
      if (!widgetDefinition(kind)?.timeModes.includes(widget.time.mode)) throw new Error(`Invalid parity time mode for ${kind}`);
      const height = settings.height ?? 18;
      if (!Number.isInteger(height) || height < 1) throw new Error('Parity widget height must be a positive grid row count');
      const next = { ...current.template, name: 'Isolated widget', widgets: [widget],
        layouts: Object.fromEntries(Object.entries(GRID_COLUMNS).map(([breakpoint, width]) => [breakpoint, [{ i: widget.id, x: 0, y: 0, w: width, h: height }]])) };
      const validated = parseTemplate(next)?.widgets[0];
      if (!validated || !sameChoices(validated.countries, widget.countries)
        || !sameChoices(validated.time, widget.time)
        || !sameChoices(validated.options ?? {}, widget.options ?? {})) throw new Error('Parity choices do not match the saved widget schema');
      current.setTemplate(next);
      current.runtime.clock.stop();
      if (settings.hour !== undefined) current.runtime.clock.seek(settings.hour);
      return { id: widget.id, kind };
    },
    seek(hour: number) { host().runtime.clock.seek(hour); return host().runtime.clock.getSnapshot(); },
    snapshot() {
      const current = host(), clock = current.runtime.clock.getSnapshot();
      return { minHour: clock.bounds?.firstHour ?? null, maxHour: clock.bounds?.lastHour ?? null,
        tracks: current.runtime.tracks.map(track => ({ path: track.source.path })) };
    },
    requests: () => [...pending.values()],
    lists: (root: Element = document.body) => [...(window.__hoi4ParityLists ?? [])]
      .filter(([element]) => element.isConnected && (element === root || root.contains(element)))
      // DOM order stays independent of effect completion order.
      .sort(([a], [b]) => a.contains(b) ? 1 : b.contains(a) ? -1
        : a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1)
      .map(([, read]) => parityJson(read())),
    prepareLists: prepareParityLists,
  };
  window.__hoi4Parity = fixture;
  return () => {
    if (client.product === wrappedProduct) client.product = product;
    if (client.openSource === wrappedOpen) client.openSource = openSource;
    if (client.preparedQuery === wrappedPrepared) client.preparedQuery = preparedQuery;
    if (client.preparationStatus === wrappedStatus) client.preparationStatus = preparationStatus;
    if (client.prepareSource === wrappedPrepare) client.prepareSource = prepareSource;
    if (window.__hoi4Parity === fixture) delete window.__hoi4Parity;
  };
}

declare global {
  interface Window {
    __hoi4Parity?: { [key: string]: unknown };
    __hoi4ParityReady?: Promise<void>;
  }
}
