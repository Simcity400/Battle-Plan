const familyName = (name: string) => name.trim().replace(/^(['"])(.*)\1$/, '$2').replace(/\\(["'\\])/g, '$1').toLowerCase();

export function fontFamilies(font: string): string[] {
  return (font.match(/(?:[^,"'\\]|\\.|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')+/g) ?? []).map(familyName);
}

export function loadedFontFamily(event: FontFaceSetLoadEvent, families: ReadonlySet<string>): boolean {
  return event.fontfaces.some(face => families.has(familyName(face.family)));
}
