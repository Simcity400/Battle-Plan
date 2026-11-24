import type { IdentityProvider } from '../contract.ts';

export interface DatedLook {
  readonly from: number;
  readonly name: string;
  readonly colour?: string;
}

export function datedIdentity(runs: Readonly<Record<string, readonly DatedLook[]>>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const list = kind === 'country' ? runs[key] ?? [] : [];
      let at = -1;
      for (let i = 0; i < list.length; i++) if (list[i]!.from <= atHour) at = i;
      const look = at >= 0 ? list[at] : undefined;
      const next = list[at + 1]?.from ?? Infinity;
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        name: look ? { status: 'ready', value: look.name } : { status: 'absent', reason: 'Name not recorded' },
        colour: look?.colour ? { status: 'ready', value: look.colour } : { status: 'absent', reason: 'Colour not recorded' },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [look?.from ?? 0, next - 1],
        nextChangeAt: next,
      };
    },
  };
}
