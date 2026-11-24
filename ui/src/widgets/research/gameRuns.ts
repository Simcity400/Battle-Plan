import type { EntityKind, GameRun } from '../../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const tone = (value: unknown) => value === null || (typeof value === 'string' && [...value].length === 1);

export const ENTITY_KINDS: readonly EntityKind[] = ['equipment', 'module', 'subUnit', 'building', 'tactic', 'technology'];

export const entityKind = (value: unknown): value is EntityKind => ENTITY_KINDS.includes(value as EntityKind);

function run(value: unknown): boolean {
  if (!object(value)) return false;
  switch (value.kind) {
    case 'text': return typeof value.text === 'string' && tone(value.tone);
    case 'icon': return typeof value.sprite === 'string';
    case 'flag': return typeof value.tag === 'string';
    case 'link': return entityKind(value.entity) && typeof value.token === 'string' && value.token !== '' && typeof value.text === 'string' && tone(value.tone)
      && (value.databaseIndex === null || (Number.isInteger(value.databaseIndex) && (value.databaseIndex as number) > 0 && (value.databaseIndex as number) <= 2147483647));
    default: return false;
  }
}

export function validGameRuns(value: unknown): value is GameRun[] {
  return Array.isArray(value) && value.every(run);
}

const TONES: Record<string, string> = {
  G: 'positive', R: 'negative', O: 'warning',
  H: 'strong', Y: 'strong', T: 'strong', W: 'strong',
  B: 'accent', C: 'accent',
  g: 'muted', L: 'muted', l: 'muted',
};

export const toneClass = (code: string | null) => code ? TONES[code] ?? null : null;

export const ENTITY_LABELS: Record<EntityKind, string> = {
  equipment: 'Equipment', module: 'Module', subUnit: 'Sub-unit', building: 'Building', tactic: 'Tactic', technology: 'Technology',
};

function trim(runs: readonly GameRun[], end: RegExp): GameRun[] {
  const out = [...runs];
  for (let at = out.length - 1; at >= 0; at--) {
    const last = out[at];
    if (last.kind !== 'text') break;
    const text = last.text.replace(end, '');
    if (text) {
      out[at] = { ...last, text };
      break;
    }
    out.splice(at, 1);
  }
  while (out[0]?.kind === 'text') {
    const text = out[0].text.replace(/^\s+/u, '');
    if (text) {
      out[0] = { ...out[0], text };
      break;
    }
    out.shift();
  }
  return out;
}

export const trimLabel = (runs: readonly GameRun[]) => trim(runs, /[\s:：]+$/u);
export const trimValue = (runs: readonly GameRun[]) => trim(runs, /\s+$/u);
