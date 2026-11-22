import type { LaneView, Painted, PaintInput } from '../lane.ts';
import { panelViews, readingStructure } from '../panels/views.ts';
import { ArsenalPicture, type ArsenalModel } from '../arsenal.ts';
import { validateNuclearArsenal } from '../../widgets/nuclear-arsenal/nuclearArsenalData.ts';
import { fieldsOf, liveValues, sameLive, type LiveMap } from './fields.ts';
import { tableView } from './table.ts';
import type { RaceItem } from '../../charts/race/race';

export type PanelTexts = Map<string, string>;

const liveOf = (map: LiveMap | null) => map && {
  get: (key: string) => { const value = map.get(key); return typeof value === 'number' ? value : undefined; },
  missing: (key: string) => { const value = map.get(key); return typeof value === 'string' ? value : 'Not recorded'; },
};

function panelTexts(): LaneView {
  return {
    next: (scene, hour) => {
      let soonest = Infinity;
      scene.subs.forEach((sub, at) => {
        const value = scene.pictures[at]!.parts[0]!.value, view = sub.product === undefined ? undefined : panelViews[sub.product];
        if (value.kind === 'panel' && value.refused === null && view?.nextChange) soonest = Math.min(soonest, view.nextChange(value.data, hour));
      });
      return soonest;
    },
    paint(input: PaintInput, state, content): Painted {
      const fields = { ...fieldsOf(input, input.hour, readingStructure, state, content) };
      input.subs.forEach((sub, at) => {
        const picture = input.pictures[at]!, value = picture.parts[0]!.value, view = sub.product === undefined ? undefined : panelViews[sub.product];
        if (value.kind !== 'panel' || value.refused !== null || !view) return;
        const structure = view.structure?.(value.data, input.hour) ?? '';
        let texts: PanelTexts | null = null;
        try { texts = view.values(value.data, liveOf(liveValues(picture)), input.hour, structure) as PanelTexts | null; } catch { texts = null; }
        const held = state.get(`texts:${sub.id}`) as { texts: PanelTexts | null; content: string } | undefined;
        const kept = held && sameLive(held.texts as LiveMap | null, texts as LiveMap | null) ? held : { texts, content: content() };
        state.set(`texts:${sub.id}`, kept);
        fields[`texts:${sub.id}`] = { value: texts, content: kept.content };
      });
      return { surfaces: [], fields, motion: false };
    },
  };
}

function race(): LaneView {
  return {
    next: (_scene, hour) => (Math.floor(hour / 24) + 1) * 24,
    paint(input: PaintInput, state, content): Painted {
      const fields = { ...fieldsOf(input, input.hour, readingStructure, state, content) };
      const surface = input.binding.surfaces[0], model = input.model as Omit<ArsenalModel, 'width' | 'height' | 'dpr'> | undefined;
      if (!surface || !model) return { surfaces: [], fields, motion: false };
      let held = state.get('race') as { canvas: OffscreenCanvas; picture: ArsenalPicture; content: string; hovered?: RaceItem; hit?: boolean } | undefined;
      if (!held) {
        const canvas = new OffscreenCanvas(Math.max(1, Math.round(surface.width * surface.dpr)), Math.max(1, Math.round(surface.height * surface.dpr)));
        const made: { canvas: OffscreenCanvas; picture: ArsenalPicture; content: string; hovered?: RaceItem; hit?: boolean } = { canvas, picture: undefined as unknown as ArsenalPicture, content: content() };
        made.picture = new ArsenalPicture(canvas, () => { made.content = content(); }, item => { made.hovered = item; made.hit = true; });
        state.set('race', held = made);
      }
      const at = input.subs.findIndex(sub => sub.id === model.follow);
      const value = at < 0 ? undefined : input.pictures[at]!.parts[0]!.value;
      const reply = value?.kind === 'panel' && value.refused === null ? validateNuclearArsenal(value.data) : model.reply;
      const full: ArsenalModel = { ...model, width: surface.width, height: surface.height, dpr: surface.dpr } as ArsenalModel;
      held.picture.draw(full, reply, input.hour);
      if (held.picture.changed) held.content = content();
      const point = state.get('pointer') as { point: [number, number] } | null | undefined;
      let hit: string | null | undefined;
      if (point !== undefined) {
        state.delete('pointer');
        held.picture.hover(point ? { x: point.point[0], y: point.point[1] } : undefined);
        hit = point && held.hovered ? `race:${held.hovered.key}` : null;
      }
      if (input.inspect?.startsWith('race:')) {
        const key = input.inspect.slice(5);
        fields.tooltip = { value: { key: input.inspect, text: held.picture.inspect({ key }) ?? null }, content: `tooltip:${input.inspect}:${held.content}` };
      }
      fields.items = { value: held.picture.inspect()?.items ?? [], content: `items:${held.content}` };
      return { surfaces: [{ source: held.canvas, content: held.content }], fields, motion: false, ...hit !== undefined ? { hit } : {} };
    },
  };
}

export function drawnViews(kind: string, wake: (widget: number) => void): LaneView | undefined {
  if (kind === 'panel') return panelTexts();
  if (kind === 'table') return tableView(readingStructure, wake);
  if (kind === 'race') return race();
  return undefined;
}
