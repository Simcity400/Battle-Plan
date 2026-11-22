export const ENTRY_MS = 700;
export const entryTime = { now: () => performance.now() };
const MOST = 4096;
const STEP = 256;

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

export const entryAlpha = (age: number) => 1 - entryEase(age / ENTRY_MS);

export function backdrop(colours: readonly string[], pixel: CanvasRenderingContext2D): string {
  pixel.globalAlpha = 1;
  pixel.fillStyle = '#fff';
  pixel.fillRect(0, 0, 1, 1);
  for (const colour of [...colours].reverse()) { pixel.fillStyle = colour; pixel.fillRect(0, 0, 1, 1); }
  const [r, g, b] = pixel.getImageData(0, 0, 1, 1).data;
  return `rgb(${r}, ${g}, ${b})`;
}

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

  add(row: HTMLElement, height: number, now: number): void {
    this.entries.push({ row, height, start: now });
  }

  get running(): boolean { return this.entries.length > 0; }

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
