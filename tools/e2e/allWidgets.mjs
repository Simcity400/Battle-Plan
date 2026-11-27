
import { rankWidgetCosts } from './datedPlayback.mjs';

export const FIXTURE = 'AllWidgets';
export const LEGIBLE = 'AllWidgetsLegible';
export const REBUILT = 'RebuiltWidgets';
export const REBUILT_ALONE = 'RebuiltAlone';
export const HEAVIEST = 'Heaviest';
export const HEAVIEST_COUNT = 30;
export const COST_EMPTY = 'CostEmpty';
export const REBUILT_KINDS = ['occupied-states', 'scoreboard', 'research', 'active-research', 'special-projects', 'doctrine-mastery', 'nuclear-arsenal'];
const UI = new URL('../../ui/src/', import.meta.url);

export const VIEWS = {
  AllWidgets: { page: FIXTURE, view: 'screen' },
  AllWidgetsLegible: { page: LEGIBLE, view: 'legible' },
  AllWidgetsTall: { page: LEGIBLE, view: 'tall' },
  RebuiltWidgets: { page: REBUILT, view: 'screen' },
  RebuiltAlone: { page: REBUILT_ALONE, view: 'screen' },
  Heaviest: { page: HEAVIEST, view: 'screen' },
};
export const SCREEN = { width: 3840, height: 2160 };
export const SHELL_HEADER_HEIGHT = 82;
export const LEGIBLE_ROWS = 64;

const gcd = (a, b) => (b ? gcd(b, a % b) : a);
export function screenMetrics(height, screen = SCREEN) {
  const step = gcd(screen.width, screen.height);
  const steps = Math.ceil(height / (screen.height / step));
  return { width: steps * screen.width / step, height: steps * screen.height / step, deviceScaleFactor: step / steps };
}

export function tallViewportHeight(gridHeight, viewportHeight, shellHeaderHeight = SHELL_HEADER_HEIGHT) {
  return Math.max(viewportHeight, gridHeight + shellHeaderHeight);
}

export function pack(sizes, cols, skyline = new Array(cols).fill(0)) {
  return sizes.map(({ w, h }) => place(skyline, w, h, Infinity));
}

function place(skyline, w, h, rows) {
  if (w > skyline.length) throw new Error(`an item ${w} wide does not fit ${skyline.length} columns`);
  let best = null;
  for (let x = 0; x + w <= skyline.length; x++) {
    const y = Math.max(...skyline.slice(x, x + w));
    if (!best || y < best.y) best = { x, y };
  }
  if (best.y + h > rows) return null;
  skyline.fill(best.y + h, best.x, best.x + w);
  return best;
}

const rowsOf = (sizes, places) => Math.max(...places.map(({ y }, at) => y + sizes[at].h));

export function tightest(sizes, cols) {
  const orders = [(a, b) => b.w - a.w || b.h - a.h, (a, b) => b.h - a.h || b.w - a.w, (a, b) => b.w * b.h - a.w * a.h || b.w - a.w];
  let best = null;
  for (const order of orders) {
    const sorted = [...sizes].sort(order);
    const places = pack(sorted, cols);
    if (!best || rowsOf(sorted, places) < rowsOf(best.sorted, best.places)) best = { sorted, places };
  }
  return best;
}

export function legible(sizes, cols, rows) {
  const skyline = new Array(cols).fill(0);
  const shown = [], placesShown = [], rest = [];
  for (const item of [...sizes].sort((a, b) => a.w * a.h - b.w * b.h || a.h - b.h)) {
    const at = place(skyline, item.w, item.h, rows);
    if (at) { shown.push(item); placesShown.push(at); } else rest.push(item);
  }
  rest.sort((a, b) => b.w - a.w || b.h - a.h);
  return { sorted: [...shown, ...rest], places: [...placesShown, ...pack(rest, cols, skyline)], inView: shown.length };
}

