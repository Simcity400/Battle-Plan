import type { LiveValues } from '../../charts/live';
import type { PanelGeometry, PanelView } from './view';
import type { Structure } from '../views/fields';
import type { SpecialProjectsReply, DoctrineMasteryReply, ResearchSlotsReply, SpecialProjectDetails, WargoalsReply } from '../../generated/dashboard';
import { arsenalNotice, arsenalStructure, arsenalStructureRows, validateNuclearArsenal } from '../../widgets/nuclear-arsenal/nuclearArsenalData.ts';
import { liveKey } from '../../charts/live.ts';
import { slotsStructure, slotValues } from '../../widgets/research/researchSlots.ts';
import { validateResearchSlots } from '../../widgets/research/researchData.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { detailTexts, detailsStructure, effectsStructure, effectTexts, validateProjectEffects, validateProjectDetails } from '../../widgets/special-projects/projectDetailsData.ts';
import { projectStructure, currentKey, currentProjects, runningValues, validateSpecialProjectsReply } from '../../widgets/special-projects/specialProjectsData.ts';
import { occupiedStructure, validateOccupiedStates } from '../../widgets/occupied-states/occupiedStatesData.ts';
import { masteryStructure, trackKey, trackNumbers, trackTexts, trackRatio, validateDoctrineMastery } from '../../widgets/doctrine-mastery/doctrineMasteryData.ts';

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

const tableStructures: Readonly<Record<string, (raw: unknown) => string>> = {
  'occupied-states': raw => occupiedStructure(validateOccupiedStates(raw)),
};

export function panelStructure(product: string, raw: unknown, hour: number): string {
  return panelViews[product]?.structure?.(raw, hour) ?? tableStructures[product]?.(raw) ?? 'present';
}

export function readingStructure(product: string | undefined): Structure | undefined {
  const view = product === undefined ? undefined : panelViews[product]?.structure, table = product === undefined ? undefined : tableStructures[product];
  if (view) return (panel, hour) => panel.data === undefined ? undefined : view(panel.data, hour);
  if (table) return panel => panel.data === undefined ? undefined : table(panel.data);
  return undefined;
}

export function panelValues(product: string, raw: unknown, live: LiveValues<number> | null, hour: number, structure: string) {
  const view = panelViews[product];
  if (!view) return null;
  return view.values(raw, live, hour, view.structure?.(raw, hour) ?? structure);
}

export function recordedPanelRows(product: string, raw: unknown): string[] {
  if (product === 'nuclear-arsenal') return arsenalStructureRows(validateNuclearArsenal(raw));
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
  if (product === 'special-project-details') {
    const r = raw as SpecialProjectDetails;
    return [...(r.scientists ?? []).flatMap(s => ['scientist:' + s.character, ...(s.skills ?? []).map(k => JSON.stringify([s.character, k.specialization.token])),
      ...(s.traits ?? []).map(t => JSON.stringify([s.character, t.token]))]), ...(r.breakthroughs ?? []).map(b => 'breakthrough:' + b.specialization.token),
      ...(r.programs ?? []).flatMap(p => ['facility:' + p.id, ...(p.assignments ?? []).map(a => JSON.stringify([p.id, a.ordinal, a.reference])),
        ...(p.facility?.sources ?? []).map(s => JSON.stringify([p.id, s.kind, s.label?.token, s.assignment, s.scientistReference]))]),
      ...(r.projects ?? []).flatMap(p => [JSON.stringify([p.owner, p.id]), ...(p.variables ?? []).map(v => JSON.stringify([p.owner, p.id, v.name])),
        ...(p.pending?.options ?? []).map(o => JSON.stringify([p.owner, p.id, p.pending?.label.token, o.label.token])),
        ...(p.choices ?? []).map(c => JSON.stringify([p.owner, p.id, c.ordinal, c.reward.token, c.option.token]))])];
  }
  return [];
}

const nextDay = (hour: number) => (Math.floor(hour / 24) + 1) * 24;
const effectsLayer = '[data-effects-ink]';
const extendInline = '.project-details [data-paint]:last-child';

