import { countryPart, keyedKnown, keyedRead, runAtHour, validateKeyedWindow } from '../../dashboard/keyedHistory.ts';
import type { KeyedWindow } from '../../dashboard/keyedHistory';
import { masteryText } from '../research/doctrineData.ts';
import type { MeterMark } from '../../charts/meter/ProgressMeter';
import type { SpriteFrame } from '../../dashboard/spriteArtwork';

/** Content-derived fields are null where the content is unavailable or refuses them; the
 * recorded structure (folders, tracks, witnessed choices) is always present. */
export interface DoctrineReward { id: string; name: string | null; description: string | null; threshold: string | null }
/** A grand doctrine or subdoctrine: `tokenId` is the recorded interned id (null for an authored
 * alternative the recording never witnessed); `id` is its spelling, or that id without one. */
export interface DoctrineChoice { id: string; tokenId: string | null; name: string | null; icon: string | null }
export interface Subdoctrine extends DoctrineChoice { rewards: readonly DoctrineReward[] | null }
export interface DoctrineTrack { id: string; name: string | null; subdoctrines: readonly Subdoctrine[] }
export interface DoctrineFolder { id: string; name: string | null; icon: string | null; grand: readonly DoctrineChoice[]; tracks: readonly DoctrineTrack[] }
export interface DoctrineMasteryReply { window: KeyedWindow; folders: readonly DoctrineFolder[]; tokens: Readonly<Record<string, string>>; warnings: readonly string[] }

const text = (value: unknown): value is string => typeof value === 'string';
const optional = (value: unknown) => value === null || text(value);
const named = (value: unknown): boolean => !!value && typeof value === 'object'
  && text((value as { id: unknown }).id) && optional((value as { name: unknown }).name) && optional((value as { icon?: unknown }).icon ?? null);
const choice = (value: unknown): boolean => named(value) && optional((value as { tokenId?: unknown }).tokenId ?? null);
const decimal = (value: unknown) => text(value) && /^-?\d+$/u.test(value);

export function validateDoctrineMastery(value: unknown): DoctrineMasteryReply {
  const reply = value as DoctrineMasteryReply;
  const reward = (item: DoctrineReward) => !!item && text(item.id) && optional(item.name) && optional(item.description)
    && (item.threshold === null || decimal(item.threshold));
  const track = (item: DoctrineTrack) => !!item && text(item.id) && optional(item.name) && Array.isArray(item.subdoctrines)
    && item.subdoctrines.every(sub => choice(sub) && (sub.rewards === null || Array.isArray(sub.rewards) && sub.rewards.every(reward)));
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.folders) || !reply.tokens || typeof reply.tokens !== 'object'
    || !Object.values(reply.tokens).every(text) || !Array.isArray(reply.warnings) || !reply.warnings.every(text)
    || !reply.folders.every(folder => named(folder) && Array.isArray(folder.grand) && folder.grand.every(choice)
      && Array.isArray(folder.tracks) && folder.tracks.every(track)))
    throw new Error('The doctrine mastery reply is malformed.');
  return { ...reply, window: validateKeyedWindow(reply.window) };
}

/** One track's recorded state at an hour. Raw values are the game's fixed-point text;
 * null where the track holds no such value then. */
export interface TrackState {
  folder: string; track: DoctrineTrack; subdoctrine: Subdoctrine | null; subToken: string | null;
  mastery: string | null; bank: string | null; daily: string | null; effective: string | null; rewards: string | null;
  /** Subdoctrines of this track the folder records as completed, in recorded order (no dates). */
  completed: Subdoctrine[];
}
export interface FolderState { folder: DoctrineFolder; grand: DoctrineChoice | null; grandToken: string | null; tracks: TrackState[] }

/** The choice a recorded interned id selects: by its recorded id, else by its spelling. */
export function chosen<C extends DoctrineChoice>(choices: readonly C[], id: string | undefined, tokens: Readonly<Record<string, string>>): C | null {
  if (id === undefined || id === '0') return null;
  return choices.find(item => item.tokenId === id) ?? choices.find(item => item.tokenId == null && item.id === tokens[id]) ?? null;
}

/** Display names where the content gives none. */
export const folderName = (folder: DoctrineFolder) => folder.name ?? 'Doctrine folder name unavailable';
export const trackName = (track: DoctrineTrack) => track.name ?? 'Track name unavailable';
export const rewardName = (reward: DoctrineReward) => reward.name ?? 'Reward name unavailable';