export function selectHeaviestKinds(costs, count = HEAVIEST_COUNT) {
  const list = Array.isArray(costs) ? costs : costs?.widgets;
  if (!Array.isArray(list)) throw new Error('widget-costs.json holds no widgets array');
  const limit = Math.max(1, Math.floor(count));
  const measured = list.filter(entry => entry && typeof entry.kind === 'string' && Number.isFinite(entry.costScoreMs));
  const ranked = rankWidgetCosts(measured);
  const distinctEntries = [];
  const seen = new Set();
  for (const entry of ranked) {
    if (seen.has(entry.kind)) continue;
    seen.add(entry.kind);
    distinctEntries.push(entry);
  }
  if (distinctEntries.length < limit) {
    throw new Error(`Heaviest requested ${limit} kinds, but widget-costs.json has only ${distinctEntries.length} measured kinds with a valid costScoreMs`);
  }
  const selected = distinctEntries.slice(0, limit);
  const selectedKinds = new Set(selected.map(entry => entry.kind));
  const cutoff = selected.at(-1);
  const noiseByKind = new Map();
  for (const entry of ranked) {
    if (selectedKinds.has(entry.kind)) continue;
    const delta = entry.mainDelta?.p95;
    if (!(Number.isFinite(delta) && delta < 0) && !entry.belowBaselineNoise) continue;
    const zeroClampedScore = Math.max(0, delta ?? 0) + (entry.worker?.p95 ?? 0);
    if (zeroClampedScore < cutoff.costScoreMs) continue;
    const prev = noiseByKind.get(entry.kind);
    if (!prev || zeroClampedScore > prev.zeroClampedScore) noiseByKind.set(entry.kind, { entry, zeroClampedScore });
  }
  const excludedByNoise = [...noiseByKind.values()].map(item => item.entry);
  const label = entry => (entry.view && entry.view !== 'default' ? `${entry.kind}/${entry.view}` : entry.kind);
  const warning = excludedByNoise.length
    ? `Heaviest cutoff (${label(cutoff)} at ${cutoff.costScoreMs.toFixed(1)} ms) excluded ${excludedByNoise.length} kind${excludedByNoise.length === 1 ? '' : 's'} below baseline noise: ${excludedByNoise.map(e => `${label(e)} (${e.costScoreMs.toFixed(1)} ms)`).join(', ')}`
    : null;
  const entries = selected.map(entry => ({ kind: entry.kind, ...(typeof entry.view === 'string' && entry.view ? { view: entry.view } : {}) }));
  return { kinds: selected.map(entry => entry.kind), entries, excludedByNoise, warning };
}

export function heaviestKinds(costs, count = HEAVIEST_COUNT, { allowBelowNoiseCutoff = false } = {}) {
  const { kinds, excludedByNoise, warning } = selectHeaviestKinds(costs, count);
  if (excludedByNoise.length && !allowBelowNoiseCutoff) throw new Error(warning);
  return kinds;
}

function createPlacedWidget(model, placement, id) {
  const widget = model.createWidget(placement.kind, 'main', id);
  return placement.options ? { ...widget, options: { ...placement.options } } : widget;
}

async function loadFixtureModules() {
  const [{ widgetMetas }, { FAMILY_VIEWS }, families, model, storage, pageList] = await Promise.all([
    import(new URL('widgets/definitions.ts', UI)), import(new URL('widgets/familyManifest.ts', UI)),
    import(new URL('families/manifest.ts', UI)), import(new URL('dashboard/model.ts', UI)),
    import(new URL('dashboard/storage.ts', UI)), import(new URL('dashboard/pages.ts', UI))]);
  return { widgetMetas, FAMILY_VIEWS, families, model, storage, pageList };
}

