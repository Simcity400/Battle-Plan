import type { ProductRequest } from '../../dashboard/client';
import type { CountryParams, ResearchReply } from '../../generated/dashboard';

export const PRODUCT = 'research';

export function researchRequest(sourceId: string, revision: string, params: CountryParams): ProductRequest<CountryParams> {
  return { sourceId, revision, product: PRODUCT, params };
}
import type { ResearchHistory, ResearchSlotHistory, ResearchSlotState } from '../../generated/dashboard';

export interface ResearchColumn { slot: ResearchSlotHistory; offset: number }

/** Playback never exposes future outcomes, assignments, or slot additions. */
export function researchAt(history: ResearchHistory, hour: number): ResearchSlotHistory[] | null {
  if (!history.coverage.some(c => c.firstHour <= hour && hour <= c.lastHour)) return null;
  return history.slots.filter(s => s.availability[0]?.firstHour <= hour).map(slot => ({
    ...slot,
    availability: slot.availability.filter(c => c.firstHour <= hour).map(c => ({ ...c, lastHour: Math.min(c.lastHour, hour) })),
    states: (slot.states ?? []).filter(s => s.firstHour <= hour).map(s => ({ ...s, lastHour: Math.min(s.lastHour, hour) })),
    progress: (slot.progress ?? []).filter(p => p.firstHour <= hour).map(p => ({ ...p, lastHour: Math.min(p.lastHour, hour) })),
    entries: slot.entries.filter(e => e.firstObservedHour <= hour).map(e => ({ ...e,
      lastObservedHour: Math.min(e.lastObservedHour, hour),
      endedByHour: e.endedByHour !== null && e.endedByHour <= hour ? e.endedByHour : null,
      outcome: e.endedByHour !== null && e.endedByHour <= hour
        ? e.outcomeObservedHour != null && e.outcomeObservedHour > hour ? 'unknown' : e.outcome : 'active',
    })),
  }));
}

export function researchStateAt(slot: ResearchSlotHistory, hour: number): ResearchSlotState | null {
  if (!slot.availability.some(span => span.firstHour <= hour && hour <= span.lastHour)) return null;
  const state = slot.states?.find(s => s.firstHour <= hour && hour <= s.lastHour);
  if (!state) return null;
  return state.details?.remainingDays == null ? state : { ...state, details: { ...state.details,
    remainingDays: Math.max(0, state.details.remainingDays - Math.floor((hour - state.firstHour) / 24)),
  } };
}

/** Recorded progress toward an assignment's current base cost at `hour`, as a
 * 0-1 fraction. Null without a sample at that hour or a known positive cost;
 * never carried across hours that were not recorded. */
export function researchProgressAt(slot: ResearchSlotHistory, entryFirstObservedHour: number, hour: number): number | null {
  const sample = slot.progress?.find(p => p.entryFirstObservedHour === entryFirstObservedHour && p.firstHour <= hour && hour <= p.lastHour);
  if (!sample?.baseDaysRaw) return null;
  const base = BigInt(sample.baseDaysRaw), done = BigInt(sample.progressRaw);
  if (base <= 0n || done < 0n) return null;
  return done >= base ? 1 : Number(done * 10_000n / base) / 10_000;
}

/** EC1810/ABE4C0 integer formula, evaluated at the selected hour. */
export function researchRateAt(state: ResearchSlotState, hour: number): { rateRaw: string; penaltyRaw: string; floored: boolean } | null {
  const modifiers = state.details?.modifiers;
  if (!modifiers || state.aheadReductionYearsRaw === null) return null;
  const reduction = BigInt(state.aheadReductionYearsRaw) * 365n;
  const rounded = ((reduction < 0n ? -reduction : reduction) + 50000n) / 100000n * (reduction < 0n ? -1n : 1n);
  const ahead = BigInt(modifiers.year) * 365n - (BigInt(hour) - 43800000n) / 24n - rounded;
  const penalty = (ahead > 0n ? ahead : 0n) * BigInt(modifiers.aheadFactorRaw) / 365n;
  if (100000n + penalty <= 0n) return null;
  const rate = (100000n + BigInt(modifiers.totalBonusRaw)) * 100000n / (100000n + penalty);
  const minimum = BigInt(modifiers.minimumSpeedRaw);
  return { rateRaw: (rate > minimum ? rate : minimum).toString(), penaltyRaw: penalty.toString(), floored: rate < minimum };
}

