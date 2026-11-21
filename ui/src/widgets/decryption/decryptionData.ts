import { formatAmount } from '../../charts/valueAxis.ts';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { formatDate, gameDate } from '../../dashboard/calendar.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { CipherPair, Country, DecryptionReply, Observation } from '../../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const country = (value: unknown) => object(value) && Number.isSafeInteger(value.id) && (value.tag === null || typeof value.tag === 'string');
const raw = (value: unknown) => typeof value === 'string' && /^-?(0|[1-9]\d{0,18})$/.test(value);
const byte = (value: unknown) => Number.isInteger(value) && (value as number) >= 0 && (value as number) <= 255;
const text = (value: unknown) => typeof value === 'string' && value.length > 0;
const observation = (value: unknown) => value === null || object(value) && Number.isSafeInteger(value.hour) && typeof value.upperBound === 'boolean';

export function validateDecryption(value: unknown): DecryptionReply {
  const ok = object(value) && Array.isArray(value.pairs) && (value.missing === null || text(value.missing))
    && (value.attemptsUnknown === null || text(value.attemptsUnknown)) && Array.isArray(value.warnings) && value.warnings.every(item => typeof item === 'string')
    && value.pairs.every(pair => object(pair) && country(pair.owner) && country(pair.target)
      && (pair.thresholdRaw === null ? text(pair.thresholdUnknown) : raw(pair.thresholdRaw) && pair.thresholdUnknown === null)
      && byte(pair.bonusActive) && byte(pair.decrypting) && (pair.broken === null || typeof pair.broken === 'boolean')
      && observation(pair.started) && observation(pair.brokenAt));
  if (!ok) throw new Error('The decryption reply is malformed.');
  return value as unknown as DecryptionReply;
}

export const pairKey = (pair: CipherPair) => `${pair.owner.id}:${pair.target.id}`;

export function progressPercent(amount: bigint, active: number, threshold: bigint): number | null {
  let ratio: bigint;
  if (threshold === 0n || BigInt.asUintN(64, BigInt.asUintN(64, amount) + 0x7fff_ffff_ffff_fffen) > 0xffff_ffff_ffff_fffcn) {
    ratio = 0xffff_ffffn;
  } else {
    const numerator = BigInt.asIntN(64, amount * 100_000n);
    if (numerator === -(1n << 63n) && threshold === -1n) return null;
    ratio = numerator / threshold;
  }
  if (active !== 0) ratio = 100_000n;
  const percent = BigInt.asIntN(64, ratio * 100n);
  const rounded = BigInt.asIntN(64, percent + (percent < 0n ? -50_000n : 50_000n)) / 100_000n;
  return Math.max(1, Math.min(100, Number(BigInt.asIntN(32, rounded))));
}

export interface CipherNumbers {
  amount: number | undefined;
  bonusDays: number | undefined;
  missing: string;
}

export function cipherNumbers(values: LiveValues<number>, pair: CipherPair): CipherNumbers {
  const key = pairKey(pair);
  const amount = values.get(liveKey(key, 'amount'));
  return { amount, bonusDays: values.get(liveKey(key, 'bonusDays')), missing: amount === undefined ? values.missing(liveKey(key, 'amount')) : '' };
}

export function percentOf(pair: CipherPair, numbers: CipherNumbers): number | null {
  if (pair.thresholdRaw === null || numbers.amount === undefined || !Number.isInteger(numbers.amount)) return null;
  return progressPercent(BigInt(numbers.amount), pair.bonusActive, BigInt(pair.thresholdRaw));
}

export const CIPHER_BROKEN = 'Cipher broken';
export const progressLabel = (percent: number | null) => percent === null ? 'Progress unknown' : `${percent}%`;
export const amountText = (numbers: CipherNumbers) => numbers.amount === undefined || !Number.isInteger(numbers.amount) ? `Amount not read (${numbers.missing})`
  : formatAmount(BigInt(numbers.amount), 5, 5);
