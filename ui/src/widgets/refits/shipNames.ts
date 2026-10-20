import { tokenName } from '../../dashboard/navalYards.ts';
import type { YardShip, YardsReply } from '../../dashboard/navalYards';

/** Where a ship's shown name comes from: the game's recorded name, a name rebuilt from the
 * recorded naming operands through the content's name lists, or none (its design then stands in). */
export type ShipNameSource = 'recorded' | 'rebuilt' | 'design';

/** A ship's shown name and its source. A ship without a recorded or rebuilt name shows its
 * design's name (else its class's), marked as such, never as its own name. */
export function shipLabel(reply: YardsReply, ship: YardShip | undefined): { name: string; source: ShipNameSource } {
  if (!ship) return { name: 'Ship name unavailable', source: 'design' };
  const recorded = tokenName(reply, ship.measures.get('name'), '');
  if (recorded) return { name: recorded, source: 'recorded' };
  const rebuilt = tokenName(reply, ship.measures.get('nameRebuilt'), '');
  if (rebuilt) return { name: rebuilt, source: 'rebuilt' };
  const design = tokenName(reply, ship.measures.get('designName'), '') || tokenName(reply, ship.measures.get('class'), 'Ship name unavailable');
  return { name: design, source: 'design' };
}

export const shipName = (reply: YardsReply, ship: YardShip | undefined) => shipLabel(reply, ship).name;

/** The tooltip row naming the shown name's source. The game's held name and one rebuilt
 * from its naming are what the recording shows, not names checked against a save. */
export function shipNameRow(source: ShipNameSource): { label: string; value: string } {
  return { label: 'Name', value: source === 'recorded' ? 'As the game held it; not checked against a save'
    : source === 'rebuilt' ? 'Rebuilt from the recorded naming; not checked against a save' : 'Not recorded; design shown' };
}
