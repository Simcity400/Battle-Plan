// The every-widget pages: every widget the app offers, each once at its default settings, saved as the app saves
// layouts (its storage format and checks, from the UI's own modules), the sweep writing them into the run's own web view.
// `AllWidgets` holds each at its minimum size, packed into as few grid rows as it can, so all of them fit one 4K screen
// at the largest zoom a user can set; `AllWidgetsLegible` holds each at its default size, as many as fit filling the top
// of a 4K screen at 100%, the rest below to be scrolled to. Each is played on its view (`VIEWS`).
export const FIXTURE = 'AllWidgets';
export const LEGIBLE = 'AllWidgetsLegible';
const UI = new URL('../../ui/src/', import.meta.url);

/** The views, each asked by its name in `--pages`, and the page each plays: `AllWidgets` the minimum-size page wholly
 * on a 4K screen (`screen`: zoomed out only as far as it must be), `AllWidgetsLegible` the default-size page on a 4K
 * screen at 100% (`legible`), `AllWidgetsTall` the default-size page wholly on a view as tall as it at the window's own
 * width and pixel ratio (`tall`, a stress number: more pixels a frame than any screen shows). */
export const VIEWS = {
  AllWidgets: { page: FIXTURE, view: 'screen' },
  AllWidgetsLegible: { page: LEGIBLE, view: 'legible' },
  AllWidgetsTall: { page: LEGIBLE, view: 'tall' },
};
/** The screen of the 4K views, in device pixels. */
export const SCREEN = { width: 3840, height: 2160 };
export const SHELL_HEADER_HEIGHT = 82;
/** The grid rows a 4K screen at 100% shows whole below the app's header: its widgets in view are those within them. */
export const LEGIBLE_ROWS = 64;

const gcd = (a, b) => (b ? gcd(b, a % b) : a);
/** The emulated metrics that show a view at least `height` CSS px tall on all of `screen`'s device pixels: its shape in
 * CSS px and its pixel ratio z, as large as that allows. Its CSS size is the screen's at the smallest whole step that
 * holds `height`, so the device pixels are exactly the screen's. */
export function screenMetrics(height, screen = SCREEN) {
  const step = gcd(screen.width, screen.height);
  const steps = Math.ceil(height / (screen.height / step));
  return { width: steps * screen.width / step, height: steps * screen.height / step, deviceScaleFactor: step / steps };
}

/** The page height needed for the dashboard grid to fit below the desktop shell header. */
export function tallViewportHeight(gridHeight, viewportHeight, shellHeaderHeight = SHELL_HEADER_HEIGHT) {
  return Math.max(viewportHeight, gridHeight + shellHeaderHeight);
}

/** Places items of `{ w, h }` on a grid `cols` wide, each where it sits highest (then leftmost), resting on what is
 * under it as the grid's vertical compaction keeps it: their `{ x, y }`. `skyline` carries on from an earlier packing. */
export function pack(sizes, cols, skyline = new Array(cols).fill(0)) {
  return sizes.map(({ w, h }) => place(skyline, w, h, Infinity));
}

/** Where an item `w` x `h` sits highest on `skyline` if it ends by row `rows`, the skyline raised under it; else null. */
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

/** The fewest rows `sizes` pack into (each where it sits highest), over a few orders: the order and its places. */
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

/** As many of `sizes` as fit in the first `rows` rows (the smallest first), then the rest under them: the order, places
 * and how many lead in view. */
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

/** The pages as the app would save them: `entries`, the storage keys and values the app's own `savePages` and
 * `saveLayouts` write, and `pages`, each page's label, widgets (those planned in view first, for the legible page) and
 * grid rows. A layout the app refuses is a refusal by name. */
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
