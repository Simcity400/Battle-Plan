export class HeldPointer {
  private point: { x: number; y: number } | null = null;
  private key: string | null = null;
  private readonly tell: (key: string | null) => void;
  constructor(tell: (key: string | null) => void) { this.tell = tell; }
  move(x: number, y: number) { this.point = { x, y }; }
  present(hit: (x: number, y: number) => string | null) {
    if (this.point) this.update(hit(this.point.x, this.point.y));
  }
  clear() { this.point = null; this.update(null); }
  private update(key: string | null) {
    if (key === this.key) return;
    this.key = key;
    this.tell(key);
  }
}
