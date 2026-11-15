import { useMemo } from 'react';
import { nameKey, useCountryFactsAt } from '../../dashboard/countryNamesAt.ts';
import type { FactSpec, NameAt } from '../../dashboard/countryFacts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { UnitLeaderPortrait, UnitLeaderPortraitsReply } from './types';

type PortraitFact = UnitLeaderPortrait & { warnings: readonly string[] };

/** A portrait to draw: a leader, at an hour that does not move with playback (the hour its row was first seen). */
export interface PortraitAt { reference: string; hour: number }

export function validatePortraits(input: unknown): UnitLeaderPortraitsReply {
  if (typeof input !== 'object' || input === null || !Array.isArray((input as UnitLeaderPortraitsReply).portraits)) throw new Error('The unit leader portraits reply is malformed.');
  return input as UnitLeaderPortraitsReply;
}

/** Each portrait is a texture decoded from the game's files the first time it is drawn (about 4 ms): a request names
 * a few, in the order the list shows them, so the rows in view are drawn first and no reply waits on a whole roster. */
export const PORTRAITS_PER_REQUEST = 4;

const specs = new Map<number, FactSpec<PortraitFact>>();
export function portraitSpec(edge: number): FactSpec<PortraitFact> {
  let spec = specs.get(edge);
  if (!spec) {
    spec = { identity: `unit-leader-portraits:${edge}`, product: 'unit-leader-portraits', perRequest: PORTRAITS_PER_REQUEST, requestOrder: 'wanted',
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
