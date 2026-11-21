# Preserve recorded lane bands when drawing at pixel resolution

Map recorded steps to whole pixel columns. A fully covered column shows the thickest step that reaches it and keeps that step's span and hour. Leave a column touching an unrecorded gap empty. Its peak can carry to the nearest drawn column within the same span. A step clipped only by the plot edge can still draw. Adjacent steps that stay covered across a holder change can meet without introducing a data gap.

If a span would receive no column at all, try the midpoint column it touches and then its other columns in distance order. It can occupy a free column or displace a thinner mark, which then tries its remaining columns. Full-coverage columns stay assigned. Where possible, shift thin marks away from an unrecorded gap wider than a pixel. Resolve neighbors in both directions so a mark pinned at a plot edge can leave the movement to its neighbor. Use indexed queues and arrays, and a sweep over steps and plot columns, to keep many small bands affordable. A recorded zero can still have the minimum visible thickness.

A plain bar narrower than a pixel keeps a full column inside the plot if any of its hours are visible. Hit testing uses the rectangles in reverse draw order. Band hits use the span and step assigned to the drawn column. Clamp the described hour to that step when a short band moved outside its own hours, so hover and Jump to agree. A band layout is reused while its lane and view identity are unchanged. Off-screen and removed lanes drop from this cache.

Keyboard stops include the actual draws, step changes and supplied read hours that keep the same bar. A zoom starting in a partial hour can reach the held value there, even when it is the span's last hour. Floor the described hour as pointer hover does. Skip stops outside the visible range, outside reached hours or in gaps in a band. Use a linear walk to match ordered stops and steps. A stable stop ID lets keyboard focus follow the selected observation as the view changes.

[Timeline geometry](../../ui/src/charts/timeline/layout.ts) · [Band behavior tests](../../ui/src/charts/timeline/bands.test.ts)
