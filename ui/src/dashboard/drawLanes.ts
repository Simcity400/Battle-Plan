import type { FromWorker, ToWorker } from '../draw/kit';
import type { DrawSample, Present } from '../draw/notifications';
import { ROW_HEIGHT } from '../draw/tables/logic.ts';

export interface DrawLane { postMessage(message: ToWorker, transfer: Transferable[]): void }

const SURFACE = new Set<ToWorker['type']>(['surface', 'table', 'dateFace', 'race', 'pointer', 'tablePointer', 'tableScroll', 'tableRead',
  'racePointer', 'inspectRace', 'detach', 'closeRace']);
const PAGE_WIDE = new Set<FromWorker['type']>(['displayRate', 'structure', 'followFailure']);

export type SurfaceNotice = Extract<FromWorker, { id: number }> | { type: 'present'; surface: Present } | { type: 'failed'; page: number; reason: string };

function painted(message: ToWorker): number {
  if (message.type === 'surface') return Math.max(1, message.model.cells?.length ?? 0);
  if (message.type === 'table') {
    const { columns, height, settings, rows: built } = message.model;
    const room = Math.ceil((height ?? 0) / (ROW_HEIGHT[settings?.density] ?? ROW_HEIGHT.comfortable));
    const rows = built?.length ? Math.min(room, built.length) : room;
    return Math.max(1, (columns?.filter(column => column.shown).length ?? 0) * rows);
  }
  if (message.type === 'race') return Math.max(1, (message.model.slots ?? 0) * 3);
  return 1;
}

export class DrawLanes {
  private readonly owner = new Map<number, { lane: number; weight: number; table: boolean }>();
  private readonly surfaces = new Map<number, (notice: SurfaceNotice) => void>();
  private readonly lanes: readonly DrawLane[];
  private readonly emit: (message: FromWorker) => void;
  private readonly load: number[];
  private readonly tables: number[];
  private audit?: { waiting: number; records: DrawSample[]; done: boolean; failed?: Extract<FromWorker, { type: 'failed' }> };

  constructor(lanes: readonly DrawLane[], emit: (message: FromWorker) => void) {
    this.lanes = lanes; this.emit = emit; this.load = lanes.map(() => 0); this.tables = lanes.map(() => 0);
  }

  get count() { return this.lanes.length; }

  post(message: ToWorker, transfer: Transferable[] = []) {
    if (SURFACE.has(message.type) && 'id' in message) {
      let held = this.owner.get(message.id);
      if (held === undefined) {
        if (message.type === 'detach' || message.type === 'closeRace') return;
        if (message.type !== 'surface' && message.type !== 'table' && message.type !== 'dateFace' && message.type !== 'race') {
          this.lanes[0]!.postMessage(message, transfer); return;
        }
        const table = message.type === 'table';
        let lane = 0;
        for (let at = 1; at < this.lanes.length; at++) {
          const fewer = table && this.tables[at]! !== this.tables[lane]! ? this.tables[at]! < this.tables[lane]! : this.load[at]! < this.load[lane]!;
          if (fewer) lane = at;
        }
        held = { lane, weight: 0, table };
        this.owner.set(message.id, held); if (table) this.tables[lane]!++;
      }
      if (message.type === 'surface' || message.type === 'table' || message.type === 'race' || message.type === 'dateFace') {
        const weight = painted(message);
        this.load[held.lane]! += weight - held.weight; held.weight = weight;
      }
      if (message.type === 'detach' || message.type === 'closeRace') {
        this.owner.delete(message.id); this.load[held.lane]! -= held.weight; if (held.table) this.tables[held.lane]!--;
      }
      this.lanes[held.lane]!.postMessage(message, transfer);
      return;
    }
    if (message.type === 'inspect') { this.lanes[0]!.postMessage(message, transfer); return; }
    if (message.type === 'audit') this.audit = { waiting: this.lanes.length, records: [], done: true };
    for (const lane of this.lanes) lane.postMessage(message, transfer);
  }

  postTo(lane: number, message: ToWorker, transfer: Transferable[] = []) { this.lanes[lane]!.postMessage(message, transfer); }

  listen(id: number, listener: (notice: SurfaceNotice) => void): () => void {
    this.surfaces.set(id, listener);
    return () => { if (this.surfaces.get(id) === listener) this.surfaces.delete(id); };
  }

  receive(lane: number, message: FromWorker) {
    if (message.type === 'audit') {
      const audit = this.audit;
      if (!audit) { this.emit(message); return; }
      audit.records.push(...message.records); audit.done &&= message.done;
      if (--audit.waiting > 0) return;
      this.audit = undefined;
      this.emit(audit.failed ?? { type: 'audit', records: audit.records, done: audit.done });
      return;
    }
    if (message.type === 'present') { for (const surface of message.surfaces) this.surfaces.get(surface.id)?.({ type: 'present', surface }); return; }
    if (message.type === 'failed' && message.page !== -1) { this.surfaces.get(message.page)?.(message); if (this.audit) this.audit.failed ??= message; return; }
    if ('id' in message) { this.surfaces.get(message.id)?.(message); return; }
    if (message.type === 'structure') this.lanes[lane]!.postMessage({ type: 'structureTaken' }, []);
    if (lane !== 0 && PAGE_WIDE.has(message.type)) return;
    this.emit(message);
  }
}
