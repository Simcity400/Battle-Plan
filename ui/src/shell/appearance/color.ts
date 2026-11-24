
export interface Rgb { r: number; g: number; b: number }
export interface Oklch { l: number; c: number; h: number }

const clamp = (value: number, min = 0, max = 1) => Math.min(max, Math.max(min, value));
const MAX_COLOR_CACHE = 1024;
const parseCache = new Map<string, { readonly rgb: Rgb; readonly alpha: number } | null>();
const mixCache = new Map<string, string>();
const rgbaCache = new Map<string, string>();
const oklchRgbCache = new Map<string, Rgb>();
let parseMisses = 0;
let mixMisses = 0;
let rgbaMisses = 0;
let oklchToRgbMisses = 0;

function cacheSet<V>(map: Map<string, V>, key: string, value: V): V {
  if (map.size >= MAX_COLOR_CACHE) map.clear();
  map.set(key, value);
  return value;
}

export const colorCache = {
  get parses() { return parseMisses; },
  get mixes() { return mixMisses; },
  get rgba() { return rgbaMisses; },
  get oklchToRgb() { return oklchToRgbMisses; },
  clear(): void {
    parseCache.clear();
    mixCache.clear();
    rgbaCache.clear();
    oklchRgbCache.clear();
    parseMisses = 0;
    mixMisses = 0;
    rgbaMisses = 0;
    oklchToRgbMisses = 0;
  },
};

export function parseColorAlpha(value: unknown): { rgb: Rgb; alpha: number } | null {
  if (typeof value !== 'string') return null;
  const cached = parseCache.get(value);
  if (cached !== undefined) return cached;
  parseMisses++;
  const text = value.trim().toLowerCase();
  const hex = /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/.exec(text);
  if (hex) {
    const digits = hex[1].length <= 4 ? [...hex[1]].map(digit => digit + digit).join('') : hex[1];
    const channel = (index: number) => parseInt(digits.slice(index * 2, index * 2 + 2), 16);
    return cacheSet(parseCache, value, { rgb: { r: channel(0), g: channel(1), b: channel(2) }, alpha: digits.length === 8 ? channel(3) / 255 : 1 });
  }
  const functional = /^rgba?\(\s*([^)]*)\)$/.exec(text);
  if (!functional) return cacheSet(parseCache, value, null);
  const parts = functional[1].split(/\s*[,/]\s*|\s+/).filter(Boolean);
  if (parts.length < 3 || parts.length > 4) return cacheSet(parseCache, value, null);
  const numbers = parts.map((part, index) => {
    const percent = part.endsWith('%');
    const number = Number(percent ? part.slice(0, -1) : part);
    if (!Number.isFinite(number)) return NaN;
    return index < 3 ? (percent ? number * 2.55 : number) : (percent ? number / 100 : number);
  });
  if (numbers.some(Number.isNaN)) return cacheSet(parseCache, value, null);
  return cacheSet(parseCache, value, { rgb: { r: clamp(numbers[0], 0, 255), g: clamp(numbers[1], 0, 255), b: clamp(numbers[2], 0, 255) }, alpha: clamp(numbers[3] ?? 1) });
}

export function parseColor(value: unknown): Rgb | null {
  return parseColorAlpha(value)?.rgb ?? null;
}

export function toHex({ r, g, b }: Rgb): string {
  return `#${[r, g, b].map(channel => Math.round(clamp(channel, 0, 255)).toString(16).padStart(2, '0')).join('')}`;
}

export function normalizeColor(value: unknown, under?: string): string | null {
  const parsed = parseColorAlpha(value);
  if (!parsed) return null;
  const base = under ? parseColor(under) : null;
  return toHex(base && parsed.alpha < 1 ? mixRgb(base, parsed.rgb, parsed.alpha) : parsed.rgb);
}

export const HEX_HINT = 'Use a hex colour like #346bf1.';

