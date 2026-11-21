import type { ChordData } from './layout.ts';
import { ChordMotion } from './motion.ts';

export interface ChordFrame {
  readonly data: ChordData | null;
  readonly playing?: boolean;
  readonly segment?: unknown;
  readonly revision?: unknown;
}

export class ChordFrames {
  readonly motion = new ChordMotion();
  data: ChordData | null = null;
  revision: unknown;
  private segment: unknown;

  accept(frame: ChordFrame): void {
    if (frame.segment !== this.segment || frame.data == null) this.clear();
    this.segment = frame.segment;
    if (frame.data && frame.data !== this.data) {
      this.motion.set(frame.data);
      this.data = frame.data;
    }
    this.revision = frame.revision;
  }

  clear(): void { this.motion.clear(); this.data = null; this.revision = undefined; }
}
