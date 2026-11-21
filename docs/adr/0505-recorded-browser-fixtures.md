# 0505. Replay recorded service answers for browser tests

The browser fixture replays product replies and ordered followed messages collected from the app. It refuses a format mismatch by naming the tables that differ. A request the app did not make is counted and refused. Requests for itemized results can regroup already recorded items, with each answer renumbered to its position in the new batch. Followed messages translate client subscription IDs and preserve service-owned live-cell IDs. A completed answer wins over a cancelled or refused attempt at the same request.

The test drives the recorded pages at their original size and hours. It keeps one client as pages change so cached answers and prepared layouts survive the same transitions as in the app. Widget code loads before the timed run. The fixture can then compare requested data, painted values, commit work and shown hours without including the test server's compilation time. A partial playback run can seek to the same recorded message hour to compare settled output while allowing display-frame timing to vary.

[Recorded replies](../../ui/src/test-support/recorded.ts), [Browser fixture](../../ui/src/test-support/recordedPage.dom.fixture.tsx), [Fixture driver](../../ui/src/test-support/recordedDriver.ts)
