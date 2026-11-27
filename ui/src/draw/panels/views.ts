import type { LiveValues } from '../../charts/live';
import type { PanelGeometry, PanelView } from './view';
import type { Structure } from '../views/fields';
import type { DoctrineMasteryReply, ResearchSlotsReply, WargoalsReply } from '../../generated/dashboard';
import { liveKey } from '../../charts/live.ts';
import { slotsStructure, slotValues } from '../../widgets/research/researchSlots.ts';
import { validateResearchSlots } from '../../widgets/research/researchData.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { hasLogRowIdentity, MISSING_LOG_ROW_IDENTITY } from '../../families/table/rowSource.ts';
import { technologyEffectsStructure, validateTechnologyEffects } from '../../widgets/research/technologyEffects.ts';
import { entityStatsStructure, validateEntityStats } from '../../widgets/research/entityStats.ts';

function recordView<T extends object>(validate: (raw: unknown) => T, shapeOf: (record: T) => string,
  fields: (record: T, values: LiveValues<number>, hour: number) => Map<string, string>, geometry: PanelGeometry, nextChange?: (reply: T, hour: number) => number) {
  const records = new WeakMap<object, { reply: T; shape: string; fields?: Map<string, string>; hour?: number }>();
  function record(raw: unknown) {
    let saved = records.get(raw as object);
    if (!saved) {
      const reply = validate(raw);
      saved = { reply, shape: shapeOf(reply) };
      records.set(raw as object, saved);
    }
    return saved;
  }
  return { geometry, structure: (raw: unknown) => record(raw).shape,
    nextChange: nextChange && ((raw: unknown, hour: number) => nextChange(record(raw).reply, hour)),
    values(raw: unknown, live: LiveValues<number> | null, hour: number, shape: string) {
      const saved = record(raw);
      if (saved.shape !== shape) return null;
      if (live) return fields(saved.reply, live, hour);
      if (!saved.fields || nextChange && saved.hour !== hour) {
        saved.fields = fields(saved.reply, repliedCells(raw).read(), hour);
        saved.hour = hour;
      }
      return saved.fields;
    },
  };
}

const logTailStructure = (raw: unknown): string => {
  const r = raw as { total?: string | number; offset?: string; rows?: readonly { key?: string }[]; missing?: readonly { firstHour?: number; lastHour?: number; reason?: string }[] };
  const missing = (r.missing ?? []).map(span => `${span.firstHour ?? ''}-${span.lastHour ?? ''}:${span.reason ?? ''}`).join(',');
  return `${r.total ?? ''}|${r.offset ?? ''}|${r.rows?.[0]?.key ?? ''}|${r.rows?.length ?? 0}|${missing}`;
};

const tableStructures: Readonly<Record<string, (raw: unknown, hour: number) => string>> = {
  'technology-effects': raw => technologyEffectsStructure(validateTechnologyEffects(raw)),
  'content-entity-stats': raw => entityStatsStructure(validateEntityStats(raw)),
  'log-tail': logTailStructure,
  'market-ledger-tail': logTailStructure,
};

export function readingStructure(product: string | undefined): Structure | undefined {
  const view = product === undefined ? undefined : panelViews[product]?.structure, table = product === undefined ? undefined : tableStructures[product];
  if (view) return (panel, hour) => panel.data === undefined ? undefined : view(panel.data, hour);
  if (table) return (panel, hour) => panel.data === undefined ? undefined : table(panel.data, hour);
  return undefined;
}

export function recordedPanelRows(product: string, raw: unknown): string[] {
  if (product === 'wargoals') {
    const r = raw as WargoalsReply;
    return [...r.goals.map(g => JSON.stringify(['goal', g.handle, g.actor, g.recipient, g.name, g.available, g.expires])),
      ...r.justifications.map(j => JSON.stringify(['justification', j.id, j.recipient, j.name, j.paused]))];
  }
  if (product === 'research-slots') {
    const r = raw as ResearchSlotsReply;
    return r.slots.map(s => JSON.stringify([s.index, s.state.token, s.state.entryFirstObservedHour]));
  }
  if (product === 'doctrine-mastery') {
    return (raw as DoctrineMasteryReply).folders.flatMap(f => [JSON.stringify([f.id, f.grand?.id]), ...f.completed.map(s => JSON.stringify([f.id, s.id])),
      ...f.tracks.flatMap(t => [JSON.stringify([f.id, t.ordinal, t.sub?.id]), ...t.rewards.map(r => JSON.stringify([f.id, t.ordinal, r.id])),
        ...t.rewards.slice(0, t.unlocked ?? 0).map(r => JSON.stringify(['unlocked', f.id, t.ordinal, r.id]))])]);
  }
  if (product === 'log-tail' || product === 'market-ledger-tail') {
    const r = raw as { total?: string | number; rows?: readonly { hour?: number; key?: string }[]; missing?: readonly { firstHour?: number; lastHour?: number; reason?: string }[] };
    return [
      JSON.stringify(['total', String(r.total ?? '')]),
      ...(r.rows ?? []).map(row => JSON.stringify(hasLogRowIdentity(row.key) ? [row.hour, row.key] : ['refused', MISSING_LOG_ROW_IDENTITY])),
      ...(r.missing ?? []).map(span => JSON.stringify(['missing', span.firstHour, span.lastHour, span.reason])),
    ];
  }
  return [];
}

const nextDay = (hour: number) => (Math.floor(hour / 24) + 1) * 24;

export const panelViews: Readonly<Record<string, PanelView>> = {
  'research-slots': recordView(validateResearchSlots, slotsStructure, (reply, source, hour) =>
    slotValues(reply, hour, slot => source.get(liveKey(String(slot), 'progress'))),
    { rowAttribute: 'data-research-slot', layers: '.research-lanes,.active-rows', meters: '.research-now-meter' },
    (reply, hour) => {
      let next = nextDay(hour);
      for (const { state } of reply.slots) if (state.details?.remainingDays != null) {
        next = Math.min(next, state.firstHour + nextDay(hour - state.firstHour));
      }
      return next;
    }),
};
