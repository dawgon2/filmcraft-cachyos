#!/usr/bin/env bash
# Fork Phase XPS (FORK-PLAN §1a.99 e): record what a hybrid laptop (Dell XPS 16: Intel Arc iGPU +
# NVIDIA RTX 4050) offers FilmCraft, in one report. Read-only: no sudo, installs nothing, changes
# no setting. Missing tools are noted, not fatal.
#
#   tools/fork/xps-probe.sh [report-file]     (default: target/xps-probe-<date>.txt)
#
# With FILMCRAFT_PROBE_TESTS=1 it also runs the NVENC / VA-API probe tests through
# ~/Apps/filmcraft-builds/fc-cargo.sh (slow the first time: it builds filmcraft-platform).
set -u
cd "$(dirname "$0")/../.." || exit 1
out=${1:-target/xps-probe-$(date +%Y%m%d-%H%M).txt}
mkdir -p "$(dirname "$out")"
exec > >(tee "$out") 2>&1

section() { printf '\n===== %s =====\n' "$1"; }
have() { command -v "$1" >/dev/null 2>&1; }
run() { if have "$1"; then timeout 60 "$@" 2>&1 | head -n 400; else echo "($1 not installed)"; fi; }

# The runtime power state of every DRM device: "suspended" means the GPU is asleep (runtime D3).
power_states() {
    for card in /sys/class/drm/card[0-9]*; do
        [ -e "$card/device" ] || continue
        case "$(basename "$card")" in *-*) continue ;; esac
        printf '%s vendor=%s device=%s driver=%s runtime_status=%s\n' "$(basename "$card")" \
            "$(cat "$card/device/vendor" 2>/dev/null)" "$(cat "$card/device/device" 2>/dev/null)" \
            "$(basename "$(readlink "$card/device/driver" 2>/dev/null)" 2>/dev/null)" \
            "$(cat "$card/device/power/runtime_status" 2>/dev/null || echo n/a)"
    done
}

section "when / what"
date; uname -srvm; run lsb_release -ds; echo "session: ${XDG_SESSION_TYPE:-?} ${XDG_CURRENT_DESKTOP:-?}"
section "CPU / RAM"
grep -m1 'model name' /proc/cpuinfo; grep -E 'MemTotal|MemAvailable' /proc/meminfo
section "GPUs (lspci) and render nodes"
run lspci -nnk -d ::0300; run lspci -nnk -d ::0302; run lspci -nnk -d ::0380
for n in /dev/dri/renderD*; do
    d=/sys/class/drm/$(basename "$n")/device
    echo "$n vendor=$(cat "$d/vendor" 2>/dev/null) device=$(cat "$d/device" 2>/dev/null) driver=$(basename "$(readlink "$d/driver" 2>/dev/null)" 2>/dev/null)"
done
section "GPU power states before anything below runs"
power_states
section "power supplies (what Lower playback resolution on battery reads)"
for p in /sys/class/power_supply/*; do
    [ -e "$p/type" ] && echo "$(basename "$p") type=$(cat "$p/type") online=$(cat "$p/online" 2>/dev/null || echo -) status=$(cat "$p/status" 2>/dev/null || echo -)"
done
section "kernel modules / drivers"
lsmod 2>/dev/null | grep -E '^(i915|xe|nvidia|nvidia_drm|nvidia_modeset|nvidia_uvm|nouveau|amdgpu) ' || echo "(none of the expected modules)"
cat /proc/driver/nvidia/version 2>/dev/null || echo "(no /proc/driver/nvidia)"
cat /sys/module/nvidia_drm/parameters/modeset 2>/dev/null | sed 's/^/nvidia_drm modeset=/'
section "environment overrides FilmCraft honours"
for v in DRI_PRIME __NV_PRIME_RENDER_OFFLOAD __GLX_VENDOR_LIBRARY_NAME MESA_VK_DEVICE_SELECT WGPU_POWER_PREF WGPU_ADAPTER_NAME LIBVA_DRIVER_NAME FILMCRAFT_VAAPI_DEVICE FILMCRAFT_NVDEC_FIRST VK_ICD_FILENAMES; do
    printf '%s=%s\n' "$v" "${!v-}"
done
section "VA-API per render node"
for n in /dev/dri/renderD*; do echo "--- $n"; run vainfo --display drm --device "$n"; done
section "Vulkan"
run vulkaninfo --summary
section "NVIDIA"
run nvidia-smi
run nvidia-smi -q -d CLOCK,POWER,PERFORMANCE,ENCODER_STATS,SUPPORTED_CLOCKS
section "displays / scaling"
run xrandr --listmonitors
run wlr-randr
run kscreen-doctor -o
section "GPU power states after the driver queries above (did any of them wake the dGPU?)"
power_states
if [ "${FILMCRAFT_PROBE_TESTS:-0}" = 1 ]; then
    section "FilmCraft probe tests (NVENC, VA-API encode, 4:2:2)"
    CARGO=${CARGO_WRAPPER:-$HOME/Apps/filmcraft-builds/fc-cargo.sh}
    RUST_TEST_THREADS=2 "$CARGO" test -p filmcraft-platform --test nvenc_hevc_probe --test nvenc --test hevc_export --test va_422_handoff -- --nocapture 2>&1 | grep -vE '^\s+(Compiling|Running|Finished)' | tail -n 120
fi
section "done"
echo "report: $out"
