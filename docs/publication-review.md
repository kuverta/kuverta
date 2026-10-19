# Publication preparation review

Reviewed on 2026-10-19. This records preparation of a local review branch;
no push, pull request or release was performed by this preparation.

## Scope

The snapshot starts directly from public `origin/main` at
`f84155f5d2c2a379687544b6c5a428d660b23790` (release 0.5.45). It incorporates
101 changed source, test, dependency and documentation files, plus this report.
The complete current file delta is retained without cherry-picking or merging
prior local practice commits. Existing public history remains unchanged.

## Privacy review

- Inspected the changed-file inventory, full contents of all 101 files, added
  lines, documentation, fixtures and the new commit metadata. Targeted checks
  covered known private identifiers, account domains, local profile paths,
  credential URLs, common API-token formats, private keys and secret assignments.
- Reviewed the email, URL and absolute-path inventories. New examples use
  synthetic identities and paths. Removed a remaining personal attribution
  from the README. Production databases, mailbox exports, real attachments,
  screenshots, backups and private audit logs are not included in this delta.
- Three pattern findings were unchanged examples inherited from public main,
  not newly copied private records. The review does not rewrite upstream history
  or claim that every historical public fixture is anonymous.
- The new commit uses the existing public contributor identity with a GitHub
  no-reply email address. Its message contains technical information only.

These are targeted pattern checks and manual review, not a guarantee that a
scanner can identify every possible secret. Review subsequent changes as well.
Detailed local scan inventories stay outside tracked files.

## History and validation

A local bundle was created using only the prepared branch ref and cloned into
a separate bare repository. Verification confirmed a single new snapshot commit
whose parent is the public base. None of the 48 prior local practice commits
was reachable or even present as a commit object in the isolated repository.
`git fsck --full --no-reflogs` passed. No symlink or submodule was added.

Runtime sources, test contents and dependency lockfiles match the previously
tested development snapshot; the only source-file difference is removal of a
trailing blank line in a synthetic fixture. All 34 changed JavaScript/module
files pass `node --check`, and `git diff --check` passes. The previously recorded
59 focused regression tests and native Windows checks are documented in
[releasing.md](releasing.md). They were not rerun or represented as remote CI
results during this history-only preparation. No installation change or live
mail/model test was needed.

## Publishing later

Use only the prepared branch for a future review. Keep the local practice branch
and its private history local; its push guard remains in place. Do not merge that
history back into this branch or distribute a copy of the shared `.git` directory.
A source archive or Git transfer must select only the reviewed branch.
Before publishing, review changes made after this preparation and check that the
public base has not moved. See [publication hygiene](releasing.md#publication-hygiene).