/** Without exactly one witnessed holder, inventory is unknown, never empty. */
export function doctrinesAt(reply: DoctrineMasteryReply, tag: string, hour: number): { folders: FolderState[]; readHour: number | undefined } | null {
  if (!keyedKnown(reply.window, hour)) return null;
  const holders = new Set<string>();
  for (const series of reply.window.series) {
    const [country, ...rest] = series.name.split('|');
    if (rest.length === 1 && rest[0] === 'present' && countryPart(country)?.tag === tag && runAtHour(series.runs, hour)) holders.add(country);
  }
  if (holders.size !== 1) return null;
  const [holder] = holders;
  const values = new Map<string, string>();
  for (const series of reply.window.series) {
    const [country, ...rest] = series.name.split('|');
    if (country !== holder) continue;
    const run = runAtHour(series.runs, hour);
    if (run) values.set(rest.join('|'), run[2]);
  }
  // A recorded selection: null for none (zero), else the interned id the series holds.
  const selection = (id: string | undefined) => id === undefined || id === '0' ? null : id;
  const folders: FolderState[] = [];
  for (const folder of reply.folders) {
    const grandId = values.get(`${folder.id}|grand`);
    const grandToken = selection(grandId);
    const completedIds = [...values].filter(([name]) => name.startsWith(`${folder.id}|completed|`))
      .sort(([a], [b]) => Number(a.split('|')[2]) - Number(b.split('|')[2])).map(([, id]) => id);
    const tracks = folder.tracks.flatMap(track => {
      const field = (name: string) => values.get(`${folder.id}|${track.id}|${name}`) ?? null;
      const fields = ['mastery', 'bank', 'daily', 'effective', 'rewards', 'sub'].map(field);
      if (fields.every(value => value === null)) return [];
      const subId = field('sub') ?? undefined;
      return [{ folder: folder.id, track, subToken: selection(subId), subdoctrine: chosen(track.subdoctrines, subId, reply.tokens),
        mastery: fields[0], bank: fields[1], daily: fields[2], effective: fields[3], rewards: fields[4],
        completed: completedIds.flatMap(id => { const sub = chosen(track.subdoctrines, id, reply.tokens); return sub ? [sub] : []; }) }];
    });
    if (!tracks.length && grandToken === null) continue;
    folders.push({ folder, grandToken, grand: chosen(folder.grand, grandId, reply.tokens), tracks });
  }
  return { folders, readHour: keyedRead(reply.window, hour) };
}

/** The meter's scale: the track's last reward threshold (null when the content gives no
 * positive one, which leaves the meter unscaled rather than invented). */
export function trackScale(track: TrackState): bigint | null {
  const thresholds = (track.subdoctrine?.rewards ?? []).flatMap(reward => reward.threshold === null ? [] : [BigInt(reward.threshold)]).filter(value => value > 0n);
  return thresholds.length ? thresholds.reduce((max, value) => value > max ? value : max) : null;
}

/** Mastery as a share of the scale, for the meter; null when either is unknown. */
export function trackRatio(track: TrackState): number | null {
  const scale = trackScale(track);
  if (scale === null || track.mastery === null) return null;
  return Number(BigInt(track.mastery) * 10000n / scale) / 10000;
}

/** Whether the reward at `ordinal` (authored order) is unlocked, by the track's recorded
 * reward count, never by comparing mastery with its requirement; null when the count is not
 * recorded. */
export function rewardUnlocked(track: TrackState, ordinal: number): boolean | null {
  if (track.rewards === null) return null;
  return BigInt(ordinal) < BigInt(track.rewards);
}

/** The rewards the recorded count certifies as unlocked, in authored order. */
export function unlockedRewards(track: TrackState): DoctrineReward[] {
  return (track.subdoctrine?.rewards ?? []).filter((_, ordinal) => rewardUnlocked(track, ordinal) === true);
}

/** Whether the recorded mastery meets a reward's authored requirement; null when unknown. Kept
 * apart from the unlock, which the game records separately. */
export function requirementMet(track: TrackState, reward: DoctrineReward): boolean | null {
  return track.mastery === null || reward.threshold === null ? null : BigInt(track.mastery) >= BigInt(reward.threshold);
}

/** A mark at each authored reward threshold, reached when the recorded count unlocks it. */
export function trackMarks(track: TrackState, labels: boolean): MeterMark[] {
  const scale = trackScale(track);
  if (scale === null) return [];
  // A reward without a known requirement has no place on the meter; ordinals stay authored.
  return (track.subdoctrine?.rewards ?? []).flatMap((reward, index) => reward.threshold === null ? [] : [{ id: `${reward.id}:${index}`,
    at: Number(BigInt(reward.threshold) * 10000n / scale) / 10000,
    reached: rewardUnlocked(track, index) === true, ...labels ? { label: rewardName(reward) } : {} }]);
}

/** "+x a day" from the effective gain, or null when not recorded. */
export const dailyText = (raw: string | null) => raw === null ? null : `${raw.startsWith('-') ? '' : '+'}${masteryText(raw)} a day`;

/** Tracks with a subdoctrine chosen, for the header. */
export const activeTracks = (folders: readonly FolderState[]) => folders.reduce((sum, folder) => sum + folder.tracks.filter(track => track.subToken !== null).length, 0);

export { masteryText };

/** A doctrine icon's sprite: the authored key's single image. */
export const doctrineSprite = (key: string | null | undefined): SpriteFrame | null => key ? { key, frame: 0 } : null;

/** Every sprite key the reply names for folders, grand doctrines and subdoctrines; the
 * images load through the shared sprite route. */
export function doctrineIcons(reply: DoctrineMasteryReply | undefined): (string | null)[] {
  return (reply?.folders ?? []).flatMap(folder => [folder.icon, ...folder.grand.map(grand => grand.icon),
    ...folder.tracks.flatMap(track => track.subdoctrines.map(sub => sub.icon))]);
}

/** Reward descriptions from a reply asked at one hour, keyed by folder, track, subdoctrine
 * and reward position; the history's own descriptions keep only the fixed fragments. */
export function rewardDescriptions(reply: DoctrineMasteryReply): Map<string, string | null> {
  return new Map(reply.folders.flatMap(folder => folder.tracks.flatMap(track => track.subdoctrines.flatMap(sub =>
    (sub.rewards ?? []).map((reward, index) => [rewardKey(folder.id, track.id, sub.id, index), reward.description] as const)))));
}

export const rewardKey = (folder: string, track: string, subdoctrine: string, index: number) => `${folder}|${track}|${subdoctrine}|${index}`;
