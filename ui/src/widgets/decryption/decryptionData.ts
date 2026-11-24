import { formatAmount } from '../../charts/valueAxis.ts';
import { formatDate, gameDate } from '../../dashboard/calendar.ts';
import type { CipherPair, DecryptionReply, Observation } from '../../generated/dashboard';

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
      && observation(pair.started) && observation(pair.brokenAt)
      && (pair.weekChangeTenths === null ? text(pair.weekChangeUnknown) : Number.isSafeInteger(pair.weekChangeTenths) && pair.weekChangeUnknown === null));
  if (!ok) throw new Error('The decryption reply is malformed.');
  return value as unknown as DecryptionReply;
}

export const pairKey = (pair: CipherPair) => `${pair.owner.id}:${pair.target.id}`;

export const weekChangeText = (tenths: number) => `${tenths > 0 ? '+' : tenths < 0 ? '−' : ''}${(Math.abs(tenths) / 10).toFixed(1)} points`;

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

export function percentOf(pair: CipherPair, numbers: CipherNumbers): number | null {
  if (pair.thresholdRaw === null || numbers.amount === undefined || !Number.isInteger(numbers.amount)) return null;
  return progressPercent(BigInt(numbers.amount), pair.bonusActive, BigInt(pair.thresholdRaw));
}

const CIPHER_BROKEN = 'Cipher broken';
export const progressLabel = (percent: number | null) => percent === null ? 'Progress unknown' : `${percent}%`;
export const amountText = (numbers: CipherNumbers) => numbers.amount === undefined || !Number.isInteger(numbers.amount) ? `Amount not read (${numbers.missing})`
  : formatAmount(BigInt(numbers.amount), 5, 5);
export const activeBonusDays = (pair: CipherPair, numbers: CipherNumbers): number | null =>
  pair.broken === true && pair.bonusActive !== 0 && numbers.bonusDays !== undefined ? numbers.bonusDays : null;

export const workingStatus = (pair: CipherPair, capitalized = false) => {
  const status = pair.decrypting !== 0 ? 'decrypting' : 'paused';
  return capitalized ? status[0].toUpperCase() + status.slice(1) : status;
};

function observationDate(observation: Observation | null, unknown = 'Not recorded'): string {
  if (!observation) return unknown;
  const date = gameDate(observation.hour);
  return `${observation.upperBound ? 'by ' : ''}${date ? formatDate(date) : `hour ${observation.hour}`}`;
}

const attemptUnknown = (reply: Pick<DecryptionReply, 'attemptsUnknown'>) => reply.attemptsUnknown === null ? 'Not recorded' : 'Not known';

export const cipherStatus = (pair: CipherPair, reply: Pick<DecryptionReply, 'attemptsUnknown'>) => pair.broken === true
  ? `${CIPHER_BROKEN} ${observationDate(pair.brokenAt, attemptUnknown(reply).toLowerCase())}`
  : pair.broken === null ? `${workingStatus(pair, true)}; cipher status unknown` : workingStatus(pair, true);

export type CountrySide = 'decryptor' | 'target';
