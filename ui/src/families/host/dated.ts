import { revisionOf } from '../../dashboard/follow/payloads.ts';
import type { SupplyChange } from '../../dashboard/follow/pipeline.ts';
import type { FamilyStaticProof, FamilyStaticSources } from './adapter.ts';

export interface BoundDated {
  readonly h: number;
  readonly c: number;
}

export function boundDated(
  staticSources: Pick<FamilyStaticSources, 'hour' | 'follows'> | null,
  proof: readonly [lo: number, hi: number] | null,
  hasSubscriptions: boolean,
  datedByBounds: boolean,
  bounds: readonly [number, number] | null,
): BoundDated | undefined {
  if (staticSources?.follows && proof) return { h: proof[0], c: proof[1] };
  if (hasSubscriptions) return undefined;
  if (staticSources) return proof ? { h: proof[0], c: proof[1] } : { h: staticSources.hour, c: staticSources.hour };
  return datedByBounds && bounds ? { h: bounds[0], c: bounds[1] } : undefined;
}

export function ownProof(staticSources: FamilyStaticSources | null, proof: ReturnType<FamilyStaticProof['read']>): readonly [number, number] | null {
  if (!staticSources) return null;
  return proof && proof.bundle === staticSources.stamp ? proof.proof : staticSources.proof;
}

const sameDated = (a: BoundDated | null, b: BoundDated | null) => a === b || (!!a && !!b && a.h === b.h && a.c === b.c);

export class StaticSupply {
  private readonly send: (change: SupplyChange) => void;
  private held: FamilyStaticSources | null | undefined = undefined;
  private readonly groups = new Map<string, Readonly<Record<string, unknown>>>();
  private dated: BoundDated | null = null;

  constructor(send: (change: SupplyChange) => void) {
    this.send = send;
  }

  statics(sources: FamilyStaticSources | null, dated: BoundDated | null): void {
    this.held = sources;
    this.dated = dated;
    this.send({ ...this.payload(), dated });
  }

  data(fields: Readonly<Record<string, unknown>>, group = 'data'): void {
    this.update({ [group]: fields });
  }

  update(entries: Readonly<Record<string, Readonly<Record<string, unknown>>>>): void {
    let changed = false;
    for (const [group, fields] of Object.entries(entries)) {
      const was = this.groups.get(group) ?? {};
      const names = new Set([...Object.keys(was), ...Object.keys(fields)]);
      if ([...names].every(name => was[name] === fields[name])) continue;
      this.groups.set(group, fields);
      changed = true;
    }
    if (changed && this.held !== undefined) this.send(this.payload());
  }

  private payload(): SupplyChange {
    const sources = this.held ?? null;
    const data = Object.fromEntries([...this.groups.values()].flatMap(fields => Object.entries(fields)).filter(([, value]) => value !== undefined));
    return {
      ...sources ? { stamp: revisionOf(sources.stamp) } : {},
      statics: sources || Object.keys(data).length
        ? { ...sources ? { staticPanels: sources.panels, staticAnswers: sources.answers, ...(sources.shifts ? { staticShifts: sources.shifts } : {}) } : {}, ...data }
        : null,
    };
  }

  date(sources: FamilyStaticSources | null, dated: BoundDated | null): void {
    if (sources !== this.held || sameDated(dated, this.dated)) return;
    this.dated = dated;
    this.send({ ...sources ? { stamp: revisionOf(sources.stamp) } : {}, dated });
  }
}
