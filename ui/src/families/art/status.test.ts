import assert from 'node:assert/strict';
import test from 'node:test';
import type { CountryFlagChoice } from '../../generated/dashboard';
import { createBoundedReadView } from '../boundedView.ts';
import type { FamilyTheme } from '../contract.ts';
import { layoutFamilyBars } from '../bars/layout.ts';
import { deriveBarsTooltip, type BarsModel } from '../bars/model.ts';
import { drawFamilyBars } from '../bars/painter.ts';
import { layoutCards } from '../card/layout.ts';
import { deriveCardTooltip, type CardModel } from '../card/model.ts';
import { drawCards } from '../card/painter.ts';
import { IdentityBroker, type IdentityTransport } from '../identity/broker.ts';
import { IdentityIndex } from '../identity/index.ts';
import { flagChoiceKey } from '../identity/types.ts';
import { LANE_THEME } from '../laneRuntime.fixture.ts';
import type { ArtHandle, ArtResult } from '../plan.ts';
import { buildColumnLayouts, layoutFamilyTable, measureFixedColumnWidths } from '../table/layout.ts';
import { ART_STATUS_ROW, deriveTableTooltip, resolveTableRows, type TableColumnSpec, type TableModel } from '../table/model.ts';
import { createTablePicture, drawFamilyTable, NO_GLIDE } from '../table/painter.ts';
import { OFFERED, tableSettingsOf, type TableTheme } from '../table/settings.ts';
import { knownValue } from '../value.ts';
import { artHoverText } from './adapters.ts';
import { LaneArtCache } from './laneCache.ts';
import { createLaneArtProvider, flagArtHandle } from './provider.ts';
import { artCalls, settleCall } from './transport.ts';
import { readyArt, refusedArt, failedArt, unavailableArt, type ArtResolution } from './types.ts';

async function answerHandles(...args: Parameters<typeof artCalls>): Promise<ReadonlyMap<string, ArtResolution>> {
  const settled = await Promise.all(artCalls(...args).map(call => settleCall(call, args[3])));
  return new Map(settled.flatMap(map => [...map]));
}

const THEME: FamilyTheme = { ...LANE_THEME, hatch: '#aa0001', muted: '#aa0002' };
const BITMAP = { width: 8, height: 8, close() {} };

type Status = 'ready' | 'pending' | 'unavailable' | 'refused' | 'failed' | 'confirmedMissing';
const STATUSES: readonly Status[] = ['ready', 'pending', 'unavailable', 'refused', 'failed', 'confirmedMissing'];
const FORM: Readonly<Record<Status, string>> = {
  ready: 'image', pending: 'empty', unavailable: 'unrecorded', refused: 'refused', failed: 'failed', confirmedMissing: 'missing',
};

function replyOf(status: Exclude<Status, 'pending'>, id: string) {
  switch (status) {
    case 'ready': return readyArt(`art://${id}`);
    case 'unavailable': return unavailableArt(`${id} is not recorded at this hour`);
    case 'refused': return refusedArt(`${id} needs native sprite rendering`, 'sprite-render');
    case 'failed': return failedArt(`${id} could not be read`, true);
    case 'confirmedMissing': return { status: 'confirmedMissing' as const, reason: `no layer holds ${id}` };
  }
}

interface Op { readonly op: string; readonly args: readonly unknown[]; readonly fillStyle: unknown; readonly strokeStyle: unknown }

function recorder(): { readonly ctx: CanvasRenderingContext2D; readonly ops: Op[] } {
  const ops: Op[] = [];
  const state: Record<string | symbol, unknown> = { fillStyle: '', strokeStyle: '', font: '400 12px Inter', globalAlpha: 1, lineWidth: 1, textAlign: 'left', textBaseline: 'middle' };
  const stack: Record<string | symbol, unknown>[] = [];
  const ctx = new Proxy(state, {
    get(target, prop) {
      if (prop in target) return target[prop];
      if (prop === 'save') return () => stack.push({ ...target });
      if (prop === 'restore') return () => { const prev = stack.pop(); if (prev) Object.assign(target, prev); };
      if (prop === 'measureText') return (text: string) => ({ width: text.length * 6, actualBoundingBoxAscent: 8, actualBoundingBoxDescent: 2 });
      if (prop === 'createLinearGradient' || prop === 'createPattern') return () => ({ addColorStop() {}, setTransform() {} });
      if (prop === 'getTransform') return () => ({ a: 1, d: 1, e: 0, f: 0 });
      return (...args: unknown[]) => { ops.push({ op: String(prop), args, fillStyle: target.fillStyle, strokeStyle: target.strokeStyle }); };
    },
    set(target, prop, value) { target[prop] = value; return true; },
  });
  return { ctx: ctx as unknown as CanvasRenderingContext2D, ops };
}