/** Only a slot's initial offset uses time. Every subsequent item occupies the
 * next row in that slot, regardless of other slots' dates or durations. */
export function researchColumns(slots: ResearchSlotHistory[]): ResearchColumn[] {
  const columns: ResearchColumn[] = [];
  for (const slot of [...slots].sort((a, b) => a.availability[0].firstHour - b.availability[0].firstHour || a.index - b.index)) {
    const appeared = slot.availability[0].firstHour;
    let offset = 0;
    for (const previous of columns) {
      if (previous.slot.availability[0].firstHour >= appeared) continue;
      const finished = previous.slot.entries.filter(e => e.endedByHour !== null && e.endedByHour <= appeared).length;
      offset = Math.max(offset, previous.offset + finished);
    }
    columns.push({ slot, offset });
  }
  return columns.sort((a, b) => a.slot.index - b.slot.index);
}

/** Shape checks on the product payload; identity is checked by the transport. */
export function validateResearchReply(input: unknown): ResearchReply {
  const reply = input as ResearchReply;
  const hour = (n: number) => Number.isInteger(n) && n >= -2147483648 && n <= 2147483647;
  const intervals = (spans: ResearchHistory['coverage']) => {
    let last = -Infinity;
    return Array.isArray(spans) && spans.length <= 100000 && spans.every(s => {
      const valid = hour(s.firstHour) && hour(s.lastHour) && s.firstHour > last && s.lastHour >= s.firstHour;
      last = s.lastHour;
      return valid;
    });
  };
  const fixed = (value: unknown): value is string => typeof value === 'string' && /^-?\d{1,19}$/.test(value)
    && BigInt(value) >= -9223372036854775808n && BigInt(value) <= 9223372036854775807n;
  let index = -1, entries = 0, states = 0, progress = 0;
  const valid = !!reply && typeof reply === 'object' && Array.isArray(reply.artwork) && Array.isArray(reply.history?.slots)
    && intervals(reply.history.coverage) && reply.history.slots.length <= 64 && reply.history.slots.every(s => {
      let last = -Infinity;
      const valid = Number.isInteger(s.index) && s.index > index && s.index < 64 && s.availability.length > 0 && intervals(s.availability)
        && s.entries.every(e => {
          const ok = typeof e.token === 'string' && e.token.length > 0 && e.token.length <= 512 && Number.isInteger(e.level)
            && Number.isInteger(e.technologyHandle) && e.technologyHandle >= 0 && e.technologyHandle <= 4294967295
            && hour(e.continuityStartHour) && e.continuityStartHour <= e.firstObservedHour
            && hour(e.firstObservedHour) && e.firstObservedHour >= last && hour(e.lastObservedHour) && e.lastObservedHour >= e.firstObservedHour
            && (e.startedAfterHour === null || (hour(e.startedAfterHour) && e.startedAfterHour < e.firstObservedHour))
            && (e.outcomeObservedHour == null || (hour(e.outcomeObservedHour) && e.endedByHour !== null && e.outcomeObservedHour >= e.endedByHour))
            && ['active', 'completed', 'switched', 'unknown'].includes(e.outcome)
            && (e.endedByHour === null ? e.outcome === 'active' : hour(e.endedByHour) && e.endedByHour >= e.lastObservedHour && e.outcome !== 'active');
          last = e.endedByHour ?? Infinity;
          entries++;
          return ok;
        }) && intervals(s.progress) && s.progress.every(sample => {
          progress++;
          return s.entries.some(e => e.firstObservedHour === sample.entryFirstObservedHour
            && sample.firstHour >= e.firstObservedHour
            && sample.lastHour <= (e.endedByHour ?? e.lastObservedHour))
            && s.availability.some(span => span.firstHour <= sample.firstHour && sample.lastHour <= span.lastHour)
            && fixed(sample.progressRaw) && (sample.baseDaysRaw === null || (fixed(sample.baseDaysRaw) && BigInt(sample.baseDaysRaw) > 0n));
        }) && (s.states === undefined || (intervals(s.states) && s.states.every(state => {
          states++;
          return (state.token === null || (typeof state.token === 'string' && state.token.length > 0 && state.token.length <= 512))
            && (state.entryFirstObservedHour === null || s.entries.some(e => e.firstObservedHour === state.entryFirstObservedHour && e.token === state.token))
            && s.availability.some(span => span.firstHour <= state.firstHour && state.lastHour <= span.lastHour)
            && fixed(state.speedMultiplierRaw) && fixed(state.bankedDaysRaw) && fixed(state.appliedDaysRaw)
            && (state.aheadReductionYearsRaw === null || fixed(state.aheadReductionYearsRaw))
            && (state.details == null || (
              (state.details.remainingDays === null || (Number.isInteger(state.details.remainingDays) && state.details.remainingDays >= 0 && state.details.remainingDays <= 4294967295))
              && (state.details.modifiers == null || (hour(state.details.modifiers.year)
                && fixed(state.details.modifiers.totalBonusRaw) && fixed(state.details.modifiers.aheadFactorRaw) && fixed(state.details.modifiers.minimumSpeedRaw)
                && Array.isArray(state.details.modifiers.components) && state.details.modifiers.components.length === 8
                && new Set(state.details.modifiers.components.map(c => c.kind)).size === 8
                && state.details.modifiers.components.every(c => ['country', 'category', 'designTeam', 'experience', 'license', 'scientist', 'applied', 'sharing'].includes(c.kind)
                  && fixed(c.raw) && Array.isArray(c.sources) && c.sources.length <= 4096 && c.sources.every(source => fixed(source.raw)
                    && (source.token === null || (typeof source.token === 'string' && source.token.length <= 512))
                    && (source.name === null || (typeof source.name === 'string' && source.name.length <= 4096))))))
              && (state.details.bonuses === null || (Array.isArray(state.details.bonuses) && state.details.bonuses.length <= 4096
                && state.details.bonuses.every(b => Number.isInteger(b.id) && b.id >= -2147483648 && b.id <= 2147483647
                  && (b.token === null || (typeof b.token === 'string' && b.token.length <= 512))
                  && (b.name === null || (typeof b.name === 'string' && b.name.length <= 4096))
                  && (b.speedBonusRaw === null || fixed(b.speedBonusRaw))
                  && (b.costReductionRaw == null || fixed(b.costReductionRaw))
                  && (b.aheadReductionYearsRaw === null || fixed(b.aheadReductionYearsRaw)))))));
        })));
      index = s.index;
      return valid;
    }) && entries <= 100000 && states <= 100000 && progress <= 100000 && reply.artwork.length <= 2048 && reply.artwork.every(a =>
      a.icon === null || (a.icon.length <= 128 * 1024 && /^data:image\/png;base64,[A-Za-z0-9+/]+=*$/.test(a.icon)));
  if (!valid) throw new Error('Research history does not match this country and recording.');
  return reply;
}

/** Only the finishing slot owns a historical completion. Switched assignments
 * remain in the full history for transfer/credit calculations. */
export function researchView(slots: ResearchSlotHistory[]): ResearchSlotHistory[] {
  return slots.map(slot => ({ ...slot, entries: slot.entries.filter(e => e.outcome === 'completed') }));
}
