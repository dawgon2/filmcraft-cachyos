# Cloud queue

Jobs for the cloud routine (branch `cloud-work`, only while the PC is down; FORK-PLAN §1a.23).
CPU-verifiable, chosen not to overlap with what the PC run is doing. Take them top to bottom; note
each in `docs/fork/CLOUD-LOG.md` with what is UNVERIFIED ON GPU.

**PC is doing now (2026-10-11): U0, the first upstream merge**, on a local branch that touches
almost every crate. Until U0 lands on `hevc-vaapi`, **don't change Rust code** (it would conflict
with the merge); docs-only jobs below.

1. **GUIDE-1, `docs/guide/` (docs only):** `glossary.md`: plain-language definitions (log, gamut,
   working space, node, serial node, wheel, curve, still, wipe, key, matte, LUT, grain, proxy,
   render cache), each with the Resolve and Premiere names side by side, our own words.
   Done = page written, every term tagged with its provenance (README rules), README status
   updated. GPU owed: none.
2. **GUIDE-2, `docs/guide/hardware.md` (docs only):** what runs where on this machine (VA-API
   decode / encode on the AMD VCN, Intel as second device, zero-copy display, CPU fallbacks via the
   ffmpeg pipe, the Settings switches and env vars from `docs/fork/HANDOFF.md` "Environment"), in
   user language. Done = page written with provenance tags. GPU owed: none.
3. **GUIDE-3, `docs/guide/color/recipes.md` (docs only):** 4–6 recipes using only G0 / G1 features
   (normalise F-Log2 with a starter node tree, warm a shot with the wheels, match two shots with a
   still and the wipe, copy a grade to a scene, add grain). Every command id must exist (grep
   `crates/engine/src`). Done = page written. GPU owed: none.
4. **REVIEW-1 (no code change):** read `UPSTREAM-PLAN.md` "Contributing back" and list, per
   candidate, whether upstream `main` already has an equivalent (by reading upstream's code on
   GitHub, never copying), in `docs/fork/CONTRIBUTION-CANDIDATES.md`. GPU owed: none.
