# Third-party notices

These notices apply to the identified third-party material below. They do not
license the rest of this repository or any locally installed game or mod content.

## Lucide refresh-cw icon

The `refresh-cw` SVG paths in [Icons.tsx](ui/src/components/Icons.tsx) come from
[Lucide's refresh-cw icon](https://github.com/lucide-icons/lucide/blob/main/icons/refresh-cw.svg).
The paths are unchanged; the shared component uses its own stroke width.
The upstream SVG blob is `c27b3a9677e35fb2353167b01553d3d4cc4e3091`.
Retain the following [upstream ISC notice](https://lucide.dev/license) with source
copies and application distributions containing this icon.

```text
ISC License

Copyright (c) 2026 Lucide Icons and Contributors

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
```

## Dependency and compiler distributions

Cargo and npm dependency sources are obtained from their registries; their own
licenses remain applicable. A packaged application must include the notices its
bundled dependencies require. This file does not replace those licenses.

The DXC distribution carries separate upstream licenses. The pinned
[compiler installer](tools/install-shader-compiler.ps1) retains them; follow the
[shader-pipeline packaging contract](crates/shader-pipeline/README.md#compiler-and-packaging)
when distributing the desktop application.
