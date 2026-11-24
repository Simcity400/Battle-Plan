import type { MinefieldsReply } from '../../generated/dashboard';

const text = (value: unknown): value is string => typeof value === 'string';
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(text);
const count = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;

export function validateMinefields(value: unknown): MinefieldsReply {
  const reply = value as MinefieldsReply;
  if (!reply || typeof reply !== 'object' || !Number.isSafeInteger(reply.scale) || reply.scale <= 0 || !strings(reply.warnings)
    || !Array.isArray(reply.regions) || !reply.regions.every(region => !!region && count(region.region) && count(region.unknownOwners)
      && Array.isArray(region.owners) && region.owners.every(owner => !!owner && text(owner.tag)))
    || !Array.isArray(reply.unknown) || !reply.unknown.every(region => !!region && count(region.region) && count(region.unknownOwners))
    || !Array.isArray(reply.choices) || !reply.choices.every(choice => !!choice && text(choice.id) && text(choice.name)))
    throw new Error('The minefields reply is malformed.');
  return reply;
}

export interface RegionRow { region: string; name: string; owners: { tag: string }[]; unknownOwners: number }

export function minesText(value: bigint, scale: number): string {
  const negative = value < 0n, magnitude = negative ? -value : value, unit = BigInt(scale);
  const fraction = (magnitude % unit).toString().padStart(String(scale).length - 1, '0').replace(/0+$/u, '');
  return `${negative ? '−' : ''}${(magnitude / unit).toLocaleString('en-US')}${fraction ? `.${fraction}` : ''}`;
}
