/** Rows entering a revealing log, drawn as one shared layer instead of an animation on each row. The browser restyles
 * every element with a running animation on each frame it renders, composited or not, so 57 rows inside their 700 ms
 * entry at 1,440 h/s cost 200 elements restyled a frame however few rows the frame revealed. Here no row animates: a
 * canvas under the rows draws each entering row's flash, and one over them covers it in the list's own background,
 * clearing as the row would have faded in. The look is the rows' CSS entry exactly: over an opaque backdrop B, a cover
 * of B at alpha 1 − e(t) leaves e(t)·row + (1 − e(t))·B, a row at opacity e(t); the flash under the row's content,
 * seen through the cover, is e(t)·(1 − e(t)) of its colour, as the row's own flash under its fading content is. */

/** The rows' entry: `moving-row-in` and `moving-row-flash` in charts.css. */
export const ENTRY_MS = 700;
/** The tallest stretch of entering rows drawn; rows past it (more than 170 entering at once) appear without the fade. */
const MOST = 4096;
/** Canvas heights grow in steps, so a stretch growing a row a frame does not reallocate every frame. */
const STEP = 256;

/** `cubic-bezier(.2, .8, .2, 1)` at progress `t` (0 to 1), as the browser eases the CSS entry. */
export function entryEase(t: number): number {
  if (t <= 0) return 0;
  if (t >= 1) return 1;
  const [x1, y1, x2, y2] = [0.2, 0.8, 0.2, 1];
  const curve = (s: number, a: number, b: number) => ((1 - 3 * b + 3 * a) * s + (3 * b - 6 * a)) * s * s + 3 * a * s;
  let low = 0, high = 1, s = t;
  for (let step = 0; step < 32; step++) {
    const x = curve(s, x1, x2);
    if (Math.abs(x - t) < 1e-7) break;
    if (x < t) low = s; else high = s;
    s = (low + high) / 2;
  }
  return curve(s, y1, y2);
}

/** What `paint` draws an entry with: the cover's and the flash's alpha at `age` ms. */
export const entryAlpha = (age: number) => 1 - entryEase(age / ENTRY_MS);

/** The colour a stack of backgrounds shows, top first: each translucent one over those under it, down to the first
 * opaque one (white where none is), resolved by drawing them on a pixel. */
export function backdrop(colours: readonly string[], pixel: CanvasRenderingContext2D): string {
  pixel.globalAlpha = 1;
  pixel.fillStyle = '#fff';
  pixel.fillRect(0, 0, 1, 1);
  for (const colour of [...colours].reverse()) { pixel.fillStyle = colour; pixel.fillRect(0, 0, 1, 1); }
  const [r, g, b] = pixel.getImageData(0, 0, 1, 1).data;
  return `rgb(${r}, ${g}, ${b})`;
}

/** The backgrounds behind `element`, nearest first, down to the first opaque one. */
export function backgroundsBehind(element: Element): string[] {
  const out: string[] = [];
  for (let at: Element | null = element; at; at = at.parentElement) {
    const colour = getComputedStyle(at).backgroundColor;
    if (!colour || colour === 'transparent' || /^rgba\([^)]*,\s*0\)$/.test(colour)) continue;
    out.push(colour);
    if (!/^rgba|\/\s*0?\.\d/.test(colour)) break;
  }
  return out;
}

interface Entry { row: HTMLElement; height: number; start: number }

/** The two canvases and the rows entering under them; drawn each frame while any row is inside its entry. */
export class EntryLayer {
  private entries: Entry[] = [];
  private placed = NaN;
  private size = { width: 0, height: 0, ratio: 0 };
  cover = '#fff';
  flash = 'transparent';

  private readonly under: HTMLCanvasElement;
  private readonly over: HTMLCanvasElement;

  constructor(under: HTMLCanvasElement, over: HTMLCanvasElement) {
    this.under = under;
    this.over = over;
  }

  /** `row`, `height` px tall, enters now. */
  add(row: HTMLElement, height: number, now: number): void {
    this.entries.push({ row, height, start: now });
  }

  get running(): boolean { return this.entries.length > 0; }

  /** Draws every row still entering at `now`, at its place in the glide (`top`: none for a row no longer drawn), the
   * canvases `width` px wide; whether any row is still entering. Writes two canvases' transforms, never a row. */
  paint(now: number, top: (row: HTMLElement) => number | undefined, width: number): boolean {
    let first = Infinity, last = -Infinity;
    const live: { at: number; height: number; alpha: number }[] = [];
    this.entries = this.entries.filter(entry => {
      const age = now - entry.start, at = entry.row.isConnected ? top(entry.row) : undefined;
      if (age >= ENTRY_MS || at === undefined) return false;
      live.push({ at, height: entry.height, alpha: entryAlpha(age) });
      first = Math.min(first, at);
      last = Math.max(last, at + entry.height);
      return true;
    });
    const shown = live.length > 0;
    for (const canvas of [this.under, this.over]) if (canvas.hidden === shown) canvas.hidden = !shown;
    if (!shown) { this.placed = NaN; return false; }
    const ratio = window.devicePixelRatio || 1;
    const height = Math.min(MOST, Math.ceil((last - first) / STEP) * STEP || STEP);
    if (this.size.width !== width || this.size.height !== height || this.size.ratio !== ratio) {
      this.size = { width, height, ratio };
      for (const canvas of [this.under, this.over]) {
        canvas.width = Math.max(1, Math.round(width * ratio));
        canvas.height = Math.round(height * ratio);
        canvas.style.height = `${height}px`;
      }
    }
    if (first !== this.placed) {
      for (const canvas of [this.under, this.over]) canvas.style.transform = `translateY(${first}px)`;
      this.placed = first;
    }
    for (const [canvas, colour] of [[this.under, this.flash], [this.over, this.cover]] as const) {
      const context = canvas.getContext('2d');
      if (!context) continue;
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
      context.clearRect(0, 0, width, height);
      context.fillStyle = colour;
      for (const entry of live) {
        if (entry.at - first >= height) continue;
        context.globalAlpha = entry.alpha;
        context.fillRect(0, entry.at - first, width, entry.height);
      }
    }
    return true;
  }

  clear(): void {
    this.entries = [];
    this.placed = NaN;
    for (const canvas of [this.under, this.over]) canvas.hidden = true;
  }
}
