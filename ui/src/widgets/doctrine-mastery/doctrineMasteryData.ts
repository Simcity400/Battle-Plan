import { formatAmount, formatRate } from '../../charts/valueAxis.ts';
import type { MeterMark } from '../../charts/meter/ProgressMeter';
import type { SpriteFrame } from '../../dashboard/spriteArtwork';
import type { DescribedReward, DoctrineMasteryReply, DoctrineRewardsReply, MasteryChoice, MasteryFolder, MasteryReward, MasteryTrack } from '../../generated/dashboard';

const text = (value: unknown): value is string => typeof value === 'string';
const optional = (value: unknown) => value === null || text(value);
const decimal = (value: unknown) => text(value) && /^-?\d+$/u.test(value);
const choice = (value: unknown): value is MasteryChoice => !!value && typeof value === 'object'
  && text((value as MasteryChoice).id) && optional((value as MasteryChoice).name) && optional((value as MasteryChoice).icon);
const reward = (value: unknown): value is MasteryReward => !!value && typeof value === 'object' && text((value as MasteryReward).id)
  && optional((value as MasteryReward).name) && ((value as MasteryReward).threshold === null || decimal((value as MasteryReward).threshold));
const track = (value: unknown): value is MasteryTrack => {
  const item = value as MasteryTrack;
  return !!item && typeof item === 'object' && Number.isSafeInteger(item.ordinal) && optional(item.name) && (item.sub === null || choice(item.sub))
    && Array.isArray(item.rewards) && item.rewards.every(reward) && Number.isSafeInteger(item.unlocked);
};

export function validateDoctrineMastery(value: unknown): DoctrineMasteryReply {
  const reply = value as DoctrineMasteryReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.folders) || !Array.isArray(reply.warnings) || !reply.warnings.every(text)
    || !reply.folders.every(folder => !!folder && Number.isSafeInteger(folder.ordinal) && text(folder.id) && optional(folder.name) && optional(folder.icon)
      && (folder.grand === null || choice(folder.grand)) && Array.isArray(folder.completed) && folder.completed.every(choice)
      && Array.isArray(folder.tracks) && folder.tracks.every(track)))
    throw new Error('The doctrine mastery reply is malformed.');
  return reply;
}

export function validateDoctrineRewards(value: unknown): DoctrineRewardsReply {
  const reply = value as DoctrineRewardsReply;
  if (!reply || typeof reply !== 'object' || !optional(reply.sub) || !Array.isArray(reply.warnings) || !reply.warnings.every(text) || !Array.isArray(reply.rewards)
    || !reply.rewards.every(item => reward(item) && optional((item as DescribedReward).description)))
    throw new Error('The doctrine rewards reply is malformed.');
  return reply;
}

export const folderName = (folder: MasteryFolder) => folder.name ?? 'Doctrine folder name unavailable';
export const trackName = (item: MasteryTrack) => item.name ?? `Track ${item.ordinal + 1}`;
export const rewardName = (item: MasteryReward) => item.name ?? 'Reward name unavailable';
export const choiceName = (item: MasteryChoice | null, none: string) => item === null ? none : item.name ?? 'Doctrine name unavailable';

export const trackKey = (folder: MasteryFolder, item: MasteryTrack) => `${folder.ordinal}|${item.ordinal}`;

export interface TrackNumbers { mastery: bigint | null; bank: bigint | null; daily: bigint | null; effective: bigint | null; missing: string }
export function unlockMastery(item: MasteryTrack): (bigint | null)[] {
  let sum: bigint | null = 0n;
  return item.rewards.map(next => sum = sum === null || next.threshold === null ? null : sum + BigInt(next.threshold));
}

export function trackScale(item: MasteryTrack): bigint | null {
  const last = unlockMastery(item).at(-1);
  return last != null && last > 0n ? last : null;
}

export function trackRatio(item: MasteryTrack, mastery: bigint | null): number | null {
  const scale = trackScale(item);
  if (scale === null || mastery === null) return null;
  return Number(mastery * 10000n / scale) / 10000;
}

export const rewardUnlocked = (item: MasteryTrack, ordinal: number) => ordinal < item.unlocked;

export const unlockedRewards = (item: MasteryTrack) => item.rewards.filter((_, ordinal) => rewardUnlocked(item, ordinal));

export function trackMarks(item: MasteryTrack, labels: boolean): MeterMark[] {
  const scale = trackScale(item);
  if (scale === null) return [];
  const unlocks = unlockMastery(item);
  return item.rewards.map((next, index) => ({ id: `${next.id}:${index}`, at: Number(unlocks[index]! * 10000n / scale) / 10000,
    reached: rewardUnlocked(item, index), ...labels ? { label: rewardName(next) } : {} }));
}

export const masteryText = (raw: string | bigint) => formatAmount(raw, 5, 2);
export const rateText = (raw: string | bigint) => formatRate(String(raw), 5);
export const dailyText = (raw: bigint | null) => raw === null ? '' : `${raw < 0n ? '' : '+'}${rateText(raw)} a day`;

export const activeTracks = (folders: readonly MasteryFolder[]) => folders.reduce((sum, folder) => sum + folder.tracks.filter(item => item.sub !== null).length, 0);

export const doctrineSprite = (key: string | null | undefined): SpriteFrame | null => key ? { key, frame: 0 } : null;
