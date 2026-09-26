/** Widget options: the view, measure, order and on/off choices a widget declares.
 * A definition lists its fields; a layout stores only the values. Stored values
 * are checked twice: generically on load (so a widget that is not installed keeps
 * its choices) and against the field when read, so a bad value falls back to the
 * field's default instead of reaching the widget. */

export type OptionValue = boolean | number | string | null;
export type WidgetOptions = Readonly<Record<string, OptionValue>>;
/** Where a field shows in the settings panel. */
export type OptionSection = 'time' | 'options' | 'axes' | 'interaction';

interface FieldBase {
  key: string;
  section: OptionSection;
  label: string;
  hint?: string;
  /** Shown only when this returns true for the current resolved options. */
  visibleWhen?: (options: Readonly<Record<string, OptionValue>>) => boolean;
}
export type OptionField = FieldBase & (
  | { kind: 'toggle'; default: boolean }
  | { kind: 'choice'; choices: readonly { value: string; label: string }[]; default: string }
  /** A null default means "automatic" until the user sets a number. */
  | { kind: 'number'; min: number; max: number; step: number; unit?: string; default: number | null }
);

export const MAX_OPTION_KEYS = 48;
const MAX_TEXT = 160;
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
      || (typeof value === 'string' && value.length <= MAX_TEXT))) return null;
    options[key] = value;
  }
  return options;
}

/** Whether a value is legal for a field. */
export function validOption(field: OptionField, value: unknown): value is OptionValue {
  switch (field.kind) {
    case 'toggle': return typeof value === 'boolean';
    case 'choice': return typeof value === 'string' && field.choices.some(choice => choice.value === value);
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
export function sectionFields(fields: readonly OptionField[] = [], section: OptionSection, resolved: Readonly<Record<string, OptionValue>>): OptionField[] {
  return fields.filter(field => field.section === section && (field.visibleWhen?.(resolved) ?? true));
}
