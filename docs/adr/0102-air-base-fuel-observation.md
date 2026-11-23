# Read current state-base fuel separately from held registration spans

The existing AirBases worker reads both fuel words for each current container after a complete registration snapshot. The registration segment establishes membership and can carry across hours, but its held spans intentionally omit fuel values. Fuel demand is updated in the hourly strategic-air worker stage, and allocation in the serial country pass before the boundary. Those stages are quiescent during capture. This establishes when to read the values without assuming an exhaustive set of demand writers.

The worker gathers demand first and allocation second for every container. When one guarded word fails, the gather resumes after it so the other word and other containers keep their observations. A failure of the optional state-fuel projection leaves the predecessor registration rows intact.

Metadata projections interpret subfields of owned spans separately from the typed replay's exact-key cursor. A covering observation wins even when it failed, and the worker may fetch additional subtype or owner fields only during fresh membership projection. The reusable index holds geometry and ordinals alone; each value and failure comes from the current owned log. Equal start addresses keep their input order, including later failed observations, during the stable address sort. A partially built index is discarded on error before another hour can use it.

[State-base fuel projection and gather](../../crates/collector-air/src/extended/air/state_fuel_inputs.rs)
