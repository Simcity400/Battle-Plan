import type { FromWorker, ToWorker } from '../draw/kit';
import type { DrawSample } from '../draw/notifications';
import { ROW_HEIGHT } from '../draw/tables/logic.ts';

export interface DrawLane { postMessage(message: ToWorker, transfer: Transferable[]): void }

const SURFACE = new Set<ToWorker['type']>(['surface', 'table', 'dateFace', 'race', 'pointer', 'tablePointer', 'tableScroll', 'tableRead',
  'racePointer', 'inspectRace', 'detach', 'closeRace']);
const PAGE_WIDE = new Set<FromWorker['type']>(['displayRate', 'structure', 'structureLead', 'followFailure']);

function painted(message: ToWorker): number {
  if (message.type === 'surface') return Math.max(1, message.model.cells?.length ?? 0);
  if (message.type === 'table') {
    const { columns, height, settings } = message.model;
    const rows = Math.ceil((height ?? 0) / (ROW_HEIGHT[settings?.density] ?? ROW_HEIGHT.comfortable));
    return Math.max(1, (columns?.filter(column => column.shown).length ?? 0) * rows);
  }
  if (message.type === 'race') return Math.max(1, (message.model.slots ?? 0) * 3);
  return 1;
}

export class DrawLanes {
  private readonly owner = new Map<number, { lane: number; weight: number }>();
  private readonly lanes: readonly DrawLane[];
  private readonly emit: (message: FromWorker) => void;
  private readonly load: number[];
  private audit?: { waiting: number; records: DrawSample[]; done: boolean };

  constructor(lanes: readonly DrawLane[], emit: (message: FromWorker) => void) {
    this.lanes = lanes; this.emit = emit; this.load = lanes.map(() => 0);
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
        held = { lane: this.load.indexOf(Math.min(...this.load)), weight: 0 };
        this.owner.set(message.id, held);
      }
      if (message.type === 'surface' || message.type === 'table' || message.type === 'race' || message.type === 'dateFace') {
        const weight = painted(message);
        this.load[held.lane]! += weight - held.weight; held.weight = weight;
      }
      if (message.type === 'detach' || message.type === 'closeRace') { this.owner.delete(message.id); this.load[held.lane]! -= held.weight; }
      this.lanes[held.lane]!.postMessage(message, transfer);
      return;
    }
    if (message.type === 'inspect') { this.lanes[0]!.postMessage(message, transfer); return; }
    if (message.type === 'audit') this.audit = { waiting: this.lanes.length, records: [], done: true };
    for (const lane of this.lanes) lane.postMessage(message, transfer);
  }

  postTo(lane: number, message: ToWorker, transfer: Transferable[] = []) { this.lanes[lane]!.postMessage(message, transfer); }

  receive(lane: number, message: FromWorker) {
    if (message.type === 'audit') {
      const audit = this.audit;
      if (!audit) { this.emit(message); return; }
      audit.records.push(...message.records); audit.done &&= message.done;
      if (--audit.waiting > 0) return;
      this.audit = undefined;
      this.emit({ type: 'audit', records: audit.records, done: audit.done });
      return;
    }
    if (lane !== 0 && PAGE_WIDE.has(message.type)) return;
    this.emit(message);
  }
}
