import type { ChordData } from './layout.ts';
import { ChordMotion } from './motion.ts';

export interface ChordFrame {
  /** Null means unavailable, never zero. */
  data: ChordData | null;
  playing?: boolean;
  /** Change at a gap, seek or source change. */
  segment?: unknown;
}

/** Commit exact tooltip data only after the whole target has been accepted.
 * The caller clears presentation on refusal and can retry the next frame. */
export class ChordFrames {
  readonly motion = new ChordMotion();
  data: ChordData | null = null;
  private segment: unknown;

  accept(frame: ChordFrame): void {
    if (frame.segment !== this.segment || frame.data == null) this.clear();
    this.segment = frame.segment;
    if (frame.data && frame.data !== this.data) {
      this.motion.set(frame.data);
      this.data = frame.data;
    }
  }

  clear(): void { this.motion.clear(); this.data = null; }
}
