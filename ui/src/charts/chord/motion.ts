import { approach } from '../easing.ts';
import { checkChord } from './check.ts';
import type { ChordData, ChordFlow, ChordNode } from './layout.ts';

/** Stable insertion order across updates. Removed items keep their position until
 * zero, and their identity keeps its position if it returns. No data is interpolated. */
export class ChordMotion {
  private nodes = new Map<string, { node: ChordNode; target: number; presence: number }>();
  private flows = new Map<string, { flow: ChordFlow; target: number; value: number }>();
  private readonly nodeOrder = new Map<string, number>();
  private readonly flowOrder = new Map<string, { order: number; source: string; target: string }>();
  private readonly nodePresence = new Map<string, number>();
  readonly presence: ReadonlyMap<string, number> = this.nodePresence;
  private ring = 0;
  private ringTarget = 0;
  private snapNext = false;
  private snapOnAdvance = false;
  private settled: ReturnType<ChordMotion['advance']> | null = null;

  set(data: ChordData): void {
    const total = checkChord(data);
    for (const flow of data.flows) {
      const identity = this.flowOrder.get(flow.id);
      if (identity && (identity.source !== flow.source || identity.target !== flow.target)) throw new Error(`Chord flow changed endpoints: ${flow.id}`);
    }
    this.settled = null;
    this.snapOnAdvance ||= this.snapNext;
    this.snapNext = false;
    this.ringTarget = Number(total > 0);
    for (const state of this.nodes.values()) state.target = 0;
    for (const state of this.flows.values()) state.target = 0;
    for (const node of data.nodes) {
      if (!this.nodeOrder.has(node.id)) this.nodeOrder.set(node.id, this.nodeOrder.size);
      const state = this.nodes.get(node.id);
      if (state) state.node = node;
      else this.nodes.set(node.id, { node, target: 0, presence: 0 });
    }
    for (const flow of data.flows) {
      if (!this.flowOrder.has(flow.id)) this.flowOrder.set(flow.id, { order: this.flowOrder.size, source: flow.source, target: flow.target });
      const source = this.nodes.get(flow.source)!, target = this.nodes.get(flow.target)!;
      const state = this.flows.get(flow.id);
      if (state) { state.flow = flow; state.target = flow.value; }
      else this.flows.set(flow.id, { flow, target: flow.value, value: 0 });
      if (flow.value > 0) source.target = target.target = 1;
    }
    // Returning identities recover their old position. Sort only at a data
    // change; a frame walks active and departing shapes, never the history.
    this.nodes = new Map([...this.nodes].sort(([a], [b]) => this.nodeOrder.get(a)! - this.nodeOrder.get(b)!));
    this.flows = new Map([...this.flows].sort(([a], [b]) => this.flowOrder.get(a)!.order - this.flowOrder.get(b)!.order));
  }

  /** A recording gap or a change of source resets presentation immediately. */
  clear(): void {
    this.settled = null;
    this.nodes.clear(); this.flows.clear(); this.nodeOrder.clear(); this.flowOrder.clear();
    this.nodePresence.clear(); this.ring = this.ringTarget = 0;
    this.snapNext = true; this.snapOnAdvance = false;
  }

  advance(dt: number, tau: number): Readonly<{ data: ChordData; moving: boolean; ringPresence: number }> {
    if (this.settled) return this.settled;
    if (this.snapOnAdvance) { tau = 0; this.snapOnAdvance = false; }
    this.ring = approach(this.ring, this.ringTarget, dt, tau);
    let moving = this.ring !== this.ringTarget;
    const nodes: ChordNode[] = [], flows: ChordFlow[] = [];
    const endpoints = new Set<string>();
    for (const [id, state] of this.flows) {
      state.value = approach(state.value, state.target, dt, tau);
      moving ||= state.value !== state.target;
      if (state.value > 0) {
        flows.push({ ...state.flow, value: state.value });
        endpoints.add(state.flow.source); endpoints.add(state.flow.target);
      } else if (state.target === 0) this.flows.delete(id);
    }
    for (const [id, state] of this.nodes) {
      state.presence = approach(state.presence, state.target, dt, tau);
      moving ||= state.presence !== state.target;
      if (state.presence > 0 || state.target > 0 || endpoints.has(id)) {
        this.nodePresence.set(id, state.presence); nodes.push(state.node);
      } else { this.nodes.delete(id); this.nodePresence.delete(id); }
    }
    const frame = { data: { nodes, flows }, moving, ringPresence: this.ring };
    if (!moving) this.settled = frame;
    return frame;
  }
}
