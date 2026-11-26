export function setStyle(element: HTMLElement | null | undefined, property: string, value: string): void {
  if (element && element.style.getPropertyValue(property) !== value) element.style.setProperty(property, value);
}
