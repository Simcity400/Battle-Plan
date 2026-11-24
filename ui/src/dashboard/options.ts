
import type { WidgetPreset } from './definitions';
import type { CountrySelection, WidgetTime } from './model';

export type OptionValue = boolean | number | string | null;
export type WidgetOptions = Readonly<Record<string, OptionValue>>;
export type OptionSection = 'countries' | 'time' | 'options' | 'axes' | 'interaction';

interface FieldBase {
  key: string;
  storeAs?: string;
  section: OptionSection;
  label: string;
  hint?: string;
  visibleWhen?: (options: Readonly<Record<string, OptionValue>>, countries?: CountrySelection, time?: WidgetTime['mode']) => boolean;
}
export type OptionField = FieldBase & (
  | { kind: 'toggle'; default: boolean }
  | { kind: 'choice'; choices: readonly { value: string; label: string }[]; default: string; titleChoice?: true }
  | { kind: 'number'; min: number; max: number; step: number; unit?: string; default: number | null }
  | { kind: 'content-choice'; automatic: string; default: ''; titleChoice?: false; catalogue?: boolean }
  | { kind: 'content-set'; automatic: string; default: ''; catalogue?: boolean }
  | { kind: 'regions'; automatic: string; default: ''; sea?: boolean }
  | { kind: 'table'; default: ''; columns: readonly import('../families/table/settings.ts').TableColumnDefaults[]; base: import('../families/table/settings.ts').TableBase }
);

const TOKEN_TEXT = /^[^\s\p{Cc},]*$/u;
const contentToken = (value: string) => TOKEN_TEXT.test(value);
const setTokenText = (value: string) => value.length > 0 && contentToken(value);
const ESCAPED_POINT = /:([0-9a-f]{1,6}):/g;
const unicodeScalar = (point: number) => point <= 0x10ffff && (point < 0xd800 || point > 0xdfff);
const validEscapes = (token: string) => [...token.matchAll(ESCAPED_POINT)].every(([, hex]) => unicodeScalar(parseInt(hex, 16)));

export const storableToken = (token: string) => setTokenText(token) && validEscapes(token);
export const UNSTORABLE = 'This choice can’t be saved.';

export const setToken = (key: string) => key.replace(/[^\w.-]/gu, character => `:${character.codePointAt(0)!.toString(16)}:`);
export const contentKey = (token: string) => token.replace(ESCAPED_POINT, (escape, hex: string) => {
  const point = parseInt(hex, 16);
  return unicodeScalar(point) ? String.fromCodePoint(point) : escape;
});

export const setLimit = (max: number) => Math.max(0, Math.floor(max));

export function contentSet(value: OptionValue): string[] {
  return typeof value === 'string' && value ? value.split(' ') : [];
}

export function toggleContentSet(value: OptionValue, token: string, max: number): string | null {
  const tokens = contentSet(value);
  const next = tokens.includes(token) ? tokens.filter(item => item !== token) : [...tokens, token];
  return next.length > setLimit(max) || !next.every(storableToken) ? null : next.join(' ');
}

export const AFTER_PLAYHEAD_OPTION: OptionField = { key: 'afterPlayhead', section: 'time', label: 'Show after the playhead',
  hint: 'Draws the chart, events and hover past the moment shown', kind: 'toggle', default: false, visibleWhen: options => options.playback === 'cursor' };

export const MAX_OPTION_KEYS = 48;
const KEY = /^[a-zA-Z][a-zA-Z0-9_-]{0,63}$/;
const VIEW = /^[a-zA-Z][a-zA-Z0-9_-]{0,31}$/;
const RESERVED = new Set(['__proto__', 'constructor', 'prototype']);

export function scopedOptionKey(key: string, view: string): string {
  return `${key}@${view}`;
}

