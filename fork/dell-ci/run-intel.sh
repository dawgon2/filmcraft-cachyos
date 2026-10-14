#!/usr/bin/env bash
# FORK-ONLY: DC1 intel job: runs each line of fork/dell-ci/intel.list ("<crate> [test filter / --test name ...]")
# as `cargo test -p <crate> <rest>` on the Dell's Intel GPU. Stops at the first failure.
set -u
status=0
while read -r crate rest; do
  case "$crate" in ''|'#'*) continue ;; esac
  echo "== cargo test -p $crate $rest"
  # shellcheck disable=SC2086
  cargo test -p "$crate" $rest || { status=1; break; }
done < fork/dell-ci/intel.list
exit $status
