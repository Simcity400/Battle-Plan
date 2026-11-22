export class DisplayRate {
  private previous?: number;
  private gaps: number[] = [];
  private hz = 0;
  private gap = Infinity;
  get interval() { return this.hz ? Math.min(1000 / this.hz, this.gap) : 0; }

  sample(at: number) {
    if (this.previous !== undefined && at > this.previous) { this.gap = at - this.previous; this.gaps.push(this.gap); }
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
