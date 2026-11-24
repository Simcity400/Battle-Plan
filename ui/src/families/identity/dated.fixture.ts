import type { IdentityProvider } from '../contract.ts';
import { IdentityIndex } from './index.ts';
import { createSnapshotIdentityProvider } from './provider.ts';

export interface DatedLook {
  readonly from: number;
  readonly name: string | null;
  readonly colour?: readonly [number, number, number];
}

const LAST_HOUR = 2 ** 40;

export function datedIdentity(runs: Readonly<Record<string, readonly DatedLook[]>>): IdentityProvider {
  const index = new IdentityIndex({ sourceId: 'fixture', revision: '1', branchId: '1', content: 1 });
  for (const [tag, list] of Object.entries(runs)) {
    list.forEach((look, at) => {
      const next = list[at + 1]?.from;
      index.ingestStyle({
        tag,
        name: look.name,
        color: look.colour ? [look.colour[0], look.colour[1], look.colour[2]] : null,
        valid: [look.from, next === undefined ? LAST_HOUR : next - 1],
        flagChoice: { keys: [tag, null, null], reasons: [null, null, null] },
      }, look.from);
    });
  }
  return createSnapshotIdentityProvider(index);
}
