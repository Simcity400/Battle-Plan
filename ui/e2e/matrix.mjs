import path from 'node:path';
import { outsideRepository, readJSON } from './cache.mjs';

const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const fields = (value, allowed, label) => {
  if (!object(value) || Object.keys(value).some(key => !allowed.includes(key))) throw Error(`Invalid ${label} fields`);
};

/** A matrix supplies exact user choices. Widget-specific validity is checked
 * by the parity hook against the real definition before the widget mounts. */
export function validateMatrix(value) {
  fields(value, ['version', 'cases'], 'capture matrix');
  if (value.version !== 1 || !Array.isArray(value.cases) || !value.cases.length) throw Error('Capture matrix requires version 1 and nonempty cases');
  const ids = new Set();
  for (const item of value.cases) {
    fields(item, ['id', 'kind', 'hour', 'height', 'countries', 'time', 'options', 'focusDetails', 'models'], 'capture case');
    if (typeof item.id !== 'string' || !item.id || ids.has(item.id) || typeof item.kind !== 'string' || !item.kind
      || !Number.isInteger(item.hour) || item.hour < -0x8000_0000 || item.hour > 0x7fff_ffff
      || item.height !== undefined && (!Number.isInteger(item.height) || item.height < 1)
      || item.countries !== undefined && !object(item.countries)
      || item.time !== undefined && !object(item.time)
      || item.options !== undefined && (!object(item.options) || Object.values(item.options).some(value =>
        value !== null && typeof value !== 'boolean' && typeof value !== 'string' && !(typeof value === 'number' && Number.isFinite(value))))
      || item.focusDetails !== undefined && item.focusDetails !== 'first-visible'
        && !(typeof item.focusDetails === 'string' && /^(0|[1-9]\d*)$/.test(item.focusDetails))
      || item.models !== undefined && (!Array.isArray(item.models) || item.models.some(label => typeof label !== 'string' || !label)
        || new Set(item.models).size !== item.models.length)) throw Error(`Invalid capture case: ${String(item.id)}`);
    if (item.focusDetails !== undefined && item.kind !== 'national-focus') throw Error('Focus details require the national-focus widget');
    ids.add(item.id);
  }
  return value;
}

export async function readMatrix(file) {
  if (!file) return null;
  const source = outsideRepository(path.resolve(file));
  return { source, ...validateMatrix(await readJSON(source)) };
}

export function captureCases(catalogue, matrix, hour) {
  const kinds = new Set(catalogue.map(item => item.kind));
  if (!catalogue.length || kinds.size !== catalogue.length) throw Error('Invalid widget catalogue');
  if (!matrix) return catalogue.map(item => ({ id: item.kind, kind: item.kind, hour }));
  for (const item of matrix.cases) if (!kinds.has(item.kind)) throw Error(`Capture widget is not registered: ${item.kind}`);
  return matrix.cases;
}
