import { familyLaneView } from '../../widgets/familyManifest.ts';
import type { LaneView, Painted, PaintInput } from '../lane.ts';
import { panelViews, readingStructure } from '../panels/views.ts';
import { fieldsOf, liveValues, sameLive, type LiveMap } from './fields.ts';

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

export function drawnViews(kind: string, wake: (widget: number) => void): LaneView | undefined {
  const family = familyLaneView(kind, wake);
  if (family) return family;
  if (kind === 'panel') return panelTexts();
  return undefined;
}
