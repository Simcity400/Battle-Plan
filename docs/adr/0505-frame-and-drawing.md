# 0505. Measure, paint, and commit in one dashboard frame loop

The dashboard advances its playback clock, lands data in arrival order, measures layout, paints, commits React work, then runs after-frame tasks. A task for a later phase can join the current frame. The measure phase owns layout reads so position changes, canvas painting and React commits do not interleave with measurement. A hidden page or a runtime without animation frames uses a timer, and the frame time step is bounded to avoid animation jumps after a stall. Offscreen widgets stop receiving frames and repaint once on return. Their clock methods keep stable identities.

The dashboard engine drives row entry, exit and gliding with the same curves used for table motion. Transform transitions on many individual elements caused the browser to restyle a compositor layer for every moving element. A single paint-phase task updates the styles instead. Retargeting starts from the value already drawn. Reduced motion snaps directly to the target.

Draw commands take layout coordinates, with source raster rectangles and image resolution in raster pixels. Retained paths bind their components to the chart axes that should move with the view and can be revealed at a specific clock position. A retained backend uploads only the changed raster columns. Worker clock anchors carry a generation to discard queued notices from an earlier anchor and a seek count to discard prepared steps from the old path. The host keeps moving canvases hidden during a rate change until the worker has painted under the new anchor.

At integral playback step sizes the worker uses the first display frame after the native step becomes eligible. Refresh-rate measurements use short animation-frame intervals so occasional dropped frames do not determine the display period. Workers share the dashboard's timing curves without starting a second dashboard frame loop.

Chromium's slowdown affects the page's main thread and leaves drawing workers at normal speed. The worker benchmark therefore accounts for the measured frame work and adds a proportional delay by occupying that worker. Rate one adds no delay. The fixture checks repeatable outcomes separately from measured frame timing.

[Frame engine](../../ui/src/engine/frame.ts), [Motion](../../ui/src/engine/glide.ts), [Draw commands](../../ui/src/draw/kit.ts), [Drawing worker](../../ui/src/draw/worker.ts), [Worker benchmark](../../ui/src/draw/weakCpu.ts)
