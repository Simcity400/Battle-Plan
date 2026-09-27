# Command power

Each country's command power balance over time, one line per country, with the hours its balance fell. It is the [Political power](../political-power/README.md) widget's chart for another recorded balance: both render [`country-measure/MeasureWidget`](../country-measure/MeasureWidget.tsx) with the same look, settings and behaviour.

## Data

- **Product `country-measure`** with `measure: 'commandPower'` (`crates/dashboard/src/products/country_measure.rs`): each country's `Countries.command_power_raw` as runs of the exact fixed-point integer, and its net drops. Runs and drops follow the same rules as political power.
- **Products `country-styles` and `country-groups`**, as for Political power.
- **Product `country-stability`** (`crates/dashboard/src/products/country_stability.rs`) for the **maximum**: each country's maximum command power, computed from its recorded inputs with the engine's arithmetic (`GetMaxCommandPower`: the loaded define plus the recorded `max_command_power` and `max_command_power_mult` totals, less the command power reserved by allocators, as the game's top bar shows it and clamps the balance to). It is drawn dashed in the country's colour, named by a "Dashed line" key under the legend and in the tooltip; the **Maximum** setting hides it. It loads on its own, so the balance never waits for it, and a country's maximum follows the same group membership as its balance. Where the inputs were not recorded it is missing, never guessed from defines.

## Not recorded yet

- **Daily income and causes** (abilities, traits, promotions, upkeep) need new capture, so spends are **net** falls in the balance and there is no drill-down.
