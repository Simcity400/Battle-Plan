/** Widget options: the view, measure, order and on/off choices a widget declares.
 * A definition lists its fields; a layout stores only the values. Stored values
 * are checked twice: generically on load (so a widget that is not installed keeps
 * its choices) and against the field when read, so a bad value falls back to the
 * field's default instead of reaching the widget. */

import type { CountrySelection, WidgetTime } from './model';

export type OptionValue = boolean | number | string | null;
export type WidgetOptions = Readonly<Record<string, OptionValue>>;
/** Where a field shows in the settings panel. */
export type OptionSection = 'countries' | 'time' | 'options' | 'axes' | 'interaction';

interface FieldBase {
  key: string;
  section: OptionSection;
  label: string;
  hint?: string;
  /** Shown only when this returns true for the resolved options, country choice and
   * time mode (a setting that does nothing in a mode is hidden there, not explained). */
  visibleWhen?: (options: Readonly<Record<string, OptionValue>>, countries?: CountrySelection, time?: WidgetTime['mode']) => boolean;
}
export type OptionField = FieldBase & (
  | { kind: 'toggle'; default: boolean }
  | { kind: 'choice'; choices: readonly { value: string; label: string }[]; default: string }
  /** A null default means "automatic" until the user sets a number. */
  | { kind: 'number'; min: number; max: number; step: number; unit?: string; default: number | null }
  /** A choice among things only the recording's content knows (such as its idea
   * categories): the widget lists them at run time (`useOptionChoices`). The raw
   * scalar string is kept, so a choice survives a campaign that lacks it. The empty
   * default lets the widget pick, shown as `automatic`. `titleChoice: false`
   * keeps the widget title when this choice changes. */
  | { kind: 'content-choice'; automatic: string; default: ''; titleChoice?: false }
  /** Several things only the recording's content knows (such as its resources),
   * shown as chips, or a searchable virtual list with `catalogue`. Stored as their tokens
   * separated by spaces; the empty default lets the widget pick, shown as `automatic`. */
  | { kind: 'content-set'; automatic: string; default: ''; catalogue?: boolean }
);

/** The characters a stored content-set token may hold. A script token holds no
 * whitespace but may hold other characters, non-ASCII included (a mod's own
 * names), so only whitespace, control characters and commas are refused: spaces
 * separate a content set's tokens, and a comma-joined list is never one token. */
const TOKEN_TEXT = /^[^\s\p{Cc},]*$/u;
const contentToken = (value: string) => TOKEN_TEXT.test(value);
/** One token of a content set. */
const setTokenText = (value: string) => value.length > 0 && contentToken(value);
const ESCAPED_POINT = /:([0-9a-f]{1,6}):/g;
const unicodeScalar = (point: number) => point <= 0x10ffff && (point < 0xd800 || point > 0xdfff);
const validEscapes = (token: string) => [...token.matchAll(ESCAPED_POINT)].every(([, hex]) => unicodeScalar(parseInt(hex, 16)));

/** Whether a content token can be stored in a content set: some content names
 * cannot (with characters a stored token may not hold), and a control
 * offering them says so rather than failing when picked. */
export const storableToken = (token: string) => setTokenText(token) && validEscapes(token);
/** Why a chip for a token that cannot be stored is disabled. */
export const UNSTORABLE = 'This choice cannot be saved.';

/** A content key as a storable token: each character a setting cannot store (and
 * `:` itself) is written as `:<hex code point>:`, so a key with any characters, such
 * as a type content does not resolve (`?token`), can still be chosen; plain keys are
 * unchanged. `contentKey` reads it back. A lone surrogate can
 * still make it unstorable (`storableToken`). */
export const setToken = (key: string) => key.replace(/[^\w.-]/gu, character => `:${character.codePointAt(0)!.toString(16)}:`);
export const contentKey = (token: string) => token.replace(ESCAPED_POINT, (escape, hex: string) => {
  const point = parseInt(hex, 16);
  return unicodeScalar(point) ? String.fromCodePoint(point) : escape;
});

/** The token limit an in-widget chip control requests. */
export const setLimit = (max: number) => Math.max(0, Math.floor(max));

/** The tokens of a stored content-set value, in the order chosen. */
export function contentSet(value: OptionValue): string[] {
  return typeof value === 'string' && value ? value.split(' ') : [];
}

/** A content set with `token` added or removed; null when adding would pass the
 * caller's limit or the token cannot be stored.
 * Removing the last token returns to automatic. */
export function toggleContentSet(value: OptionValue, token: string, max: number): string | null {
  const tokens = contentSet(value);
  const next = tokens.includes(token) ? tokens.filter(item => item !== token) : [...tokens, token];
  return next.length > setLimit(max) || !next.every(storableToken) ? null : next.join(' ');
}

/** Whole-campaign playback stops at the playhead unless this is on (user decision). */
export const AFTER_PLAYHEAD_OPTION: OptionField = { key: 'afterPlayhead', section: 'time', label: 'Show after the playhead',
  hint: 'Draws the chart, events and hover past the moment shown', kind: 'toggle', default: false, visibleWhen: options => options.playback === 'cursor' };

export const MAX_OPTION_KEYS = 48;
const KEY = /^[a-zA-Z][a-zA-Z0-9_-]{0,63}$/;
const RESERVED = new Set(['__proto__', 'constructor', 'prototype']);

const validKey = (key: string) => KEY.test(key) && !RESERVED.has(key);

/** Generic bounds for stored options, independent of any widget's fields. */
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

/** Whether a value is legal for a field. */
export function validOption(field: OptionField, value: unknown): value is OptionValue {
  switch (field.kind) {
    case 'toggle': return typeof value === 'boolean';
    case 'choice': return typeof value === 'string' && field.choices.some(choice => choice.value === value);
    case 'content-choice': return typeof value === 'string';
    case 'content-set': {
      if (typeof value !== 'string') return false;
      const tokens = contentSet(value);
      return tokens.every(storableToken) && new Set(tokens).size === tokens.length;
    }
    case 'number': return (value === null && field.default === null) || (typeof value === 'number' && Number.isFinite(value) && value >= field.min && value <= field.max
      && Math.abs((value - field.min) / field.step - Math.round((value - field.min) / field.step)) < 1e-9);
  }
}

/** Every field's value: the stored one when legal, otherwise the field's default. */
export function resolveOptions(fields: readonly OptionField[] = [], stored?: WidgetOptions): Record<string, OptionValue> {
  const resolved: Record<string, OptionValue> = {};
  for (const field of fields) {
    const value = stored && Object.hasOwn(stored, field.key) ? stored[field.key] : undefined;
    resolved[field.key] = validOption(field, value) ? value : field.default;
  }
  return resolved;
}

/** Stores one value. A value equal to the default is removed, so a widget picks up
 * a changed default; keys this widget does not declare are kept untouched. */
export function setOption(fields: readonly OptionField[] = [], stored: WidgetOptions | undefined, key: string, value: OptionValue): WidgetOptions | null {
  const field = fields.find(item => item.key === key);
  if (!field || !validOption(field, value)) return null;
  const next: Record<string, OptionValue> = { ...stored };
  if (value === field.default) delete next[key];
  else next[key] = value;
  return next;
}

/** Fields shown in one settings section, in declaration order. */
export function sectionFields(fields: readonly OptionField[] = [], section: OptionSection, resolved: Readonly<Record<string, OptionValue>>,
  countries?: CountrySelection, time?: WidgetTime['mode']): OptionField[] {
  return fields.filter(field => field.section === section && (field.visibleWhen?.(resolved, countries, time) ?? true));
}
