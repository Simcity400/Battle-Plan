export interface PictureInterval { since: number; through: number }

export function pictureHour(date: PictureInterval | undefined, position: number, fixed?: number): number | null {
  const head = Math.floor(fixed ?? position);
  if (!date || date.since > head || date.since > date.through || (fixed !== undefined && date.through < head)) return null;
  return Math.min(head, date.through);
}

export class PictureDate {
  pending?: PictureInterval;
  drawn?: PictureInterval;
  fixedPosition?: number;

  set(through: number | null, fixedPosition?: number, since: number | null = through) {
    this.fixedPosition = fixedPosition;
    this.pending = since !== null && through !== null ? { since, through } : undefined;
  }
  canPaintAt(position: number) { return pictureHour(this.pending, position, this.fixedPosition) !== null; }
  presentableAt(position: number) { return pictureHour(this.drawn, position, this.fixedPosition) !== null; }
  paint() { this.drawn = this.pending; }
}
