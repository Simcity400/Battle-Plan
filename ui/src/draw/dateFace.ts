import { formatDate, formatDateTime, formatTime, gameDate } from '../dashboard/calendar.ts';

export interface DateFaceModel {
  offset: number;
  date: { font: string; color: string };
  hour: { font: string; color: string };
}

type Context = OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D;

export function dateFaceTexts(hour: number | null): { date: string; hour: string } {
  const date = hour == null ? null : gameDate(hour);
  return { date: date ? formatDate(date) : hour != null ? formatDateTime(hour) : 'Choose date', hour: date ? formatTime(date.hour) : '—' };
}

export function drawDateFace(ctx: Context, model: DateFaceModel, hour: number | null, width: number, height: number, dpr: number): void {
  const texts = dateFaceTexts(hour);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);
  ctx.font = model.date.font;
  const metrics = ctx.measureText(texts.date);
  const baseline = Math.round((height + metrics.fontBoundingBoxAscent - metrics.fontBoundingBoxDescent) / 2);
  ctx.textBaseline = 'alphabetic';
  ctx.textAlign = 'left';
  ctx.fillStyle = model.date.color;
  ctx.fillText(texts.date, 0, baseline);
  ctx.font = model.hour.font;
  ctx.textAlign = 'right';
  ctx.fillStyle = model.hour.color;
  ctx.fillText(texts.hour, width, baseline);
}