export function parseOptionKey(stored: string): { readonly key: string; readonly view: string | null } | null {
  const at = stored.indexOf('@');
  const key = at < 0 ? stored : stored.slice(0, at);
  const view = at < 0 ? null : stored.slice(at + 1);
  if (!KEY.test(key) || RESERVED.has(key) || (view !== null && !VIEW.test(view))) return null;
  return { key, view };
}

const validKey = (key: string) => parseOptionKey(key) !== null;

export function parseStoredOptions(input: unknown): WidgetOptions | null {
  if (typeof input !== 'object' || input == null || Array.isArray(input)) return null;
  const entries = Object.entries(input);
  if (entries.length > MAX_OPTION_KEYS) return null;
  const options: Record<string, OptionValue> = {};
  for (const [key, value] of entries) {
    if (!validKey(key)) return null;
    if (!(value === null || typeof value === 'boolean' || (typeof value === 'number' && Number.isFinite(value))
      || typeof value === 'string')) return null;
    options[key] = value;
  }
  return options;
}

export function validOption(field: OptionField, value: unknown): value is OptionValue {
  switch (field.kind) {
    case 'toggle': return typeof value === 'boolean';
    case 'choice': return typeof value === 'string' && field.choices.some(choice => choice.value === value);
    case 'content-choice': return typeof value === 'string';
    case 'content-set':
    case 'regions': {
      if (typeof value !== 'string') return false;
      const tokens = contentSet(value);
      return tokens.every(storableToken) && new Set(tokens).size === tokens.length;
    }
    case 'table': return typeof value === 'string';
    case 'number': return (value === null && field.default === null) || (typeof value === 'number' && Number.isFinite(value) && value >= field.min && value <= field.max
      && Math.abs((value - field.min) / field.step - Math.round((value - field.min) / field.step)) < 1e-9);
  }
}

export function resolveOptions(fields: readonly OptionField[] = [], stored?: WidgetOptions): Record<string, OptionValue> {
  const resolved: Record<string, OptionValue> = {};
  for (const field of fields) {
    const at = field.storeAs ?? field.key;
    const value = stored && Object.hasOwn(stored, at) ? stored[at] : undefined;
    resolved[field.key] = validOption(field, value) ? value : field.default;
  }
  return resolved;
}

export function setOption(fields: readonly OptionField[] = [], stored: WidgetOptions | undefined, key: string, value: OptionValue): WidgetOptions | null {
  const field = fields.find(item => item.key === key);
  if (!field || !validOption(field, value)) return null;
  const at = field.storeAs ?? key;
  const next: Record<string, OptionValue> = { ...stored };
  if (value === field.default) delete next[at];
  else next[at] = value;
  return next;
}

export function followedTitle(field: OptionField | undefined, title: string, baseTitle: string | undefined, from: OptionValue | undefined, to: OptionValue,
  label: (value: OptionValue | undefined) => string | undefined = value => (field?.kind === 'choice' ? field.choices.find(choice => choice.value === value)?.label : undefined)): string {
  const follows = field?.kind === 'content-choice' ? field.titleChoice !== false : field?.kind === 'choice' && field.titleChoice === true;
  if (!follows || ![baseTitle, label(from)].includes(title)) return title;
  return label(to) ?? baseTitle ?? title;
}

export function choiceVariants(field: OptionField | undefined, name: (choice: { value: string; label: string }) => string,
  values?: readonly string[]): WidgetPreset[] {
  if (field?.kind !== 'choice') return [];
  return field.choices.filter(choice => values ? values.includes(choice.value) : choice.value !== field.default)
    .map(choice => ({ key: `${field.key}=${choice.value}`, initial: { title: name(choice), options: { [field.key]: choice.value } } }));
}

export function sectionFields(fields: readonly OptionField[] = [], section: OptionSection, resolved: Readonly<Record<string, OptionValue>>,
  countries?: CountrySelection, time?: WidgetTime['mode']): OptionField[] {
  return fields.filter(field => field.section === section && (field.visibleWhen?.(resolved, countries, time) ?? true));
}
