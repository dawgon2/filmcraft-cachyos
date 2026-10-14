#!/bin/bash
# AUDIO-PLAN A1 acceptance soak (fork): plays the desktop app's real audio output for a long run
# into a PipeWire / PulseAudio null sink (nothing is heard), removes and restores that sink and
# switches the first sound card's profile off and back mid-run, then restores everything.
# See apps/filmcraft/src/audio_soak.rs. Linux; needs pactl; never needs root.
#   tools/fork/a1-audio-soak.sh [seconds=1800] [report.json]
set -u
export LC_ALL=C # pactl's "Active Profile:" is localized
RUN_SECONDS=${1:-1800}
REPORT=${2:-/tmp/fc-a1-soak.json}
SINK=fc_a1_test
CARD=${FC_SOAK_CARD:-$(pactl list short cards | awk 'NR==1{print $2}')}
PROFILE=$(pactl list cards | awk -v c="$CARD" '$1=="Name:"{n=$2} n==c && /Active Profile:/{sub(/.*Active Profile: /,""); print; exit}')
cleanup() {
    # the test binary first (cargo doesn't always pass a signal on), then restore
    pkill -f 'deps/filmcraft-[0-9a-f]* .*audio_soak' 2>/dev/null
    if [ -n "$PROFILE" ] && [ -n "$CARD" ]; then pactl set-card-profile "$CARD" "$PROFILE"; fi
    for id in $(pactl list short modules | awk -v s="sink_name=$SINK" '/module-null-sink/ {for (i = 1; i <= NF; i++) if ($i == s) print $1}'); do
        pactl unload-module "$id"
    done
}
trap cleanup EXIT
trap 'kill "$TEST_PID" 2>/dev/null; exit 143' TERM
trap 'kill "$TEST_PID" 2>/dev/null; exit 130' INT HUP
TEST_PID=
if pactl list short sinks | awk '{print $2}' | grep -qx "$SINK"; then
    echo "a sink named $SINK already exists; remove it first" >&2
    trap - EXIT
    exit 1
fi
pactl load-module module-null-sink "sink_name=$SINK" "sink_properties=device.description=$SINK" >/dev/null || exit 1
echo "soak: ${RUN_SECONDS}s on $SINK; card $CARD profile '$PROFILE' (restored at the end)"
FC_SOAK_SECONDS=$RUN_SECONDS FC_SOAK_DEVICE=$SINK FC_SOAK_NULL_SINK=$SINK FC_SOAK_CARD=$CARD FC_SOAK_PROFILE=$PROFILE \
    FC_SOAK_REPORT=$REPORT PULSE_SINK=$SINK \
    cargo test --release -p filmcraft --bin filmcraft -- --ignored --nocapture --test-threads=1 audio_soak &
# (in the background, so a TERM / INT is handled at once: the card profile is always restored)
TEST_PID=$!
wait "$TEST_PID"
