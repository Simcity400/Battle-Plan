import type { FromWorker, ToWorker } from '../draw/kit';
import type { DrawSample } from '../draw/notifications';

export interface DrawLane { postMessage(message: ToWorker, transfer: Transferable[]): void }

const SURFACE = new Set<ToWorker['type']>(['surface', 'table', 'dateFace', 'race', 'pointer', 'tablePointer', 'tableScroll', 'tableRead',
  'racePointer', 'inspectRace', 'detach', 'closeRace']);
const PAGE_WIDE = new Set<FromWorker['type']>(['displayRate', 'structure', 'structureLead', 'followFailure']);

export class DrawLanes {
  private readonly owner = new Map<number, number>();
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
      let lane = this.owner.get(message.id);
      if (lane === undefined) {
        if (message.type === 'detach' || message.type === 'closeRace') return;
        if (message.type !== 'surface' && message.type !== 'table' && message.type !== 'dateFace' && message.type !== 'race') {
          this.lanes[0]!.postMessage(message, transfer); return;
        }
        lane = this.load.indexOf(Math.min(...this.load));
        this.owner.set(message.id, lane); this.load[lane]!++;
      }
      if (message.type === 'detach' || message.type === 'closeRace') { this.owner.delete(message.id); this.load[lane]!--; }
      this.lanes[lane]!.postMessage(message, transfer);
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
