
export interface EndLabel { id: string; y: number; height: number }

export function layoutEndLabels(labels: readonly EndLabel[], top: number, bottom: number, gap = 2): Map<string, number> {
  const sorted = [...labels].sort((a, b) => a.y - b.y || a.id.localeCompare(b.id));
  const placed = sorted.map(label => ({ ...label, at: label.y }));
  let floor = top;
  for (const label of placed) {
    label.at = Math.max(label.at, floor + label.height / 2);
    floor = label.at + label.height / 2 + gap;
  }
  let ceiling = bottom;
  for (let index = placed.length - 1; index >= 0; index--) {
    const label = placed[index];
    label.at = Math.min(label.at, ceiling - label.height / 2);
    ceiling = label.at - label.height / 2 - gap;
  }
  floor = top;
  for (const label of placed) {
    label.at = Math.max(label.at, floor + label.height / 2);
    floor = label.at + label.height / 2 + gap;
  }
  return new Map(placed.map(label => [label.id, label.at]));
}

export function fitLabelWidth<T = string>(texts: Iterable<T>, measure: (text: T) => number, min: number, max: number, padding = 0): number {
  let widest = 0;
  for (const text of texts) {
    widest = Math.max(widest, measure(text));
    if (widest + padding >= max) break;
  }
  return Math.ceil(Math.min(max, Math.max(min, widest + padding)));
}

const ELLIPSIS = '\u2026';
const GRAPHEMES = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

export function graphemes(text: string): string[] {
  return Array.from(GRAPHEMES.segment(text), part => part.segment);
}

export function graphemeCount(text: string): number {
  let count = 0;
  for (const _ of GRAPHEMES.segment(text)) count++;
  return count;
}

function fitPrefix(parts: readonly string[], suffix: string, maxPrefix: number, width: number, measure: (text: string) => number): string {
  let lo = 0, hi = maxPrefix;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (measure(`${parts.slice(0, mid).join('')}${suffix}`) <= width) lo = mid; else hi = mid - 1;
  }
  return `${parts.slice(0, lo).join('')}${suffix}`;
}

export function ellipsise(text: string, width: number, measure: (text: string) => number): string {
  if (measure(text) <= width) return text;
  if (measure(ELLIPSIS) > width) return '';
  const parts = graphemes(text);
  return fitPrefix(parts, ELLIPSIS, parts.length, width, measure);
}

function commonPrefixLength(groups: readonly (readonly string[])[]): number {
  const first = groups[0];
  let limit = first.length;
  for (let i = 1; i < groups.length; i++) limit = Math.min(limit, groups[i].length);
  let matched = 0;
  while (matched < limit && groups.every(parts => parts[matched] === first[matched])) matched++;
  return matched;
}

function commonSuffixLength(groups: readonly (readonly string[])[], prefix: number): number {
  const first = groups[0];
  let limit = first.length - prefix;
  for (let i = 1; i < groups.length; i++) limit = Math.min(limit, groups[i].length - prefix);
  let matched = 0;
  while (matched < limit && groups.every(parts => parts[parts.length - 1 - matched] === first[first.length - 1 - matched])) matched++;
  return matched;
}

function maxTailLength(parts: readonly string[], width: number, measure: (text: string) => number): number {
  let lo = 0, hi = parts.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (measure(`${ELLIPSIS}${parts.slice(parts.length - mid).join('')}`) <= width) lo = mid; else hi = mid - 1;
  }
  return lo;
}

function fitMiddle(parts: readonly string[], prefix: number, suffix: number, width: number, measure: (text: string) => number, mode: 'balanced' | 'tail' | 'slice'): string {
  if (measure(ELLIPSIS) > width) return '';
  if (mode === 'slice') {
    const tail = parts.slice(prefix);
    if (measure(`${ELLIPSIS}${tail.join('')}`) <= width) return `${ELLIPSIS}${tail.join('')}`;
    if (measure(`${ELLIPSIS}${ELLIPSIS}`) > width) return fitPrefix(parts, ELLIPSIS, parts.length, width, measure);
    let lo = 0, hi = tail.length;
    while (lo < hi) {
      const mid = Math.ceil((lo + hi) / 2);
      const end = prefix + mid === parts.length ? '' : ELLIPSIS;
      if (measure(`${ELLIPSIS}${tail.slice(0, mid).join('')}${end}`) <= width) lo = mid; else hi = mid - 1;
    }
    if (lo === 0) return fitPrefix(parts, ELLIPSIS, parts.length, width, measure);
    return `${ELLIPSIS}${tail.slice(0, lo).join('')}${prefix + lo === parts.length ? '' : ELLIPSIS}`;
  }
  const rest = parts.slice(prefix).join('');
  if (mode === 'balanced' && rest.length > 0 && measure(`${ELLIPSIS}${rest}`) <= width) {
    return fitPrefix(parts, `${ELLIPSIS}${rest}`, prefix, width, measure);
  }
  const maxTail = maxTailLength(parts, width, measure);
  if (maxTail <= 0) return ELLIPSIS;
  if (suffix >= maxTail) return fitMiddle(parts, prefix, suffix, width, measure, 'slice');
  const tailLen = mode === 'tail' ? maxTail : Math.min(maxTail, Math.max(suffix + 1, Math.ceil(maxTail * 0.6)));
  const tail = parts.slice(parts.length - tailLen).join('');
  if (mode === 'tail') return `${ELLIPSIS}${tail}`;
  return fitPrefix(parts, `${ELLIPSIS}${tail}`, parts.length - tailLen, width, measure);
}

function disambiguate(group: readonly string[], roomOf: (text: string) => number, measure: (text: string) => number, out: Map<string, string>, mode: 'balanced' | 'tail' | 'slice'): void {
  const entries = group.map(text => ({ text, parts: graphemes(text), room: roomOf(text) }));
  const prefix = commonPrefixLength(entries.map(entry => entry.parts));
  const suffix = commonSuffixLength(entries.map(entry => entry.parts), prefix);
  const buckets = new Map<string, string[]>();
  for (const entry of entries) {
    const shown = measure(entry.text) <= entry.room ? entry.text : fitMiddle(entry.parts, prefix, suffix, entry.room, measure, mode);
    out.set(entry.text, shown);
    if (shown !== '') {
      const bucket = buckets.get(shown);
      if (bucket) bucket.push(entry.text); else buckets.set(shown, [entry.text]);
    }
  }
  for (const collided of buckets.values()) {
    if (collided.length <= 1) continue;
    if (collided.length < group.length) disambiguate(collided, roomOf, measure, out, 'balanced');
    else if (mode === 'balanced') disambiguate(collided, roomOf, measure, out, 'tail');
    else if (mode === 'tail') disambiguate(collided, roomOf, measure, out, 'slice');
  }
}

export function fitLabels(texts: Iterable<string>, width: number | ((text: string) => number), measure: (text: string) => number): Map<string, string> {
  const roomOf = typeof width === 'number' ? () => width : width;
  const out = new Map<string, string>();
  const buckets = new Map<string, string[]>();
  for (const text of texts) {
    if (out.has(text)) continue;
    const shown = ellipsise(text, roomOf(text), measure);
    out.set(text, shown);
    if (shown !== '') {
      const bucket = buckets.get(shown);
      if (bucket) bucket.push(text); else buckets.set(shown, [text]);
    }
  }
  for (const collided of buckets.values()) {
    if (collided.length > 1) disambiguate(collided, roomOf, measure, out, 'balanced');
  }
  return out;
}