function slotForm(ops: readonly Op[], label: string): string {
  if (ops.some(op => op.op === 'drawImage' && op.args[0] === BITMAP)) return 'image';
  if (ops.some(op => op.op === 'arc' && op.fillStyle === THEME.muted)) return 'failed';
  if (ops.some(op => op.op === 'lineTo' && op.strokeStyle === THEME.muted)) return 'refused';
  const hatched = ops.some(op => op.op === 'lineTo' && op.strokeStyle === THEME.hatch);
  if (hatched) return ops.some(op => op.op === 'fillText' && op.args[0] === label) ? 'missing' : 'unrecorded';
  return 'empty';
}

const flush = async () => { for (let i = 0; i < 50; i++) await Promise.resolve(); };
const TRACK = { source: { id: 's', revision: 'r' }, branch: { id: '1' } } as never;

async function artAtLane(): Promise<{ readonly resolve: (handle: ArtHandle) => ArtResult; readonly handles: Readonly<Record<string, Readonly<Record<Status, ArtHandle>>>>; readonly cache: LaneArtCache }> {
  const handles: Record<string, Record<Status, ArtHandle>> = { portrait: {} as Record<Status, ArtHandle>, sprite: {} as Record<Status, ArtHandle>, textIcon: {} as Record<Status, ArtHandle>, flag: {} as Record<Status, ArtHandle> };
  for (const status of STATUSES) {
    handles.portrait![status] = { kind: 'portrait', leader: status, hour: 300 };
    handles.sprite![status] = { kind: 'sprite', key: `GFX_${status}`, frame: 0 };
    handles.textIcon![status] = { kind: 'sprite', key: `GFX_text_${status}`, frame: 0, text: true };
  }
  const asked = STATUSES.filter(status => status !== 'pending').flatMap(status => [handles.portrait![status]!, handles.sprite![status]!, handles.textIcon![status]!]);
  const answered = await answerHandles(async request => request.product === 'unit-leader-portraits'
    ? { portraits: STATUSES.filter(status => status !== 'pending').map(status => {
        const resolution = replyOf(status as Exclude<Status, 'pending'>, status);
        return { reference: status, image: resolution.status === 'ready' ? resolution.url : null, missing: resolution.status === 'ready' ? null : resolution.reason, resolution };
      }), warnings: [] }
    : { items: (request.params as { sprites: { key: string; text?: boolean }[] }).sprites.map(sprite => {
        const status = sprite.key.replace(/^GFX_(text_)?/, '') as Exclude<Status, 'pending'>;
        const resolution = replyOf(status, sprite.key);
        return { key: sprite.key, frame: 0, ...(sprite.text ? { text: true } : {}), icon: resolution.status === 'ready' ? resolution.url : null, resolution };
      }), warnings: [] },
  TRACK, asked, new AbortController().signal);

  const choice = (status: Status): CountryFlagChoice => ({ keys: [`QQ_${status}`, `QQ_${status}`, `QQ_${status}`], reasons: [null, null, null] });
  const transport = {
    loadFlags: async (_scope: unknown, _size: unknown, choices: readonly CountryFlagChoice[]) => ({
      warnings: [],
      flags: choices.flatMap((asked, flag) => asked.keys[0] === 'QQ_ready' ? [{ flag, asset: 'gfx/flags/QQ_ready.tga', key: 'QQ_ready#1', width: 8, height: 8, png: 'art://flag' }] : []),
      unavailable: choices.flatMap((asked, flag) => {
        const status = asked.keys[0]!.slice(3) as Exclude<Status, 'pending'>;
        if (status === 'ready') return [];
        const resolution = replyOf(status, `flag ${status}`);
        return [{ flag, reason: 'reason' in resolution ? resolution.reason : '', resolution }];
      }),
    }),
  } as unknown as IdentityTransport;
  const broker = new IdentityBroker({ transport, clock: { now: () => 0, after: () => () => {} } });
  const scope = { sourceId: 's', revision: 'r', branchId: '1', content: 1 };
  broker.requestFlagChoices(scope, 'small', STATUSES.filter(status => status !== 'pending').map(choice));
  await flush();
  for (const status of STATUSES) handles.flag![status] = flagArtHandle(flagChoiceKey(choice(status)), 'small');
  const index = IdentityIndex.fromSnapshot(broker.snapshotOf(scope));

  const cache = new LaneArtCache({ fetchBitmap: async () => BITMAP as unknown as ImageBitmap });
  cache.requestVisible(['art://ready', 'art://GFX_ready', 'art://GFX_text_ready', 'art://flag']);
  await flush();
  const provider = createLaneArtProvider(cache, { index, resolutions: Object.fromEntries(answered) });
  return { resolve: handle => provider.resolve(handle), handles, cache };
}

