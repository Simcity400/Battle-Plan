import { useMemo } from 'react';
import { nameKey, useCountryFactsAt } from '../../dashboard/countryNamesAt.ts';
import type { FactSpec, NameAt } from '../../dashboard/countryFacts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { UnitLeaderPortrait } from '../../generated/dashboard';
import type { LeaderRow } from './generalsData';
import { validateLeaderPortraitsReply } from './generalsData.ts';

type PortraitFact = UnitLeaderPortrait & { warnings: readonly string[] };
const specs = new Map<number, FactSpec<PortraitFact>>();
export function portraitSpec(edge: number): FactSpec<PortraitFact> {
  let spec = specs.get(edge);
  if (!spec) {
    spec = { identity: `unit-leader-portraits:${edge}`, product: 'unit-leader-portraits', perRequest: Infinity,
      params: (branchId, hour, references) => ({ branchId, hour, references, edge }),
      extract: data => {
        const reply = validateLeaderPortraitsReply(data);
        return new Map(reply.portraits.map(p => [p.reference, { ...p, warnings: reply.warnings }]));
      },
    };
    specs.set(edge, spec);
  }
  return spec;
}

export function useLeaderPortraits(runtime: WidgetRuntime, track: BoundTrack | undefined, rows: readonly LeaderRow[], edge: number, active: boolean) {
  const wanted = useMemo<NameAt[]>(() => rows.map(row => ({ hour: row.person.portraitHour, tag: row.person.portraitReference })), [rows]);
  const spec = portraitSpec(edge);
  const portraits = useCountryFactsAt(runtime.client, track, wanted, active, spec);
  return {
    at: (row: LeaderRow) => portraits.get(nameKey(row.person.portraitHour, row.person.portraitReference)),
    warnings: [...new Set([...portraits.values()].flatMap(portrait => portrait.warnings))],
  };
}
