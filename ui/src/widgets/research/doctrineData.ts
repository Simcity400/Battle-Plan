import type { DoctrineReply } from '../../generated/dashboard';

type ObjectValue = Record<string, unknown>;
const object = (value: unknown): value is ObjectValue => !!value && typeof value === 'object' && !Array.isArray(value);
const integer = (value: unknown) => typeof value === 'number' && Number.isSafeInteger(value);
const raw = (value: unknown): value is string => typeof value === 'string' && /^-?\d+$/.test(value) && BigInt(value) >= -(1n << 63n) && BigInt(value) < 1n << 63n;
const texts = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');
const facts = (value: unknown) => Array.isArray(value) && value.every(item => object(item) && typeof item.name === 'string' && typeof item.value === 'string');
const rewards = (value: unknown) => Array.isArray(value) && value.every(item => object(item) && typeof item.name === 'string' && raw(item.mastery) && (item.unlocked === null || typeof item.unlocked === 'boolean') && typeof item.description === 'string');
const nullableText = (value: unknown) => value === null || typeof value === 'string';

export function validateDoctrines(value: unknown): DoctrineReply {
  if (!object(value) || typeof value.branchId !== 'string' || typeof value.tag !== 'string' || !integer(value.hour)
    || !texts(value.warnings) || !Array.isArray(value.folders) || !Array.isArray(value.inventory)
    || !value.folders.every(folder => object(folder) && integer(folder.ordinal) && typeof folder.name === 'string' && nullableText(folder.grandDoctrine)
      && typeof folder.benefits === 'string' && typeof folder.authoredCost === 'string' && rewards(folder.milestones) && texts(folder.completed) && Array.isArray(folder.tracks) && folder.tracks.every(track => object(track) && integer(track.ordinal)
        && typeof track.name === 'string' && nullableText(track.subDoctrine) && integer(track.rewards) && raw(track.mastery) && raw(track.masteryBank)
        && typeof track.benefits === 'string' && typeof track.authoredCost === 'string' && rewards(track.rewardBenefits) && raw(track.dailyMastery) && raw(track.unitDailyMastery) && Array.isArray(track.rewardRequirements) && track.rewardRequirements.every(raw) && facts(track.commanders)))
    || !value.inventory.every(item => object(item) && typeof item.kind === 'string' && integer(item.ordinal) && facts(item.fields))) {
    throw new Error('Invalid doctrine state reply.');
  }
  return value as unknown as DoctrineReply;
}

export function masteryText(raw: string): string {
  const value = BigInt(raw), magnitude = value < 0n ? -value : value;
  const fraction = (magnitude % 100000n).toString().padStart(5, '0').replace(/0+$/, '');
  return `${value < 0n ? '−' : ''}${magnitude / 100000n}${fraction ? `.${fraction}` : ''}`;
}
