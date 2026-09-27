# Command power

Each country's command power balance over time, one line per country, with the hours its balance fell. It is the [Political power](../political-power/README.md) widget's chart for another recorded balance: both render [`country-measure/MeasureWidget`](../country-measure/MeasureWidget.tsx) with the same look, settings and behaviour.

## Data

- **Product `country-measure`** with `measure: 'commandPower'` (`crates/dashboard/src/products/country_measure.rs`): each country's `Countries.command_power_raw` as runs of the exact fixed-point integer, and its net drops. Runs and drops follow the same rules as political power.
- **Products `country-styles` and `country-groups`**, as for Political power.

## Not recorded yet

- **The maximum** (drawn as a dashed ceiling in the spec) is not recorded. It is part of the new modifier capture (Wave 2, part A); until then no ceiling is drawn and nothing is guessed from defines.
- **Daily income and causes** (abilities, traits, promotions, upkeep) need new capture, so spends are **net** falls in the balance and there is no drill-down.