export const progressText = (percent: number | null, numbers: CipherNumbers) => percent === null ? `${progressLabel(percent)} · ${amountText(numbers)}` : progressLabel(percent);
export const activeBonusDays = (pair: CipherPair, numbers: CipherNumbers): number | null =>
  pair.broken === true && pair.bonusActive !== 0 && numbers.bonusDays !== undefined ? numbers.bonusDays : null;
export const bonusText = (days: number | null) => days === null ? '' : `${days >= 0 ? '+' : ''}${days} bonus days`;

export const workingStatus = (pair: CipherPair, capitalized = false) => {
  const status = pair.decrypting !== 0 ? 'decrypting' : 'paused';
  return capitalized ? status[0].toUpperCase() + status.slice(1) : status;
};

export function observationDate(observation: Observation | null, unknown = 'Not recorded'): string {
  if (!observation) return unknown;
  const date = gameDate(observation.hour);
  return `${observation.upperBound ? 'by ' : ''}${date ? formatDate(date) : `hour ${observation.hour}`}`;
}

const attemptUnknown = (reply: Pick<DecryptionReply, 'attemptsUnknown'>) => reply.attemptsUnknown === null ? 'Not recorded' : 'Not known';

export const cipherStatus = (pair: CipherPair, reply: Pick<DecryptionReply, 'attemptsUnknown'>) => pair.broken === true
  ? `${CIPHER_BROKEN} ${observationDate(pair.brokenAt, attemptUnknown(reply).toLowerCase())}`
  : pair.broken === null ? `${workingStatus(pair, true)}; cipher status unknown` : workingStatus(pair, true);

export type RaceOrder = 'progress' | 'decryptor';
export type CountrySide = 'decryptor' | 'target';

const countryName = (value: Country) => value.tag ?? `Country ${value.id}`;

export function raceRows(reply: Pick<DecryptionReply, 'pairs'>, order: RaceOrder, side: CountrySide, hour: number,
  matches?: (tag: string, hour: number) => boolean): CipherPair[] {
  const shown = matches ? reply.pairs.filter(pair => {
    const tag = (side === 'decryptor' ? pair.owner : pair.target).tag;
    return tag !== null && matches(tag, hour);
  }) : [...reply.pairs];
  return order === 'decryptor' ? shown.sort((a, b) => compareTags(countryName(a.owner), countryName(b.owner)) || a.owner.id - b.owner.id) : shown;
}

export function decryptionTooltip(pair: CipherPair, reply: Pick<DecryptionReply, 'attemptsUnknown'>, numbers: CipherNumbers,
  owner: ReturnType<CountryLabel> | null, target: ReturnType<CountryLabel> | null, height: number): TooltipContent {
  const percent = percentOf(pair, numbers);
  const bonus = activeBonusDays(pair, numbers);
  const flag = (label: ReturnType<CountryLabel> | null) => label === null ? undefined
    : { flag: label.flag, flagReason: label.flagReason, ...(label.flagDetail !== undefined ? { flagDetail: label.flagDetail } : {}), flagPending: label.flagPending };
  const content: TooltipContent = { title: 'Decryption', rows: [
    { ...countryNameDetails(owner), label: 'Decryptor', value: owner?.name ?? countryName(pair.owner), countryFlag: flag(owner) },
    { ...countryNameDetails(target), label: 'Target', value: target?.name ?? countryName(pair.target), countryFlag: flag(target) },
    { label: 'Progress', value: progressText(percent, numbers) },
    ...(pair.thresholdUnknown === null ? [] : [{ label: 'Threshold', value: `Not known: ${pair.thresholdUnknown}` }]),
    { label: 'Started', value: observationDate(pair.started, attemptUnknown(reply)) },
    ...(bonus === null ? [] : [{ label: 'Bonus days', value: String(bonus) }]),
    { label: 'Status', value: cipherStatus(pair, reply), divider: true },
  ] };
  return fitTooltip([content], height, { keep: 1 });
}

export const raceStructure = (reply: DecryptionReply) => JSON.stringify([reply.missing, reply.attemptsUnknown, reply.pairs, reply.warnings]);
