import { fitLabelWidth } from '../labels.ts';

export interface TimelineLabel { text: string; space: number; weight: number; size?: number }

export function catalogueWidth(labels: readonly TimelineLabel[], measure: (label: TimelineLabel) => number): number {
  return fitLabelWidth(labels, label => measure(label) + label.space, 120, 280, 0);
}
