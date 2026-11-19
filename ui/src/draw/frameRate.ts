export class PresentationRate {
  private next: number | undefined;
  private last = -Infinity;
  reset() { this.next = undefined; this.last = -Infinity; }

  due(at: number, cap: number, boundary?: number) {
    if (!cap) return true;
    const interval = 1000 / cap;
    if (boundary !== undefined) {
      if (boundary <= this.last) return false;
      this.next = boundary + interval;
    } else {
      this.next ??= at;
      if (at + .1 < this.next) return false;
      this.next += (1 + Math.floor((at + .1 - this.next) / interval)) * interval;
    }
    this.last = at;
    return true;
  }
}

/** Short animation-frame intervals expose the display period even when a few frames are missed. */
export class DisplayRate {
  private previous?: number;
  private gaps: number[] = [];
  private hz = 0;

  sample(at: number) {
    if (this.previous !== undefined && at > this.previous) this.gaps.push(at - this.previous);
    this.previous = at;
    if (this.gaps.length < 32) return;
    this.gaps.sort((a, b) => a - b);
    const measured = Math.round(1000 / this.gaps[3]!);
    this.gaps.length = 0;
    if (measured > 0 && Math.abs(measured - this.hz) > this.hz * .02) {
      this.hz = measured;
      return measured;
    }
  }
}
