import { validOption } from '../options.ts';
import type { OptionField } from '../options';

type NumberField = Extract<OptionField, { kind: 'number' }>;

export interface NumberDraft { text: string; sent?: number | null }
export interface NumberStep { draft: NumberDraft; save?: number | null }

export const numberText = (value: number | null) => value == null ? '' : String(value);

const legal = (field: NumberField, text: string) => {
  const number = Number(text.trim());
  return text.trim() !== '' && validOption(field, number) ? number : null;
};

export function typeNumber(field: NumberField, draft: NumberDraft, text: string, value: number | null): NumberStep {
  const number = legal(field, text);
  return number !== null && number !== value ? { draft: { text, sent: number }, save: number } : { draft: { ...draft, text } };
}

export function receiveNumber(draft: NumberDraft, value: number | null): NumberDraft {
  return draft.sent !== undefined && draft.sent === value ? draft : { text: numberText(value) };
}

export function leaveNumber(field: NumberField, draft: NumberDraft, value: number | null): NumberStep {
  if (draft.text.trim() === '' && field.default === null) return value === null ? { draft } : { draft: { text: '', sent: null }, save: null };
  return legal(field, draft.text) === null ? { draft: { text: numberText(value) } } : { draft };
}
