# Publishing source and artifacts

Keep the existing development repository private. The October 2026 audit found
personal paths and commit identities, agent transcripts, captures, executable
artifacts, and decompiled/game-derived material in reachable history. A cleanup
commit changes the current tree; it does not remove old blobs, authors, messages,
tags, pull-request diffs, or hosted build output.

## Current-tree policy

Game screenshots and viewer images containing game artwork are private inputs.
The [reference notes](../reference/README.md) retain measurements and limitations;
matching images belong under ignored `captures/reference/`. Obsolete shader port
oracles are also private. Production reads game assets and shaders from the
user's recorded content stack; an installed file is not permission to distribute
its source or a compiled copy. Use independently authored synthetic test inputs
instead of copying installation files into fixtures.

The shared [.gitignore](../.gitignore) excludes captures, local settings and agent
configuration, environment files, databases, recordings, crash dumps, compiled
shader output, and local build caches. It does not remove previously tracked files
or protect force-added files. Stage explicit paths and review the staged diff.

Run the small [publication guard](../tools/check-publication.py) before committing:

```powershell
python tools/check-publication.py
python tools/check-publication.py --revision HEAD
```

The first command reads the staged index; the second reads that committed tree.
It checks private artifact paths, unreviewed binary files, concrete home-directory
paths, URL credentials and several common token/key formats. It reports locations
without printing values. Reviewed original application artwork and synthetic codec/
archive fixtures are allowed only at their reviewed Git blob hashes. Passing is
not a complete secret scan, a history audit,
a review of arbitrary personal information, or copyright clearance. Review Git
identities, free text, visual assets, archive contents and hosted metadata separately.
Keep detailed audit reports and original evidence under ignored `captures/`.

## Rights and recording privacy

The [third-party notices](../THIRD_PARTY_NOTICES.md) cover the identified embedded
icon material. Rust/Node dependencies retain their own licences. Binary releases
need a separate dependency/asset notice audit; keep the upstream compiler licences
required by the [shader-pipeline packaging contract](../crates/shader-pipeline/README.md).
No project-wide source licence has been selected. Choose one only for material the
project has the right to license; public visibility alone does not grant open-source
rights ([GitHub licensing guidance](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/licensing-a-repository)).

Paradox retains its own and third-party content rights. Its screenshot/streaming
policy is not a blanket licence for copied game source or shader ports. Review
publication and distribution rights against the applicable
[Paradox User Agreement](https://legal.paradoxplaza.com/eula?locale=en); this technical
audit does not settle the legal status of reverse-engineered implementations.

Real recordings can contain player identifiers/display names, user-renamed units,
campaigns and factions, mod names, and session times. Machine-path-free content
identity does not anonymize those fields. Saves, recordings, screenshots, logs,
crash dumps, compiler caches and diagnostic exports need their own review before
sharing. Do not attach raw captures to public issues.

## Publish without the private history

The safer route is a new repository initialized from the reviewed tracked tree,
with a public alias and GitHub noreply author email, while keeping this development
repository private. Export only that tree, initialize fresh Git, and verify that
the resulting repository has one root commit and no old refs, remotes or objects.
Do not use a fork, mirror, old tags, old branches, or copied `.git` metadata as the
public starting point. Recheck the snapshot and any added README, licence or build
artifact before making that new repository public.

The exported tree also leaves out what this repository keeps as development
evidence: executable addresses and game function names in code, comments and docs;
game file paths, line citations and installation paths; data extracted from the
game (`crates/installed-content/src/gui/native/keywords.tsv` and the game-file hash table in
`crates/viewer/src/map/native_effects.rs`); copied shader text and game formulas in
comments; and `reference/`. The public build must work without the extracted data.

If the existing repository must become public, coordinate a complete history and
identity rewrite across every hosted branch/tag and account for old pull-request
refs, forks, cached commits, comments, logs and artifacts. A force-push alone cannot
purge all hosted copies ([GitHub removal guidance](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/removing-sensitive-data-from-a-repository)).
Keep it private until that separate cleanup is complete. Local T3/Codex checkpoints
and private backups are not ordinarily pushed, but must never be mirror-published.
