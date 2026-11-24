import '../../families/table/registerAll.ts';
import { familyBarsLaneView } from '../../families/bars/laneView.ts';
import type { LaneView, Painted, PaintInput, Scene } from '../lane.ts';
import { panelViews, readingStructure } from '../panels/views.ts';
import { ArsenalPicture, arsenalNext, type ArsenalModel } from '../arsenal.ts';
import { validateNuclearArsenal } from '../../widgets/nuclear-arsenal/nuclearArsenalData.ts';
import { fieldsOf, liveValues, sameLive, type LiveMap } from './fields.ts';
import { tableView } from './table.ts';

export type PanelTexts = Map<string, string>;

const liveOf = (map: LiveMap | null) => map && {
  get: (key: string) => { const value = map.get(key); return typeof value === 'number' ? value : typeof value === 'bigint' ? Number(value) : undefined; },
  raw: (key: string) => { const value = map.get(key); return typeof value === 'bigint' ? value : typeof value === 'number' && Number.isSafeInteger(value) ? BigInt(value) : undefined; },
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

type RaceModel = Omit<ArsenalModel, 'width' | 'height' | 'dpr'>;
interface RaceHeld { canvas: OffscreenCanvas; picture: ArsenalPicture; content: string; items?: { text: string; content: string }; tip?: { text: string; content: string } }

function replyOf(scene: Scene, model: RaceModel) {
  const at = scene.subs.findIndex(sub => sub.id === model.follow);
  const value = at < 0 ? undefined : scene.pictures[at]!.parts[0]!.value;
  return value?.kind === 'panel' && value.refused === null ? validateNuclearArsenal(value.data) : model.reply;
}

function race(wake: (widget: number) => void): LaneView {
  return {
    next: (scene, hour) => { const model = scene.model as RaceModel | undefined; return model ? arsenalNext(model, replyOf(scene, model), hour) : Infinity; },
    paint(input: PaintInput, state, content): Painted {
      const fields = { ...fieldsOf(input, input.hour, readingStructure, state, content) };
      const surface = input.binding.surfaces[0], model = input.model as RaceModel | undefined;
      if (!surface || !model) return { surfaces: [], fields, motion: false };
      let held = state.get('race') as RaceHeld | undefined;
      if (!held) {
        const canvas = new OffscreenCanvas(Math.max(1, Math.round(surface.width * surface.dpr)), Math.max(1, Math.round(surface.height * surface.dpr)));
        const made: RaceHeld = { canvas, picture: undefined as unknown as ArsenalPicture, content: content() };
        made.picture = new ArsenalPicture(canvas, () => wake(input.widget));
        state.set('race', held = made);
      }
      const full: ArsenalModel = { ...model, width: surface.width, height: surface.height, dpr: surface.dpr } as ArsenalModel;
      held.picture.draw(full, replyOf(input, model), input.hour);
      if (held.picture.changed) held.content = content();
      if (input.inspect?.startsWith('race:')) {
        const text = held.picture.inspect({ key: input.inspect.slice(5) }) ?? null, said = JSON.stringify(text);
        held.tip = held.tip?.text === said ? held.tip : { text: said, content: content() };
        fields.tooltip = { value: { key: input.inspect, text }, content: `tooltip:${input.inspect}:${held.tip.content}` };
      }
      const items = held.picture.inspect()?.items ?? [], listed = JSON.stringify(items);
      held.items = held.items?.text === listed ? held.items : { text: listed, content: content() };
      fields.items = { value: items, content: `items:${held.items.content}` };
      fields.audit = { value: held.picture.rows() ?? [], content: `audit:${held.items.content}:${held.content}` };
      const at = held.picture.hitTest();
      return { surfaces: [{ source: held.canvas, content: held.content }], fields, motion: false, hit: point => { const item = at({ x: point[0], y: point[1] }); return item ? `race:${item.key}` : null; } };
    },
  };
}

export function drawnViews(kind: string, wake: (widget: number) => void): LaneView | undefined {
  if (kind === 'panel') return panelTexts();
  if (kind === 'table') return tableView(readingStructure, wake);
  if (kind === 'bars') return familyBarsLaneView(wake);
  if (kind === 'race') return race(wake);
  return undefined;
}