export const panelViews: Readonly<Record<string, PanelView>> = {
  'nuclear-arsenal': {
    nextChange: (_reply, hour) => nextDay(hour),
    structure: raw => arsenalStructure(validateNuclearArsenal(raw)),
    values(raw, _live, hour) {
      const reply = validateNuclearArsenal(raw);
      return new Map([['empty', arsenalNotice(reply, hour)]]);
    },
  },
  'special-project-effects': recordView(validateProjectEffects, effectsStructure, effectTexts,
    { layers: effectsLayer, owns: field => !!field.closest(effectsLayer), extendInline }),
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
  'special-project-details': recordView(validateProjectDetails, detailsStructure, reply => {
    const values = detailTexts(reply);
    if (reply.scientists) values.set('Scientists', String(reply.scientists.length));
    if (reply.programs) values.set('Facility assignments', String(reply.programs.length));
    if (reply.projects) values.set('Project rewards and variables', String(reply.projects.length));
    return values;
  }, { owns: field => !field.closest(effectsLayer), extendInline }),
  'doctrine-mastery': recordView(validateDoctrineMastery, masteryStructure, (reply, values) => {
    const fields = new Map<string, string>();
    for (const folder of reply.folders) for (const track of folder.tracks) {
      const key = trackKey(folder, track);
      const numbers = trackNumbers(values, key);
      const fraction = trackRatio(track, numbers.mastery);
      fields.set(key + ':bar', fraction === null ? '' : String(fraction));
      for (const [field, text] of Object.entries(trackTexts(numbers))) fields.set(key + ':' + field, text);
    }
    return fields;
  }, { rowAttribute: 'data-track', layers: '.doctrine-mastery-tracks' }),
  'special-projects': {
    structure: (raw, hour) => projectStructure(projectHistory(raw), hour),
    nextChange: (raw, hour) => projectChangeAfter(projectHistory(raw), hour),
    geometry: { rowAttribute: 'data-project', extraFields: '.special-project-progress > span', meters: '.special-project-progress',
      barVariable: { host: '[data-project]', name: '--project-progress' },
      knob: field => !!field.closest('.special-project-progress') && !field.closest('.is-paused,.is-unknown') },
    values(raw, _live, hour, shape) {
      const reply = projectHistory(raw);
      if (currentKey(reply, hour) !== shape && projectStructure(reply, hour) !== shape) return null;
      const cells = new Map<string, string>();
      const { running, completed, cancelled } = currentProjects(reply, hour);
      if (!running.length) cells.set('none', 'No projects in progress.');
      for (const card of [...completed, ...cancelled]) cells.set(card.project.id, card.info.name);
      for (const { project } of running) {
        const at = runningValues(project, hour);
        if (!at) continue;
        cells.set(project.id + ':bar', String(at.fill));
        cells.set(project.id + ':progress', at.progress);
        cells.set(project.id + ':iterations', at.iterations);
      }
      return cells;
    },
  },
};

const histories = new WeakMap<object, SpecialProjectsReply>();
function projectHistory(raw: unknown) {
  let reply = histories.get(raw as object);
  if (!reply) { reply = validateSpecialProjectsReply(raw); histories.set(raw as object, reply); }
  return reply;
}

const boundaries = new WeakMap<SpecialProjectsReply, number[]>();
export function projectChangeAfter(reply: SpecialProjectsReply, hour: number) {
  let edges = boundaries.get(reply);
  if (!edges) {
    const all = new Set<number>();
    for (const spans of [reply.available, ...reply.projects.map(p => p.runs)]) {
      for (const span of spans) { all.add(span.firstHour); all.add(span.lastHour + 1); }
    }
    edges = Array.from(all).sort((a, b) => a - b);
    boundaries.set(reply, edges);
  }
  let lo = 0, hi = edges.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (edges[mid]! <= hour) lo = mid + 1; else hi = mid;
  }
  return edges[lo] ?? Infinity;
}
