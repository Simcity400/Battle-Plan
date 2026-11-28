import { datedPanel } from '../dashboard/follow/messages.fixture.ts';
import { Lane } from '../draw/lane.ts';
import { DEFAULT_SEEDS, deriveColors } from '../shell/appearance/themes.ts';
import { defaultArtFrame, readyArt } from './art/types.ts';
import { barsLane, type FamilyBarsBinding } from './bars/laneView.ts';
import type { BarsModel } from './bars/model.ts';
import type { FamilyTheme } from './contract.ts';
import { createLaneView } from './laneRuntime.ts';
import { registerFamilyProjection } from './registry.ts';
import { knownValue } from './value.ts';

const colours = deriveColors(DEFAULT_SEEDS.dark);
const theme: FamilyTheme = {
  surface: colours.surface, raised: colours.raised, dark: true, text: colours.text, muted: colours.muted,
  line: colours.border, lineSoft: colours.border, accent: colours.accent, blue: colours.accent, red: colours.error,
  shortage: colours.error, amber: colours.warning, green: colours.positive, grey: colours.muted, hatch: colours.muted,
  hatchEdge: colours.border, font: 'Segoe UI, sans-serif', fontSize: { small: 11, body: 12 }, lineWidth: 1, hatchSpacing: 6,
};

function rig(kind: string, dpr: number) {
  let now = 1000;
  let serial = 0;
  const queued: (() => void)[] = [];
  const view = createLaneView(barsLane, () => {});
  const lane = new Lane<ImageBitmap>({
    lane: 0, lanes: 1, views: () => view, crop: async () => [], present() {}, release() {},
    fail: reason => { throw new Error(reason); }, later: draw => queued.push(draw), clock: () => now,
  });
  const canvas = new OffscreenCanvas(1, 1);
  lane.setLaneSurface(canvas, true);
  lane.follow(1, 'panel', 'p');
  lane.bindRoute(1, 1, 9, 1, 0);
  lane.mount(1, [1], 'bars', {
    family: 'bars', kind, view: 'main', options: {}, sources: { main: 1 }, scrollTop: 0, width: 481, height: 235, theme,
  } satisfies FamilyBarsBinding);
  lane.bind(1, { generation: 1, pin: null, offset: 0, surfaces: [{ width: 481, height: 235, dpr }] });
  lane.setLaneSlots({
    width: 490, height: 250, dpr, scrollLeft: 0, scrollTop: 0, clipPathFallback: false, surfaceColor: theme.surface, cornerRadius: 9,
    slots: [{ widget: 1, x: 3, y: 4, w: 481, h: 235, bx: 3, by: 2, bw: 481, bh: 238, visible: true }],
  });
  lane.receive('panels', datedPanel(1, 100, 30));
  lane.receive('panels', datedPanel(1, 101, 150));
  const request = async (now: number) => {
    lane.frame({ type: 'frame', serial: ++serial, now, target: now, exact: true, seek: 1, speed: 0, installed: [], refused: [], inspect: [] });
    for (let i = 0; i < 12; i++) await Promise.resolve();
  };
  const clock = (playing: boolean) => lane.setClock({
    playing, speed: playing ? 1 : 0, anchorPosition: 100, anchorTimeMs: performance.timeOrigin + now, seek: 1, exactHour: 100,
  });
  const step = () => {
    now += 200;
    for (const draw of queued.splice(0)) draw();
  };
  return { request, clock, step, pixels: () => canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data };
}

function changedBytes(a: Uint8ClampedArray, b: Uint8ClampedArray): number {
  let changed = 0;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) changed++;
  return changed;
}

async function checkSettledPixels() {
  const sprite = new OffscreenCanvas(37, 21);
  const brush = sprite.getContext('2d')!;
  brush.fillStyle = theme.red;
  brush.fillRect(0, 0, 37, 21);
  brush.fillStyle = theme.text;
  brush.fillRect(9, 0, 23, 11);
  const bitmap = sprite.transferToImageBitmap();
  const flag = { ...readyArt('fixture-flag', { frame: defaultArtFrame(37, 21) }), bitmap };
  const kind = 'ordered-bars-pixel-settle';
  registerFamilyProjection({
    family: 'bars', kind,
    project(view): BarsModel {
      const value = Number(view.panel('main'));
      return {
        form: 'ordered', hour: view.hour, scaleMax: 200, showAxis: true,
        rows: [{
          kind: 'data', key: 'fixture', label: knownValue('Fixture country'), numeric: value, value: { value: knownValue(value) },
          media: { kind: 'flag', art: flag, tag: 'FIX' },
        }],
      };
    },
  }, 'main');
  const results: { dpr: number; moving: number; paused: number }[] = [];
  for (const dpr of [1, 1.25, 1.5, 2]) {
    const played = rig(kind, dpr);
    await played.request(100);
    const initial = played.pixels();
    played.clock(true);
    played.step();
    played.step();
    const moving = changedBytes(initial, played.pixels());
    played.clock(false);
    await played.request(100);
    played.step();
    const cold = rig(kind, dpr);
    await cold.request(100);
    results.push({ dpr, moving, paused: changedBytes(cold.pixels(), played.pixels()) });
  }
  bitmap.close();
  return results;
}

(window as unknown as { checkSettledPixels: typeof checkSettledPixels }).checkSettledPixels = checkSettledPixels;
