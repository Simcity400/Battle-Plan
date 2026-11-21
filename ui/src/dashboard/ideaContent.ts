import { int, object } from './validation.ts';
import type { EffectGroup, EffectLine, IdeaInfo, LocalisedText, ModifierFormat, TraitEffects } from '../generated/dashboard';

const text = (value: unknown): value is string | null => value === null || typeof value === 'string';
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');
const effectLine = (value: unknown): value is EffectLine => object(value) && typeof value.key === 'string'
  && typeof value.fixed5 === 'string' && /^-?\d+$/.test(value.fixed5) && typeof value.hidden === 'boolean';
const lines = (value: unknown): value is EffectLine[] => Array.isArray(value) && value.every(effectLine);
const effectGroup = (value: unknown): value is EffectGroup => object(value) && text(value.target)
  && typeof value.instant === 'boolean' && lines(value.lines) && strings(value.tooltips);
const groups = (value: unknown): value is EffectGroup[] => Array.isArray(value) && value.every(effectGroup);
const traitEffects = (value: unknown): value is TraitEffects => object(value) && typeof value.token === 'string'
  && typeof value.known === 'boolean' && text(value.unknown) && text(value.name) && lines(value.lines)
  && groups(value.targeted) && groups(value.equipment) && strings(value.tooltips);

export function ideaInfo(value: unknown): value is IdeaInfo {
  return object(value) && typeof value.token === 'string' && text(value.category) && text(value.slot)
    && typeof value.law === 'boolean' && typeof value.designer === 'boolean' && typeof value.effectsKnown === 'boolean'
    && text(value.unknown) && lines(value.effects) && groups(value.targeted) && groups(value.equipment)
    && lines(value.research) && Array.isArray(value.traits) && value.traits.every(traitEffects) && strings(value.tooltips);
}

export const modifierFormat = (value: unknown): value is ModifierFormat => object(value) && typeof value.key === 'string'
  && int(value.flags) && int(value.precision) && int(value.feature) && (value.index === null || int(value.index))
  && text(value.label) && text(value.postfix) && text(value.postfixKey);
export const localisedText = (value: unknown): value is LocalisedText => object(value) && typeof value.key === 'string' && text(value.text);