test('one art status travels from the product reply to the painted slot and the tooltip: flag, portrait, sprite and text icon, through a table cell, a bars row and a card', async () => {
  const { resolve, handles, cache } = await artAtLane();
  for (const [kind, byStatus] of Object.entries(handles)) {
    const flag = kind === 'flag';
    const label = flag ? 'QQA' : 'HT';
    for (const status of STATUSES) {
      const at = `${kind} ${status}`;
      const readView = createBoundedReadView({ hour: 300, art: { demandOwner: 'view', resolve } });
      const art = readView.art(byStatus[status]!);
      assert.equal(art.status, status, `${at}: the status the product gave reaches the read view`);
      const hover = artHoverText(art);

      const columns: TableColumnSpec[] = [
        flag
          ? { key: 'name', label: 'Country', priority: 0, as: 'flagName', offered: OFFERED.country, text: true }
          : { key: 'name', label: 'Leader', priority: 0, as: 'plain', offered: OFFERED.text, cellKind: 'kind' },
        { key: 'v', label: 'Value', priority: 1, as: 'plain', offered: OFFERED.number, numeric: true },
      ];
      const tableSettings = tableSettingsOf(undefined, columns, { density: 'comfortable', sort: { column: 'v', direction: 'descending' } });
      const model: TableModel = {
        columns,
        hour: 300,
        rows: [{ key: 'r', name: 'Sample', cells: {
          name: flag
            ? { value: knownValue('Sample Country'), iconSlot: true, country: 'QQA', art }
            : { value: knownValue('Heavy Tank'), iconSlot: true, artShape: 'square', artName: 'Heavy Tank', art },
          v: { value: knownValue(5) },
        } }],
      };
      const rows = resolveTableRows(model, tableSettings);
      const table = recorder();
      const widths = measureFixedColumnWidths(buildColumnLayouts(columns, tableSettings), tableSettings, THEME as TableTheme, text => text.length * 6);
      const layout = layoutFamilyTable(buildColumnLayouts(columns, tableSettings, widths), 480, { rankShown: false, rankWidth: 0 });
      drawFamilyTable(table.ctx, { layout, rows, settings: tableSettings, hour: 300, width: 480, height: 200, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: THEME as TableTheme, now: 0, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, artCache: cache, dpr: 1 }, createTablePicture());
      assert.equal(slotForm(table.ops, label), FORM[status], `${at}: table cell slot`);
      const tip = deriveTableTooltip(rows, 'r', columns)!;
      if (flag) {
        assert.deepEqual(tip.countryFlag, art.status === 'ready' ? { flag: art.url } : art.status === 'pending' ? { flagPending: true } : { flagReason: hover }, `${at}: table tooltip flag`);
      } else {
        const shown = tip.rows.find(row => row.lookupKey === ART_STATUS_ROW)?.value ?? null;
        assert.equal(shown, hover, `${at}: table tooltip`);
      }

      const bars: BarsModel = {
        form: 'ordered', hour: 300, scaleMax: 10, showAxis: false,
        rows: [{ kind: 'data', key: 'r', label: 'Sample Country', media: flag ? { kind: 'flag', art, tag: 'QQA' } : { kind: 'art', art, resolvedName: 'Heavy Tank' }, color: '#888888', value: { value: knownValue(5), format: 'integer' }, numeric: 5 }],
      };
      const barsDraw = recorder();
      drawFamilyBars(barsDraw.ctx, { model: bars, layout: layoutFamilyBars(bars, 540, 200, THEME), theme: THEME, scrollTop: 0, now: 0, glideDuration: 0, artCache: cache });
      assert.equal(slotForm(barsDraw.ops, label), FORM[status], `${at}: bars row slot`);
      const barsTip = deriveBarsTooltip(bars, 'r')?.items?.find(item => item.label === (flag ? 'Flag' : 'Art'))?.missingReason ?? null;
      assert.equal(barsTip, hover, `${at}: bars tooltip`);

      const card: CardModel = {
        hour: 300, rowHeight: 40,
        columns: flag ? { flag: true } : { art: { width: 32, height: 32 } },
        rows: [{ kind: 'item', key: 'r', title: knownValue('Sample'), ...(flag ? { flag: { kind: 'flag', art, tag: 'QQA' } } : { art: { kind: 'art', art, name: 'Heavy Tank' } }) }],
      };
      const cardDraw = recorder();
      drawCards(cardDraw.ctx, { model: card, layout: layoutCards(card, 600, 200), theme: THEME, scrollTop: 0, artCache: cache });
      assert.equal(slotForm(cardDraw.ops, label), FORM[status], `${at}: card slot`);
      const cardTip = deriveCardTooltip(card, 'r')?.items?.find(item => item.label === (flag ? 'Flag' : 'Art'))?.missingReason ?? null;
      assert.equal(cardTip, hover, `${at}: card tooltip`);
    }
  }
});
