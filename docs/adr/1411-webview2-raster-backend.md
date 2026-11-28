# WebView2 raster backend

The desktop app asks WebView2 to use the Ganesh raster backend by passing `--disable-skia-graphite`. On WebView2 158.0.4370.45 with an RTX 5090, Graphite allocated four-sample colour and depth attachments for the compositor. The runtime’s raster sample-count argument alone left 19 four-sample depth and 21 four-sample colour attachments in place. Ganesh retains hardware acceleration for compositing, rasterization and Canvas2D.

The main and hidden export WebViews share one browser argument list. It carries Wry’s existing mini-menu, PDF mini-menu, SmartScreen and autoplay arguments because Wry replaces its defaults when the app supplies arguments. If a later runtime ignores the raster switch, it uses its default backend and memory footprint. Playback and export still use the same application paths.

The visible difference is edge antialiasing on dashboard icons and rounded shapes. Text, recorded numbers, colours and layout match, and native map exports matched in RGB on both recordings.

The dedicated startup traces found about 250 MB less GPU-process private memory on vanilla and Millennium Dawn. In the repeated playback runs, median GPU-process private memory at open fell by 203 MB on vanilla and 123 MB on Dawn. The whole desktop app’s peak private memory during playback fell by 222 MB and 196 MB, respectively. Whole-app memory with the map visible was within the repeat spread. Six alternating Dawn runs after panel catch-up was bounded showed no consistent difference in Training queue freshness.
