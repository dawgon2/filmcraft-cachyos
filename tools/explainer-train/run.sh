#!/usr/bin/env bash
# One training session for FilmCraft's node explainer (GRADE-PLAN G2.11b; docs/fork/TRAINING.md):
#   generate -> train -> quantize -> eval -> score
#
#   tools/explainer-train/run.sh <base> <name>
#     <base>  smol360 (SmolLM2-360M-Instruct, Q8_0) | qwen05 (Qwen2.5-0.5B-Instruct, Q8_0 + Q4_K_M)
#     <name>  run name, e.g. smol360-v2
#
# Environment (all optional):
#   STAGES="generate train quantize eval score"  stages to run (default: all)
#   DATA=<dir>             training data folder (default: $RUNS/data-v1; generated if missing)
#   GRADES=2600 PER_GRADE=2 SEED=1   generator settings
#   TRAIN_ARGS="--epochs 2"          extra arguments for train.py (see train.py --help)
#   DEVICE=cuda|cpu        cpu = the slow fallback (no GPU)
#   REPEAT_PENALTY=1.0     eval generation setting (tuned models run without a penalty)
#   MODELS=~/.local/share/filmcraft-fork/models   FilmCraft's models folder
#   RUNS=~/Apps/filmcraft-train/runs              run folders (data, logs, eval outputs)
#   VENV=~/Apps/filmcraft-train/.venv             the trainer's Python environment
#   CARGO_TARGET_DIR       as usual
#
# Offline tooling only: nothing here is part of FilmCraft or bundled with it.
set -euo pipefail

base="${1:-}"
name="${2:-}"
if [[ -z "$base" || -z "$name" ]]; then
  sed -n '2,24p' "$0"
  exit 2
fi

repo="$(cd "$(dirname "$0")/../.." && pwd)"
MODELS="${MODELS:-$HOME/.local/share/filmcraft-fork/models}"
RUNS="${RUNS:-$HOME/Apps/filmcraft-train/runs}"
VENV="${VENV:-$HOME/Apps/filmcraft-train/.venv}"
DATA="${DATA:-$RUNS/data-v1}"
STAGES="${STAGES:-generate train quantize eval score}"
DEVICE="${DEVICE:-cuda}"
GRADES="${GRADES:-2600}"
PER_GRADE="${PER_GRADE:-2}"
SEED="${SEED:-1}"

case "$base" in
  smol360) base_dir="$MODELS/llm-base/SmolLM2-360M-Instruct"; family=llama; quants="q8_0" ;;
  qwen05)  base_dir="$MODELS/llm-base/Qwen2.5-0.5B-Instruct"; family=qwen2; quants="q8_0 q4k" ;;
  *) echo "unknown base $base (smol360 or qwen05)" >&2; exit 2 ;;
esac

run="$RUNS/$name"
tuned="$MODELS/tuned/$name"
mkdir -p "$run"
log() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$run/session.log"; }
has() { [[ " $STAGES " == *" $1 "* ]]; }
cargo_bg() { nice -n 19 ionice -c 3 cargo "$@"; }
timed() {
  local stage="$1"; shift
  local t0=$SECONDS
  "$@"
  log "$stage took $((SECONDS - t0)) s"
}

cd "$repo"
log "session $name: base $base ($base_dir), data $DATA, stages: $STAGES"

if has generate; then
  if [[ -f "$DATA/train.jsonl" ]]; then
    log "generate: $DATA exists, kept"
  else
    timed generate cargo_bg run --release -p filmcraft-render --example explain_traindata -- \
      --out-dir "$DATA" --grades "$GRADES" --per-grade "$PER_GRADE" --seed "$SEED"
  fi
fi

if has train; then
  # shellcheck disable=SC2086
  timed train env HSA_OVERRIDE_GFX_VERSION="${HSA_OVERRIDE_GFX_VERSION:-10.3.0}" \
    PYTORCH_HIP_ALLOC_CONF="${PYTORCH_HIP_ALLOC_CONF:-expandable_segments:True}" \
    nice -n 10 "$VENV/bin/python" "$repo/tools/explainer-train/train.py" \
      --base "$base_dir" --data "$DATA" --run-dir "$run" --out "$tuned" --device "$DEVICE" ${TRAIN_ARGS:-} \
      2>&1 | grep --line-buffered -v -e 'amdgpu.ids' | tee "$run/train.log"
fi

ggufs=()
for q in $quants; do ggufs+=("$tuned/$name-$q.gguf"); done

if has quantize; then
  for q in $quants; do
    timed "quantize $q" cargo_bg run --release -p filmcraft-llm --features infer --example llm_quantize -- \
      "$tuned" "$tuned/$name-$q.gguf" --type "$q" --name "filmcraft-explainer-$name" | tee -a "$run/quantize.log"
  done
fi

if has eval; then
  cargo_bg run --release -p filmcraft-render --example explain_score -- --dump-cases "$run/cases.jsonl"
  locals=()
  for q in $quants; do locals+=(--local "$name-$q=$tuned/$name-$q.gguf:$family"); done
  export FILMCRAFT_LLM_DIR="$MODELS"
  timed eval cargo_bg run --release -p filmcraft-llm --features infer --example llm_eval -- \
    "$run/cases.jsonl" "${locals[@]}" --repeat-penalty "${REPEAT_PENALTY:-1.0}" --out "$run/eval.jsonl" 2> "$run/eval.log" | tee "$run/eval.summary"
fi

if has score; then
  cargo_bg run --release -p filmcraft-render --example explain_score -- "$run/eval.jsonl" \
    --summary "$run/score.md" --json "$run/score.json" --fails > /dev/null
  head -n 6 "$run/score.md" | tee -a "$run/session.log"
fi
log "done: weights in $tuned, logs and scores in $run"
