# ADR 0005: Syncing the fork with upstream storytold/filmcraft (fork)

- **Status:** accepted for the fork (2026-10-11, session 11, Phase U0). Plan set by the owner in
  [`UPSTREAM-PLAN.md`](../../UPSTREAM-PLAN.md); this records how the merge is done.
- **Note:** COMPOSITOR-PLAN's node-compositor ADR takes the next free number.

## Context

The fork (`hevc-vaapi`) was started from a one-commit snapshot (`master`, "original") whose tree is
identical to upstream commit **`8fcad73`** (2026-10-07, "Merge pull request #159"), but `master`
shares no git history with upstream. A plain `git merge upstream/main` therefore has no merge base
and would treat every file as an add/add conflict. By 2026-10-11 upstream had 806 commits on top of
`8fcad73` (563 files, +96.8k / −7.0k lines); 93 of the fork's changed files were also changed
upstream. Rules: no history rewriting of `hevc-vaapi` or `master`, no force-push, never push to
`upstream`.

## Decision

1. **Merge base by a local graft.** `git replace --graft master 8fcad73` (a local replace ref,
   never pushed) makes git see `8fcad73` as `master`'s parent, so `git merge upstream/main` gets
   the true three-way base `8fcad73`. No commit is rewritten: `master` and `hevc-vaapi` keep their
   ids, and the replace ref only changes how this checkout computes merge bases.
2. **A real merge commit.** The sync result is one merge commit on `hevc-vaapi` with parents
   (fork tip, `upstream/main` at the fetched commit). Upstream's history comes in as-is. After this
   first merge the next sync has a genuine common ancestor (the merged upstream commit), so the
   graft is only needed for U0.
3. **Work on `sync/upstream-<date>`, in its own worktree** (`.claude/worktrees/sync-upstream`), so
   the main checkout stays usable. Resolution and follow-up fixes may take several commits on the
   sync branch (a checkpointed partial merge included); when every gate is green the final merge
   commit on `hevc-vaapi` is created fresh (`git merge --no-commit upstream/main`, then the sync
   branch's final tree is checked out over it), so `hevc-vaapi`'s history gets one clean merge
   commit and no commit with conflict markers. The sync branch stays local.
4. **Two stages.** Stage 1: a faithful textual merge that keeps both sides' behaviour (both VA-API
   stacks compiled in, the fork's ffmpeg pipe renamed `vaapi_pipe` to end the clash with upstream's
   `vaapi/` module). Stage 2: consolidation per UPSTREAM-PLAN "U0 merge notes" (one VA-API decode
   stack with the fork's zero-copy path on top, upstream's `OP_LUMETRI` retired in favour of the
   ColorOps chain, `gop.rs` fixes), each consolidation a separate reviewed commit. **Update
   (owner, 2026-10-11):** U0 ships stage 1 plus the Lumetri retirement; the one-VA-API-stack
   consolidation is its own phase, **U0.2**, right after Phase 6E (owner, 2026-10-11, moved from before
   Phase 8; FORK-PLAN §1a.6).
5. **Project schema.** Upstream and the fork both used schema numbers 13 and 14. Upstream's
   numbering wins (v13 multi-stream audio, v14 narrations, v15 notes); the fork's grades and
   stills become **v16**. Fork files saved as v13 / v14 (whose fork steps were no-ops) are detected
   on load and upgraded through upstream's chain from v12 (provisional, owner question).
6. **Gates before merging into `hevc-vaapi`** (UPSTREAM-PLAN step 3): `cargo xtask ci`, GPU parity
   on AMD and Intel, the xm5 quick pass, G0 / G1 tests, no regression beyond noise against the
   grade-1 numbers. Results and every conflict resolution go to `docs/fork/UPSTREAM-SYNC.md`.

## Consequences

- `git log` on `hevc-vaapi` shows upstream's commits after U0; `git blame` works across both.
- A fresh clone of `origin` has no replace ref; it doesn't need one after U0 (the merge commit
  links the histories).
- Later weekly syncs are ordinary merges of `upstream/main` (ask-first, UPSTREAM-PLAN).

## Stage 2 (U0.2, 2026-10-13)

The two VA-API decode stacks are now one: upstream's front ends with the fork's zero-copy / hand-off
/ read-back accelerator on the fork's display layer (see ADR 0002, "Update: one VA-API stack"). The
fork-only `va_hevc.rs`, `crates/hevc/src/accel.rs` (+ its test) and upstream's `vaapi/va.rs` are
removed. The factory chain is: in-process zero-copy, then in-process read-back, then the ffmpeg pipe
(fork-only). Bindings stay as PREP §3.2 item 5 recommendation **B**: `vaapi/ffi.rs` for the buffers
the front ends fill, `va_sys` for everything else.

