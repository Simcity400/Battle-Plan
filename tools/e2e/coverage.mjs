// What the suite repeats and what it does once: every case that ever failed runs every time (with `--pin product`, every
// case the product ever failed); a placement whose kind, configuration, size and neighbours equal one already exercised
// is a real repeat; a control's values are each covered once across the menu's states (pairwise, not the product). Pure
// functions, so they are tested without the app.

/** A step's key in the failure history. A catalogue setup step is keyed by the card it follows. */
export const failureKey = result => `${result.area}|${result.card ?? result.item}|${result.label ?? ''}`;

/** The history's format: each failing step's key with whose failure it was. A history without it is refused by name. */
export const HISTORY_FORMAT = 2;

/** Failures that are the suite's own: it had no case, could not observe, or judged what the app rightly refuses. */
const SUITE_REASONS = [
  [/^not exercised: /, 'no case'],
  [/^could not act: RuntimeError: no colour dialog/, 'colour dialog folded'],
  [/^could not act: its row offers no reset$/, 'colour never set'],
  [/^could not send its input: |no map window under screen point/, 'no native input'],
  [/unobservable|unconfirmed|painted is unavailable$|^no trusted input reached the page$|^its widget is not in view$/, 'unobservable'],
  [/: This group has no members (at the end of this campaign|at this moment|in this campaign)\.$/, 'empty group chosen'],
  [/^the recording cannot draw it: /, 'refused by name'],
];
/** The product's failures, by class: the app was wrong, missing or over budget. */
const PRODUCT_REASONS = [
  [/ ms, over \d+ ms$/, 'over budget'],
  [/: still loading after \d+ s$/, 'still loading'],
  [/^showed a loading placeholder /, 'placeholder'],
  [/^still in flight after /, 'in flight'],
  [/Couldn’t load/, 'failed to load'],
  [/: shows nothing$/, 'blank'],
  [/^no visible change after the setting/, 'no visible change'],
  [/^never finished: |^not settled within /, 'never finished'],
  [/^the map /, 'map'],
];

/** Whose failure a failing step was, from its reasons: the suite's only when every reason is one of the suite's own,
 * else the product's (the app was wrong or over budget); `why` names the class. */
export function failureCause(result) {
  const reasons = result.reasons?.length ? result.reasons : [''];
  const suite = reasons.map(reason => SUITE_REASONS.find(([pattern]) => pattern.test(reason))?.[1] ?? null);
  const product = reasons.findIndex((_, at) => suite[at] === null);
  if (product < 0) return { cause: 'suite', why: suite[0] };
  return { cause: 'product', why: PRODUCT_REASONS.find(([pattern]) => pattern.test(reasons[product]))?.[1] ?? 'other' };
}

/** `history` read from a file, or null for none; one without the product/suite mark is refused by name. */
export function readHistory(history, file = 'the failure history') {
  if (history === null || history === undefined) return null;
  if (history.format !== HISTORY_FORMAT) throw new Error(`${file} predates the mark of whose failure each step was (format ${history.format ?? 1}, not ${HISTORY_FORMAT}): seed it again`);
  return history;
}

/** The history with this run's failing steps added, each marked with whose failure it was (`failureCause`); a step once
 * the product's stays so. `slowest` replaces its slowest catalogue cards when given. Setup steps of a catalogue run that
 * predate `card` take the card of the add before them. */
export function mergeHistory(history, results, { slowest = null, at = null } = {}) {
  const failed = { ...history?.failed };
  let card = null;
  for (const result of results) {
    if (result.area === 'catalogue' && result.item !== 'setup') card = result.item;
    if (result.pass) continue;
    const keyed = result.area === 'catalogue' && result.item === 'setup' && result.card === undefined && card ? { ...result, card } : result;
    const key = failureKey(keyed);
    if (failed[key]?.cause !== 'product') failed[key] = failureCause(result);
  }
  const sorted = Object.fromEntries(Object.keys(failed).sort().map(key => [key, failed[key]]));
  return { format: HISTORY_FORMAT, failed: sorted, slowest: slowest ?? history?.slowest ?? [], updated: at ?? history?.updated ?? null };
}

/** The `count` catalogue entries slowest to show content in a run's catalogue results, slowest first. */
export const slowestCards = (results, count = 5) => results.filter(result => result.area === 'catalogue' && result.label === 'add' && Number.isFinite(result.ms))
  .sort((a, b) => b.ms - a.ms).slice(0, count).map(result => result.item);

/** Which failures pin a case: every one (`all`), or only the product's (`product`). */
export const PINS = ['all', 'product'];

/** Each history's failing keys, sorted, per pin. */
const pinned = new WeakMap();
function keysOf(history, pin) {
  let held = pinned.get(history);
  if (!held) {
    const all = Object.keys(history.failed ?? {}).sort();
    pinned.set(history, held = { all, product: all.filter(key => history.failed[key].cause === 'product') });
  }
  return held[pin];
}

/** Whether any step of a case ever failed: a history key starting `area|item|prefix`, among the failures `pin` counts. */
export function everFailed(history, area, item, prefix = '', pin = 'all') {
  if (!history) return false;
  const keys = keysOf(history, pin);
  const wanted = `${area}|${item}|${prefix}`;
  let low = 0, high = keys.length;
  while (low < high) { const mid = (low + high) >> 1; if (keys[mid] < wanted) low = mid + 1; else high = mid; }
  return low < keys.length && keys[low].startsWith(wanted);
}

/** A placement's configuration as a user meets it: its kind, size, every setting its menu shows, and the page around it
 * (its neighbours' kinds and sizes). Two placements with one key are the same case. */
export function placementKey({ kind, width, height, controls, neighbours }) {
  const settings = controls.map(c => `${c.kind}|${c.label}|${c.context ?? ''}|${JSON.stringify(c.state ?? null)}|${c.disabled ? 'off' : 'on'}`);
  return JSON.stringify([kind, Math.round(width), Math.round(height), settings, [...neighbours].sort()]);
}

/** A control as itself whatever values it lists: the same setting revealed under another value of its parent. */
export const controlBase = c => `${c.kind}|${c.label}|${c.context ?? ''}|${c.nth ?? 0}`;

/** Each value of each setting at least once across every state of its menu. `take(base, value, failedBefore)` says whether
 * to exercise `value` of the control `base` now: once each, and again whenever its case ever failed. `limit` caps how many
 * values a widget exercises at all (the quick tier's one setting per widget kind); `skipped` counts what was not repeated. */
export class Coverage {
  constructor({ limit = Infinity } = {}) { this.seen = new Map(); this.limit = limit; this.taken = 0; this.skipped = 0; }

  /** Whether the widget has exercised all it may. */
  get full() { return this.taken >= this.limit; }

  take(base, value, failedBefore = false) {
    if (this.taken >= this.limit) { this.skipped++; return false; }
    const values = this.seen.get(base) ?? new Set();
    this.seen.set(base, values);
    if (values.has(value) && !failedBefore) { this.skipped++; return false; }
    values.add(value);
    this.taken++;
    return true;
  }
}
