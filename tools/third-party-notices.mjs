import { readFile, readdir } from "node:fs/promises";
import path from "node:path";

const RULE = "=".repeat(78);

export async function thirdPartyNotices(folder, components) {
  const present = (await readdir(folder, { withFileTypes: true })).filter((entry) => entry.isFile()).map((entry) => entry.name);
  const listed = components.flatMap((component) => component.files);
  const problems = [
    ...listed.filter((name, index) => listed.indexOf(name) !== index).map((name) => `${name} is listed twice.`),
    ...listed.filter((name) => !present.includes(name)).map((name) => `${name} is listed but not in LICENSES.`),
    ...present.filter((name) => !listed.includes(name)).map((name) => `LICENSES/${name} is in no component of tools/shipped.json's notices.`),
  ];
  if (problems.length > 0) throw new Error(`The third-party notices do not match LICENSES:\n${problems.join("\n")}`);
  const sections = [];
  for (const [index, component] of components.entries()) {
    const texts = [];
    for (const name of component.files) {
      const text = await readFile(path.join(folder, name), "utf8");
      texts.push(`--- ${name} ---\n\n${text.endsWith("\n") ? text : `${text}\n`}`);
    }
    sections.push(`${RULE}\n${index + 1}. ${component.name}\n\n${component.use}\n${RULE}\n\n${texts.join("\n")}`);
  }
  const contents = components.map((component, index) => `${String(index + 1).padStart(2)}. ${component.name}`).join("\n");
  return `Battle Plan third-party notices\n\nBattle Plan is free software under the MIT License, in LICENSE beside this file.\n`
    + `It includes or is adapted from the components below. Each section names the component,\n`
    + `what Battle Plan uses it for, and its licence text, verbatim.\n\n${contents}\n\n${sections.join("\n")}`;
}
