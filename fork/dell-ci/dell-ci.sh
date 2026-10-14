#!/usr/bin/env bash
# FORK-ONLY: DC1 (FORK-PLAN §1a.56). Build-and-test service for the Dell, no AI involved.
# A systemd user timer starts this every 2 min. It takes the oldest job branch
# refs/heads/dell-ci/<kind>-<id> pushed to the Dell's checkout, runs it in its own worktree and
# writes ~/filmcraft-ci/results/<kind>-<id>.txt (3 lines: PASS|FAIL|TIMEOUT; commit + duration;
# first failing test / error line or "-"), the full log next to it (.log).
# Kinds: ci-<id> (cargo xtask ci), test-<crate>-<id> (cargo test -p <crate>), intel-<id> (the
# tests in fork/dell-ci/intel.list), soak-<name>-<id> (fork/dell-ci/soaks/<name>.sh).
# The <id> is the last '-'-separated token. One job at a time (flock). Never touches the
# checkout's own branch.
set -u
REPO="${DELL_CI_REPO:-$HOME/Projects/filmcraft-main}"
CI="${DELL_CI_HOME:-$HOME/filmcraft-ci}"
mkdir -p "$CI/wt" "$CI/results" "$CI/target"
exec 9>"$CI/lock"
flock -n 9 || exit 0
export PATH="$HOME/.cargo/bin:$PATH"

branch=$(git -C "$REPO" for-each-ref --sort=committerdate --format='%(refname:short)' 'refs/heads/dell-ci/' | head -n 1)
[ -n "$branch" ] || exit 0
job=${branch#dell-ci/}
case "$job" in *[!A-Za-z0-9._-]*|''|-*) echo "bad job name: $job" >&2; git -C "$REPO" branch -D "$branch" >/dev/null; exit 0 ;; esac
kind=${job%-*}
[ "$kind" != "$job" ] || kind=$job

# Owner first, but watching video doesn't stop jobs (FORK-PLAN §1a.56 gentle mode). Every job runs
# at nice 19 / ionice idle. (3) No new job while MemAvailable < 3 GB. (1) Games / emulators / Kodi:
# GPU jobs wait, CPU jobs run gently. (2) Something playing (MPRIS PlaybackStatus = Playing, e.g.
# YouTube in Firefox): jobs run gently, only long GPU soaks wait.
mem_kb=$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)
[ "${mem_kb:-0}" -ge 3145728 ] || exit 0
playing() {
  local n
  for n in $(busctl --user list --no-legend 2>/dev/null | awk '$1 ~ /^org\.mpris\.MediaPlayer2\./ {print $1}'); do
    busctl --user get-property "$n" /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player PlaybackStatus 2>/dev/null \
      | grep -q '"Playing"' && return 0
  done
  return 1
}
gpu=1
case "$kind" in test-*) gpu=0 ;; esac
gentle=0
if pgrep -x 'steam|eden|kodi|azahar' >/dev/null; then
  [ "$gpu" = 0 ] || exit 0
  gentle=1
elif playing; then
  case "$kind" in soak-*) exit 0 ;; esac
  gentle=1
fi
if [ "$gentle" = 1 ]; then build_jobs=8; else build_jobs=$(nproc); fi

case "$kind" in
  ci) limit=150m; cmd=(cargo xtask ci) ;;
  test-*) limit=60m; cmd=(cargo test -p "${kind#test-}") ;;
  intel) limit=60m; cmd=(bash fork/dell-ci/run-intel.sh) ;;
  soak-*) limit=6h; cmd=(bash "fork/dell-ci/soaks/${kind#soak-}.sh") ;;
  *) limit=1m; cmd=(false) ;;
esac

res="$CI/results/$job.txt"
log="$CI/results/$job.log"
wt="$CI/wt/$job"
commit=$(git -C "$REPO" rev-parse --short "$branch")
start=$(date +%s)
rm -rf -- "$wt"
git -C "$REPO" worktree prune
status=FAIL
if git -C "$REPO" worktree add --detach "$wt" "$branch" >"$log" 2>&1; then
  (
    cd "$wt" || exit 1
    export CARGO_TARGET_DIR="$CI/target" RUST_TEST_THREADS=4 CARGO_BUILD_JOBS="$build_jobs"
    timeout --kill-after=60 "$limit" nice -n 19 ionice -c 3 "${cmd[@]}"
  ) >>"$log" 2>&1
  rc=$?
  case $rc in 0) status=PASS ;; 124|137) status=TIMEOUT ;; *) status=FAIL ;; esac
  case "$kind" in ci|test-*|intel|soak-*) ;; *) status=FAIL; echo "unknown job kind: $kind" >>"$log" ;; esac
fi
dur=$(( $(date +%s) - start ))
first=-
if [ "$status" != PASS ]; then
  first=$(grep -m1 -E -- '^test .* FAILED$|^---- .* stdout ----|panicked at|^error(\[|:)|^FAIL|unknown job kind' "$log" | cut -c1-200)
  [ -n "$first" ] || first=$(tail -n 1 "$log" | cut -c1-200)
  [ -n "$first" ] || first=-
fi
tmp="$res.tmp"
printf '%s\n%s %dm%02ds\n%s\n' "$status" "$commit" $((dur / 60)) $((dur % 60)) "$first" >"$tmp" && mv -f "$tmp" "$res"
git -C "$REPO" worktree remove --force "$wt" >/dev/null 2>&1 || rm -rf -- "$wt"
git -C "$REPO" worktree prune
git -C "$REPO" branch -D "$branch" >/dev/null 2>&1
# keep results for two weeks
find "$CI/results" -type f -mtime +14 -delete 2>/dev/null
exit 0
