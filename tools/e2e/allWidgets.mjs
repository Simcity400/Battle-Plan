
export const FIXTURE = 'AllWidgets';
export const LEGIBLE = 'AllWidgetsLegible';
const UI = new URL('../../ui/src/', import.meta.url);

export const VIEWS = {
  AllWidgets: { page: FIXTURE, view: 'screen' },
  AllWidgetsLegible: { page: LEGIBLE, view: 'legible' },
  AllWidgetsTall: { page: LEGIBLE, view: 'tall' },
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

export async function allWidgetPages() {
  const [{ widgetMetas }, model, storage, pageList] = await Promise.all([
    import(new URL('widgets/definitions.ts', UI)), import(new URL('dashboard/model.ts', UI)),
    import(new URL('dashboard/storage.ts', UI)), import(new URL('dashboard/pages.ts', UI))]);
  const cols = model.GRID_COLUMNS.lg;
  if (model.GRID_COLUMNS.md !== cols) throw new Error('the wide breakpoints have different columns: the fixture packs one');
  const written = new Map();
  const shim = { getItem: key => written.get(key) ?? null, setItem: (key, value) => written.set(key, value) };
  const page = (label, size, arrange) => {
    const id = `page-${label.toLowerCase()}`;
    const sizes = widgetMetas.map(meta => ({ kind: meta.kind, ...size(meta), min: meta.minSize }));
    const { sorted, places, inView = sorted.length } = arrange(sizes);
    const widgets = sorted.map(({ kind }) => model.createWidget(kind, 'main', `${id}-${kind}`));
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
    return { id, label, rows: Math.max(...wide.map(item => item.y + item.h)), inView,
      widgets: widgets.map(widget => ({ id: widget.id, kind: widget.kind })) };
  };
  const pages = {
    [FIXTURE]: page(FIXTURE, meta => meta.minSize, sizes => tightest(sizes, cols)),
    [LEGIBLE]: page(LEGIBLE, meta => meta.defaultSize, sizes => legible(sizes, cols, LEGIBLE_ROWS)),
  };
  const listed = [...pageList.defaultPages(), ...Object.values(pages).map(({ id, label }) => ({ id, label }))];
  const unlisted = pageList.savePages(listed, shim);
  if (!pageList.parsePageList(listed) || unlisted) throw new Error(`the app refuses the page list holding ${Object.keys(pages).join(', ')}: ${unlisted}`);
  return { entries: [...written], pages };
}