export function colorInput(text: string): string | null {
  const trimmed = text.trim();
  return normalizeColor(/^[0-9a-f]{3,8}$/i.test(trimmed) ? `#${trimmed}` : trimmed);
}

export function mixRgb(from: Rgb, to: Rgb, amount: number): Rgb {
  const t = clamp(amount);
  return { r: from.r + (to.r - from.r) * t, g: from.g + (to.g - from.g) * t, b: from.b + (to.b - from.b) * t };
}

export function mix(from: string, to: string, amount: number): string {
  const key = `${from}\0${to}\0${amount}`;
  const cached = mixCache.get(key);
  if (cached !== undefined) return cached;
  mixMisses++;
  return cacheSet(mixCache, key, toHex(mixRgb(parseColor(from) ?? { r: 0, g: 0, b: 0 }, parseColor(to) ?? { r: 0, g: 0, b: 0 }, amount)));
}

export function rgbaRgb(rgb: Rgb, alpha: number): string {
  return `rgba(${Math.round(rgb.r)}, ${Math.round(rgb.g)}, ${Math.round(rgb.b)}, ${alpha})`;
}

export function rgbaColor(color: string, alpha: number, fallback: string): string {
  const key = `${color}\0${alpha}\0${fallback}`;
  const cached = rgbaCache.get(key);
  if (cached !== undefined) return cached;
  rgbaMisses++;
  const rgb = parseColor(color);
  return cacheSet(rgbaCache, key, rgb ? rgbaRgb(rgb, alpha) : fallback);
}

export function tintColour(color: string, amount: number): string {
  return mix(color, '#ffffff', amount);
}

export function shadeColour(color: string, amount: number): string {
  return mix(color, '#000000', amount);
}

export function surfaceMix(surface: string, amount: number): string {
  return mix(surface, '#f5f5f5', amount);
}

const LINEAR_BYTE = Float64Array.from({ length: 256 }, (_, i) => {
  const value = i / 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
});
const toLinear = (channel: number) => {
  if ((channel | 0) === channel && channel >= 0 && channel <= 255) return LINEAR_BYTE[channel]!;
  const value = channel / 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
};
const fromLinear = (value: number) => 255 * (value <= 0.0031308 ? value * 12.92 : 1.055 * value ** (1 / 2.4) - 0.055);

export function luminance({ r, g, b }: Rgb): number {
  return 0.2126 * toLinear(r) + 0.7152 * toLinear(g) + 0.0722 * toLinear(b);
}

export function contrast(first: Rgb, second: Rgb): number {
  const [light, dark] = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return (light + 0.05) / (dark + 0.05);
}

export function isDark(color: Rgb): boolean {
  return luminance(color) < 0.179;
}

export function rgbToOklch({ r, g, b }: Rgb): Oklch {
  const [lr, lg, lb] = [toLinear(r), toLinear(g), toLinear(b)];
  const l = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb);
  const m = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb);
  const s = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  const hue = (Math.atan2(B, A) * 180) / Math.PI;
  return { l: L, c: Math.hypot(A, B), h: hue < 0 ? hue + 360 : hue };
}

function oklchToLinear({ l, c, h }: Oklch): [number, number, number] {
  const angle = (h * Math.PI) / 180;
  const A = c * Math.cos(angle), B = c * Math.sin(angle);
  const lp = (l + 0.3963377774 * A + 0.2158037573 * B) ** 3;
  const mp = (l - 0.1055613458 * A - 0.0638541728 * B) ** 3;
  const sp = (l - 0.0894841775 * A - 1.291485548 * B) ** 3;
  return [
    4.0767416621 * lp - 3.3077115913 * mp + 0.2309699292 * sp,
    -1.2684380046 * lp + 2.6097574011 * mp - 0.3413193965 * sp,
    -0.0041960863 * lp - 0.7034186147 * mp + 1.707614701 * sp,
  ];
}

