# Bundled font notices

The frontend bundles the following fonts from `@fontsource-variable` 5.3.0.
Each notice is an unmodified copy of that package's `LICENSE`, including its
upstream attribution and the complete SIL Open Font License 1.1.

| Font | Upstream package | Notice |
|---|---|---|
| Cascadia Code | `@fontsource-variable/cascadia-code` | [Licence](Fontsource-cascadia-code-LICENSE.txt) |
| Inter | `@fontsource-variable/inter` | [Licence](Fontsource-inter-LICENSE.txt) |
| Roboto | `@fontsource-variable/roboto` | [Licence](Fontsource-roboto-LICENSE.txt) |
| Roboto Condensed | `@fontsource-variable/roboto-condensed` | [Licence](Fontsource-roboto-condensed-LICENSE.txt) |
| Source Serif 4 | `@fontsource-variable/source-serif-4` | [Licence](Fontsource-source-serif-4-LICENSE.txt) |

# Ported code notices

viewer-audio's [sdl.rs](../crates/viewer-audio/src/sdl.rs) is altered from
SDL 2.0.20's audio conversion code; its notice is an unmodified copy of that
release's `LICENSE.txt`: [Licence](SDL-LICENSE.txt).

Desktop packaging copies this directory beside the executables. Retain the
notices when redistributing the application or its bundled fonts.
