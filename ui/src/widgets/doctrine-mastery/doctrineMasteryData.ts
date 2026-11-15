import { formatAmount, formatRate } from '../../charts/valueAxis.ts';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
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

/** Display names where the content gives none. */
export const folderName = (folder: MasteryFolder) => folder.name ?? 'Doctrine folder name unavailable';
export const trackName = (item: MasteryTrack) => item.name ?? `Track ${item.ordinal + 1}`;
export const rewardName = (item: MasteryReward) => item.name ?? 'Reward name unavailable';
export const choiceName = (item: MasteryChoice | null, none: string) => item === null ? none : item.name ?? 'Doctrine name unavailable';

export const trackKey = (folder: MasteryFolder, item: MasteryTrack) => `${folder.ordinal}|${item.ordinal}`;

/** A track's numbers as raw fixed5, read from its live cells; null with why where a cell has no value at the hour. */
export interface TrackNumbers { mastery: bigint | null; bank: bigint | null; daily: bigint | null; effective: bigint | null; missing: string }
export function trackNumbers(values: LiveValues<number>, key: string): TrackNumbers {
  const read = (column: string) => { const value = values.get(liveKey(key, column)); return value === undefined ? null : BigInt(Math.round(value)); };
  const mastery = read('mastery');
  return { mastery, bank: read('bank'), daily: read('daily'), effective: read('effective'), missing: mastery === null ? values.missing(liveKey(key, 'mastery')) || 'Not recorded' : '' };
}

/** The track mastery each reward unlocks at: a reward's threshold is its own step, so the game unlocks it at the sum of
 * its and every earlier threshold. Null from the first unknown one. */
export function unlockMastery(item: MasteryTrack): (bigint | null)[] {
  let sum: bigint | null = 0n;
  return item.rewards.map(next => sum = sum === null || next.threshold === null ? null : sum + BigInt(next.threshold));
}

/** The meter's scale: the mastery that unlocks the track's last reward, null when any threshold is unknown. */
export function trackScale(item: MasteryTrack): bigint | null {
  const last = unlockMastery(item).at(-1);
  return last != null && last > 0n ? last : null;
}

/** Mastery as a share of the scale, for the meter; null when either is unknown. */
export function trackRatio(item: MasteryTrack, mastery: bigint | null): number | null {
  const scale = trackScale(item);
  if (scale === null || mastery === null) return null;
  return Number(mastery * 10000n / scale) / 10000;
}

/** Whether the reward at `ordinal` is unlocked, by the track's recorded count, never by comparing mastery. */
export const rewardUnlocked = (item: MasteryTrack, ordinal: number) => ordinal < item.unlocked;

/** The rewards the recorded count certifies as unlocked, in authored order. */
export const unlockedRewards = (item: MasteryTrack) => item.rewards.filter((_, ordinal) => rewardUnlocked(item, ordinal));

/** Whether the mastery reaches the reward at `ordinal`; null when it or the requirement is unknown. */
export function requirementMet(item: MasteryTrack, ordinal: number, mastery: bigint | null): boolean | null {
  const needed = unlockMastery(item)[ordinal];
  return needed == null || mastery === null ? null : mastery >= needed;
}

/** A mark at each authored reward threshold, reached when the recorded count unlocks it. */
export function trackMarks(item: MasteryTrack, labels: boolean): MeterMark[] {
  const scale = trackScale(item);
  if (scale === null) return [];
  const unlocks = unlockMastery(item);
  return item.rewards.map((next, index) => ({ id: `${next.id}:${index}`, at: Number(unlocks[index]! * 10000n / scale) / 10000,
    reached: rewardUnlocked(item, index), ...labels ? { label: rewardName(next) } : {} }));
}

/** Mastery from its fixed-point value: at most two decimals, as the game's track shows it. */
export const masteryText = (raw: string | bigint) => formatAmount(raw, 5, 2);
export const rateText = (raw: string | bigint) => formatRate(String(raw), 5);
/** "+x a day" from the effective gain; empty where it is not recorded. */
export const dailyText = (raw: bigint | null) => raw === null ? '' : `${raw < 0n ? '' : '+'}${rateText(raw)} a day`;

/** The texts a track shows, painted each frame: mastery (or why it is missing), daily gain and bank (none when empty). */
export function trackTexts(numbers: TrackNumbers): { mastery: string; daily: string; bank: string } {
  return { mastery: numbers.mastery === null ? numbers.missing : masteryText(numbers.mastery), daily: dailyText(numbers.effective),
    bank: numbers.bank === null || numbers.bank === 0n ? '' : masteryText(numbers.bank) };
}

/** Tracks with a subdoctrine chosen, for the header. */
export const activeTracks = (folders: readonly MasteryFolder[]) => folders.reduce((sum, folder) => sum + folder.tracks.filter(item => item.sub !== null).length, 0);

export const doctrineSprite = (key: string | null | undefined): SpriteFrame | null => key ? { key, frame: 0 } : null;

/** Every sprite the reply names, for the shared sprite route. */
export const doctrineIcons = (reply: DoctrineMasteryReply | undefined): (string | null)[] => (reply?.folders ?? []).flatMap(folder =>
  [folder.icon, folder.grand?.icon ?? null, ...folder.tracks.map(item => item.sub?.icon ?? null)]);

/** What a render shows: folders, tracks, names, rewards and unlocks. Every number is a live cell, painted. */
export function masteryStructure(reply: DoctrineMasteryReply): string {
  return JSON.stringify(reply.folders.map(folder => [folder.id, folder.name, folder.icon, folder.grand, folder.completed,
    folder.tracks.map(item => [item.ordinal, item.name, item.sub, item.rewards, item.unlocked])]));
}

/** A reward's description from a `doctrine-rewards` reply for its track: by its place, when the reply is of the same
 * subdoctrine; null where the game words nothing for it. */
export function rewardDescription(reply: DoctrineRewardsReply | undefined, item: MasteryTrack, index: number): string | null {
  if (!reply || reply.sub !== (item.sub?.id ?? null)) return null;
  return reply.rewards[index]?.description ?? null;
}
