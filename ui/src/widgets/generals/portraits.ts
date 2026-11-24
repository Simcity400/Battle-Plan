import { useMemo } from 'react';
import { nameKey, useCountryFactsAt } from '../../dashboard/countryNamesAt.ts';
import type { FactSpec, NameAt } from '../../dashboard/countryFacts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { UnitLeaderPortrait, UnitLeaderPortraitsReply } from './types';

type PortraitFact = UnitLeaderPortrait & { warnings: readonly string[] };

export interface PortraitAt { reference: string; hour: number }

export function validatePortraits(input: unknown): UnitLeaderPortraitsReply {
  if (typeof input !== 'object' || input === null || !Array.isArray((input as UnitLeaderPortraitsReply).portraits)) throw new Error('The unit leader portraits reply is malformed.');
  return input as UnitLeaderPortraitsReply;
}

export const PORTRAITS_PER_REQUEST = 64;

const specs = new Map<number, FactSpec<PortraitFact>>();
export function portraitSpec(edge: number): FactSpec<PortraitFact> {
  let spec = specs.get(edge);
  if (!spec) {
    spec = { identity: `unit-leader-portraits:${edge}`, product: 'unit-leader-portraits', perRequest: PORTRAITS_PER_REQUEST, requestOrder: 'wanted', shared: true,
      params: (branchId, hour, references) => ({ branchId, hour, references, edge }),
      extract: data => {
        const reply = validatePortraits(data);
        return new Map(reply.portraits.map(p => [p.reference, { ...p, warnings: reply.warnings }]));
      },
    };
    specs.set(edge, spec);
  }
  return spec;
}

export function useLeaderPortraits(runtime: WidgetRuntime, track: BoundTrack | undefined, shown: readonly PortraitAt[], edge: number, active: boolean) {
  const wanted = useMemo<NameAt[]>(() => shown.map(at => ({ hour: at.hour, tag: at.reference })), [shown]);
  const portraits = useCountryFactsAt(runtime.client, track, wanted, active, portraitSpec(edge));
  return {
    at: (at: PortraitAt) => portraits.get(nameKey(at.hour, at.reference)),
    warnings: [...new Set([...portraits.values()].flatMap(portrait => portrait instanceof Error ? [portrait.message] : portrait.warnings))],
  };
}