export async function enumerateWidgetPlacements({ mode = 'default', kinds = null } = {}) {
  const { widgetMetas, FAMILY_VIEWS, families } = await loadFixtureModules();
  const familyViewsByKind = new Map();
  for (const entry of FAMILY_VIEWS) {
    let list = familyViewsByKind.get(entry.kind);
    if (!list) familyViewsByKind.set(entry.kind, list = []);
    list.push(entry);
  }
  const byKind = new Map(widgetMetas.map(meta => [meta.kind, meta]));
  const selected = kinds === null ? widgetMetas : kinds.map(kind => {
    const meta = byKind.get(kind);
    if (!meta) throw new Error(`unknown widget kind: ${kind}`);
    return meta;
  });
  return selected.flatMap(meta => {
    const entries = familyViewsByKind.get(meta.kind) ?? [];
    const defaultView = entries.length ? families.resolveActiveView(undefined, meta, entries) : families.DEFAULT_VIEW;
    const optionKey = entries.length > 1 ? families.resolveViewOptionKey(meta, entries) : null;
    const choices = entries.length ? families.declaredViewChoices(meta, entries) : [{ value: families.DEFAULT_VIEW, label: families.DEFAULT_VIEW, family: null }];
    const picked = mode === 'all-views' ? choices : [choices.find(choice => choice.value === defaultView) ?? choices[0]];
    return picked.map(choice => {
      const isDefaultView = choice.value === defaultView;
      const options = !isDefaultView && optionKey ? { [optionKey]: choice.value } : undefined;
      return { meta, kind: meta.kind, view: choice.value, viewLabel: entries.length > 1 ? choice.label : null, family: choice.family, isDefaultView, ...(options ? { options } : {}) };
    });
  });
}

export async function allWidgetPages({ costs = null, heaviestCount = HEAVIEST_COUNT, allowBelowNoiseCutoff = false, allViews = false } = {}) {
  const { widgetMetas, model, storage, pageList } = await loadFixtureModules();
  const cols = model.GRID_COLUMNS.lg;
  if (model.GRID_COLUMNS.md !== cols) throw new Error('the wide breakpoints have different columns: the fixture packs one');
  const mode = allViews ? 'all-views' : 'default';
  const placements = await enumerateWidgetPlacements({ mode });
  const written = new Map();
  const shim = { getItem: key => written.get(key) ?? null, setItem: (key, value) => written.set(key, value) };
  const page = (label, size, arrange, entries = placements) => {
    const id = `page-${label.toLowerCase()}`;
    const sizes = entries.map(entry => ({ ...entry, ...size(entry.meta), min: entry.meta.minSize }));
    const { sorted, places, inView = sorted.length } = arrange(sizes);
    const widgets = sorted.map(entry => createPlacedWidget(model, entry,
      `${id}-${entry.kind}${entry.isDefaultView ? '' : `--${entry.view}`}${entry.copy === undefined ? '' : `-${entry.copy}`}`));
    const wide = places.map(({ x, y }, at) => ({ i: widgets[at].id, x, y, w: sorted[at].w, h: sorted[at].h, minW: sorted[at].min.w, minH: sorted[at].min.h }));
    const narrow = width => {
      let y = 0;
      return sorted.map(({ h, min }, at) => {
        const item = { i: widgets[at].id, x: 0, y, w: width, h, minW: Math.min(width, min.w), minH: min.h };
        y += h;
        return item;
      });
    };
    const template = { ...model.defaultTemplate(id), widgets, layouts: { lg: wide, md: wide, sm: narrow(model.GRID_COLUMNS.sm), xs: narrow(model.GRID_COLUMNS.xs) } };
    const refused = storage.saveLayouts({ version: 1, activeId: 'default', layouts: [{ id: 'default', template }] }, shim, id);
    if (refused) throw new Error(`the app refuses the ${label} layout: ${refused}`);
    return { id, label, rows: wide.length ? Math.max(...wide.map(item => item.y + item.h)) : 0, inView,
      widgets: widgets.map((widget, at) => ({ id: widget.id, kind: widget.kind, view: sorted[at].view })) };
  };
  const rebuilt = await enumerateWidgetPlacements({ kinds: REBUILT_KINDS });
  const pages = {
    [FIXTURE]: page(FIXTURE, meta => meta.minSize, sizes => tightest(sizes, cols)),
    [LEGIBLE]: page(LEGIBLE, meta => meta.defaultSize, sizes => legible(sizes, cols, LEGIBLE_ROWS)),
    [REBUILT]: page(REBUILT, meta => meta.minSize, sizes => tightest(sizes, cols),
      Array.from({ length: widgetMetas.length }, (_, copy) => ({ ...rebuilt[copy % rebuilt.length], copy }))),
    [REBUILT_ALONE]: page(REBUILT_ALONE, meta => meta.defaultSize, sizes => tightest(sizes, cols), rebuilt.map((entry, copy) => ({ ...entry, copy }))),
  };
  if (costs) {
    const selection = selectHeaviestKinds(costs, heaviestCount);
    if (selection.excludedByNoise.length && !allowBelowNoiseCutoff) throw new Error(selection.warning);
    const pool = await enumerateWidgetPlacements({ kinds: selection.kinds, mode: 'all-views' });
    const selected = selection.entries.map(({ kind, view }) => {
      const forKind = pool.filter(entry => entry.kind === kind);
      if (view) {
        const matched = forKind.find(entry => entry.view === view);
        if (!matched) throw new Error(`unknown view ${view} for widget kind ${kind}`);
        return matched;
      }
      return forKind.find(entry => entry.isDefaultView) ?? forKind[0];
    });
    pages[HEAVIEST] = {
      ...page(HEAVIEST, meta => meta.minSize, sizes => tightest(sizes, cols), selected),
      excludedByNoise: selection.excludedByNoise.map(e => e.kind),
      warning: selection.warning,
    };
  }
  const listed = [...pageList.defaultPages(), ...Object.values(pages).map(({ id, label }) => ({ id, label }))];
  const unlisted = pageList.savePages(listed, shim);
  if (!pageList.parsePageList(listed) || unlisted) throw new Error(`the app refuses the page list holding ${Object.keys(pages).join(', ')}: ${unlisted}`);
  return { entries: [...written], pages };
}

