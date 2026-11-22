import type { RowCell } from '../panels/view';

export class PanelProgram {
  private readonly ctx: OffscreenCanvasRenderingContext2D;
  private drawn: readonly RowCell[] = [];
  private values: string[] = [];

  constructor(ctx: OffscreenCanvasRenderingContext2D) { this.ctx = ctx; }

  private erase(cell: RowCell) {
    const pad = cell.knob ? 4 : 0;
    this.ctx.clearRect(cell.x - pad, cell.y - pad, cell.width + pad * 2, cell.height + pad * 2);
  }

  clear() {
    for (const cell of this.drawn) this.erase(cell);
    this.reset();
  }

  reset() { this.drawn = []; this.values = []; }

  write(cells: readonly RowCell[], values: ReadonlyMap<string, string>) {
    if (cells !== this.drawn) this.clear();
    const ctx = this.ctx;
    let changed = false;
    for (let i = 0; i < cells.length; i++) {
      const cell = cells[i]!, value = values.get(cell.key) ?? cell.text;
      if (value === this.values[i]) continue;
      this.erase(cell);
      changed = true; this.values[i] = value;
      ctx.fillStyle = cell.color;
      if (cell.meter) {
        if (!value) continue;
        const ratio = Math.min(1, Math.max(0, Number(value)));
        const vertical = cell.meter === 'vertical', w = cell.width * (vertical ? 1 : ratio), h = cell.height * (vertical ? ratio : 1);
        if (w && h) { ctx.beginPath(); ctx.roundRect(cell.x, cell.y, w, h, Math.min(w, h) / 2); ctx.fill(); }
        if (cell.knob) { ctx.fillStyle = '#fff'; ctx.beginPath(); ctx.arc(cell.x + w, cell.y + h / 2, 3.5, 0, 2 * Math.PI); ctx.fill(); }
      } else {
        ctx.font = cell.font; ctx.textAlign = cell.align; ctx.textBaseline = 'alphabetic';
        ctx.fillText(value, cell.x + cell.width * (cell.align === 'right' ? 1 : cell.align === 'center' ? .5 : 0), cell.baseline);
      }
    }
    this.drawn = cells;
    return changed;
  }
  texts(): [key: string, text: string][] { return this.drawn.map((cell, i) => [cell.key, this.values[i] ?? cell.text]); }
  snapshot() {
    return this.drawn.map((cell, i) => {
      const value = this.values[i];
      let excess = 0;
      if (!cell.meter && value) {
        this.ctx.font = cell.font;
        excess = this.ctx.measureText(value).width - cell.width;
      }
      return { row: cell.row, key: cell.key, value, ...(excess > 1 && { overflow: excess }) };
    });
  }
  hit(x: number, y: number) {
    for (let i = this.drawn.length - 1; i >= 0; i--) {
      const cell = this.drawn[i]!;
      if (x >= cell.x && x < cell.x + cell.width && y >= cell.y && y < cell.y + cell.height) return cell.row;
    }
    return null;
  }
}
