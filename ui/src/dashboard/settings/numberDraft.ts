import { validOption } from '../options.ts';
import type { OptionField } from '../options';

type NumberField = Extract<OptionField, { kind: 'number' }>;

/** What a number box shows, and the number it last saved while being typed in. */
export interface NumberDraft { text: string; sent?: number | null }
/** A box's next draft, and the value to save when there is one. */
export interface NumberStep { draft: NumberDraft; save?: number | null }

/** The text a box shows for a saved value: empty for automatic. */
export const numberText = (value: number | null) => value == null ? '' : String(value);

const legal = (field: NumberField, text: string) => {
  const number = Number(text.trim());
  return text.trim() !== '' && validOption(field, number) ? number : null;
};

/** Typing or stepping: the text stays as typed, and saves at once when it is a legal new number. */
export function typeNumber(field: NumberField, draft: NumberDraft, text: string, value: number | null): NumberStep {
  const number = legal(field, text);
  return number !== null && number !== value ? { draft: { text, sent: number }, save: number } : { draft: { ...draft, text } };
}

/** A saved value arriving: the box's own save coming back leaves the typing as it is (`-0`
 * comes back as 0, and the next digit must still make a negative number); any other value
 * replaces it. */
export function receiveNumber(draft: NumberDraft, value: number | null): NumberDraft {
  return draft.sent !== undefined && draft.sent === value ? draft : { text: numberText(value) };
}

/** Leaving the box: empty means automatic where the field allows it, and text that is
 * not a legal number goes back to the saved one. */
export function leaveNumber(field: NumberField, draft: NumberDraft, value: number | null): NumberStep {
  if (draft.text.trim() === '' && field.default === null) return value === null ? { draft } : { draft: { text: '', sent: null }, save: null };
  return legal(field, draft.text) === null ? { draft: { text: numberText(value) } } : { draft };
}