export async function costPages(kinds = null, { allViews = false } = {}) {
  const { model, storage, pageList } = await loadFixtureModules();
  const selected = await enumerateWidgetPlacements({ kinds, mode: allViews ? 'all-views' : 'default' });
  const written = new Map();
  const shim = { getItem: key => written.get(key) ?? null, setItem: (key, value) => written.set(key, value) };
  const emptyId = 'page-cost-empty';
  const emptyTemplate = { ...model.defaultTemplate(emptyId), widgets: [], layouts: { lg: [], md: [], sm: [], xs: [] } };
  const emptyRefused = storage.saveLayouts({ version: 1, activeId: 'default', layouts: [{ id: 'default', template: emptyTemplate }] }, shim, emptyId);
  if (emptyRefused) throw new Error(`the app refuses the ${COST_EMPTY} layout: ${emptyRefused}`);
  const empty = { id: emptyId, label: COST_EMPTY };
  const built = [];
  for (const entry of selected) {
    const { meta } = entry;
    const id = entry.isDefaultView ? `page-cost-${entry.kind}` : `page-cost-${entry.kind}--${entry.view}`;
    const label = entry.isDefaultView ? `Cost:${entry.kind}` : `Cost:${entry.kind}/${entry.view}`;
    const widget = createPlacedWidget(model, entry, entry.isDefaultView ? `${id}-${entry.kind}` : `${id}-w`);
    const wide = [{ i: widget.id, x: 0, y: 0, w: meta.defaultSize.w, h: meta.defaultSize.h, minW: meta.minSize.w, minH: meta.minSize.h }];
    const narrow = width => [{ i: widget.id, x: 0, y: 0, w: width, h: meta.defaultSize.h, minW: Math.min(width, meta.minSize.w), minH: meta.minSize.h }];
    const template = { ...model.defaultTemplate(id), widgets: [widget], layouts: { lg: wide, md: wide, sm: narrow(model.GRID_COLUMNS.sm), xs: narrow(model.GRID_COLUMNS.xs) } };
    const refused = storage.saveLayouts({ version: 1, activeId: 'default', layouts: [{ id: 'default', template }] }, shim, id);
    if (refused) throw new Error(`the app refuses the ${label} layout: ${refused}`);
    built.push({ id, label, kind: entry.kind, view: entry.view, isDefaultView: entry.isDefaultView, title: meta.title, widget: widget.id });
  }
  const listed = [empty, ...built.map(({ id, label }) => ({ id, label }))];
  const unlisted = pageList.savePages(listed, shim);
  if (!pageList.parsePageList(listed) || unlisted) throw new Error(`the app refuses the cost page list: ${unlisted}`);
  return { entries: [...written], empty, kinds: built };
}
