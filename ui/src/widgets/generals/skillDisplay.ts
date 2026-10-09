/** Fit from the rendered font and available cell width, without allocating a mod-sized string. */
export function compactSkill(maximum: number, width: number, starWidth: number): boolean {
  return width <= 0 || starWidth <= 0 || maximum * starWidth > width;
}
