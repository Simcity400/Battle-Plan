import { approach } from '../charts/easing.ts';

/** A number the engine eases toward its target each frame with the dashboard's one easing (`approach`, its time constant
 * from `easeTau`), so motion drawn by the engine feels exactly as it did. Tau 0 (paused, reduced motion) snaps. */
export class Tween {
  value: number;
  target: number;

  constructor(value: number) {
    this.value = value;
    this.target = value;
  }

  /** Moves toward the target by one frame of `dt` ms; true while still moving. */
  step(dt: number, tau: number): boolean {
    this.value = approach(this.value, this.target, dt, tau);
    return this.value !== this.target;
  }

  /** Lands on `value` at once (a seek, a gap, a new selection). */
  jump(value: number): void {
    this.value = value;
    this.target = value;
  }
}
