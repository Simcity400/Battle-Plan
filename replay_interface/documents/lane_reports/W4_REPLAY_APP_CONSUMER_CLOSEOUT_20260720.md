# Wave 4 replay-app consumer closeout

## Product boundary

`replay_app::selected_pov_runtime` is now the single typed view-model boundary
for the late-RE selected-country surfaces. `ProductSelectedPresentRuntime`
refreshes it from the current `WorldFrame` and selected POV before preparing
the authenticated selected-Present operation stream. The boundary never reads
raw replay JSON and records a feature-local unavailable reason instead of
constructing empty/default data.

## Wired consumers

- A17 topbar nukes require all three witnesses: a resolved D18 index, the
  recorder-retained `nuke_stockpile_tag_db_id`, and the exact current-hour d83
  NUK1 sidecar row with the same built total. The generic scalar projection no
  longer displays a nuke value by itself.
- A30 agency and cipher summaries consume the strict replay-core controller
  admission gates, retain producer order, require current-hour authority, and
  require the controller owner to equal the selected POV.
- A31 national focus calls the strict game-UI exact presentation adapter when
  the authenticated effective topology, catalog, and UI09 canvas are supplied.
- A34 decision panels convert only the typed d87 exact view, require the panel
  country to equal the selected POV, and bind only through an admitted static
  decision catalog.
- A36 abilities load the effective-VFS definition/icon catalog and call the
  exact ABA2/ABL1 game-UI admission for the selected country's politics view.
- A39 character presentation calls the exact d82 overlay and roster bijection
  gate when the still-required assignment/name/portrait inputs are supplied.
- A40 selected designer calls the complete d59/d85 portable join. Peace remains
  explicitly unavailable because `WorldFrame` has no typed peace-transition
  carrier; an externally supplied typed snapshot may use the existing strict
  read-only admission without implying that a producer exists.
- Domain 54 army-order controls call the strict typed binder and preserve the
  distinction between no selected army and an unavailable manager.
- X2/A33 combat detail constructs the typed multi-family binding and runs the
  XUI20 admission. Each family retains its own status/order; no global combat
  log order is invented.

## Deliberate unavailable states

The production runtime does not synthesize focus topology, a decision catalog,
character assignment/name/portrait data, a selected army, or a peace snapshot.
Those inputs remain unavailable until an authenticated upstream owner supplies
them. Dormant extractor producers therefore remain dormant, and no replay-app
consumer change bypasses their activation gates.

No build, test runner, verifier, compiler, or formatter was executed in this
subagent lane. Central validation belongs to the primary agent.