export function oklchToRgb(color: Oklch): Rgb {
  const key = `${color.l}\0${color.c}\0${color.h}`;
  const cached = oklchRgbCache.get(key);
  if (cached !== undefined) return cached;
  oklchToRgbMisses++;
  const target = { ...color, l: clamp(color.l), c: Math.max(0, color.c) };
  let low = 0, high = target.c;
  let linear = oklchToLinear(target);
  const inGamut = (values: number[]) => values.every(value => value >= -0.0005 && value <= 1.0005);
  if (!inGamut(linear)) {
    for (let step = 0; step < 18; step++) {
      const middle = (low + high) / 2;
      if (inGamut(oklchToLinear({ ...target, c: middle }))) low = middle; else high = middle;
    }
    linear = oklchToLinear({ ...target, c: low });
  }
  const [r, g, b] = linear.map(value => fromLinear(clamp(value)));
  return cacheSet(oklchRgbCache, key, { r, g, b });
}

export function mixOklab(from: Rgb, to: Rgb, amount: number): Rgb {
  const t = clamp(amount);
  const lab = (color: Rgb) => { const { l, c, h } = rgbToOklch(color); const angle = (h * Math.PI) / 180; return [l, c * Math.cos(angle), c * Math.sin(angle)]; };
  const [l1, a1, b1] = lab(from), [l2, a2, b2] = lab(to);
  const [l, a, b] = [l1 + (l2 - l1) * t, a1 + (a2 - a1) * t, b1 + (b2 - b1) * t];
  const hue = (Math.atan2(b, a) * 180) / Math.PI;
  return oklchToRgb({ l, c: Math.hypot(a, b), h: hue < 0 ? hue + 360 : hue });
}

export function toHexAlpha(color: Rgb, alpha: number): string {
  const opacity = Math.round(clamp(alpha) * 255);
  return opacity === 255 ? toHex(color) : `${toHex(color)}${opacity.toString(16).padStart(2, '0')}`;
}

export function oklchHex(color: Oklch): string {
  return toHex(oklchToRgb(color));
}

export function solveLightness(color: Oklch, against: Rgb, ratio: number, direction: 'lighter' | 'darker'): Oklch {
  const step = direction === 'lighter' ? 0.01 : -0.01;
  let current = { ...color };
  for (let index = 0; index < 100 && contrast(oklchToRgb(current), against) < ratio; index++) {
    const next = clamp(current.l + step);
    if (next === current.l) break;
    current = { ...current, l: next };
  }
  return current;
}

export function readableText(background: Rgb, dark = '#0b0f14', light = '#ffffff'): string {
  const darkRgb = parseColor(dark)!, lightRgb = parseColor(light)!;
  return contrast(background, lightRgb) >= contrast(background, darkRgb) ? light : dark;
}

export interface Hsv { h: number; s: number; v: number }

export function rgbToHsv({ r, g, b }: Rgb): Hsv {
  const [red, green, blue] = [r / 255, g / 255, b / 255];
  const max = Math.max(red, green, blue), min = Math.min(red, green, blue), delta = max - min;
  let h = 0;
  if (delta) h = max === red ? ((green - blue) / delta) % 6 : max === green ? (blue - red) / delta + 2 : (red - green) / delta + 4;
  h = (h * 60 + 360) % 360;
  return { h, s: max ? delta / max : 0, v: max };
}

export function hsvToRgb({ h, s, v }: Hsv): Rgb {
  const chroma = v * s, x = chroma * (1 - Math.abs(((h / 60) % 2) - 1)), m = v - chroma;
  const [r, g, b] = h < 60 ? [chroma, x, 0] : h < 120 ? [x, chroma, 0] : h < 180 ? [0, chroma, x] : h < 240 ? [0, x, chroma] : h < 300 ? [x, 0, chroma] : [chroma, 0, x];
  return { r: (r + m) * 255, g: (g + m) * 255, b: (b + m) * 255 };
}
