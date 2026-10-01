#!/usr/bin/env bash
# Deterministic gates for a tardy brag run. Prints one line per check and exits non-zero
# if any gate fails. Also extracts the fixed evidence set (stills + contact sheet) the
# rubric is scored from, so every grader looks at the same frames.
#
# Usage: check.sh <run-dir> <min-seconds> <max-seconds>
#   <run-dir> holds brag.mp4 and composition/. The window comes from the post-type skill.
set -uo pipefail

dir=${1:?usage: check.sh <run-dir> <min-seconds> <max-seconds>}
min=${2:?min seconds}
max=${3:?max seconds}
repo=$(git -C "$dir" rev-parse --show-toplevel)
theme="$repo/mobile/src/theme/index.ts"
video="$dir/brag.mp4"
fails=0

pass() { printf 'PASS  %-14s %s\n' "$1" "$2"; }
fail() { printf 'FAIL  %-14s %s\n' "$1" "$2"; fails=$((fails + 1)); }
warn() { printf 'WARN  %-14s %s\n' "$1" "$2"; }

[[ -f $video ]] || { fail video "missing $video"; exit 1; }

# --- Format: vertical 1080x1920, 30fps, has audio, inside the type's window ----------
IFS=, read -r w h fps < <(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height,r_frame_rate -of csv=p=0 "$video")
dur=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$video")
audio=$(ffprobe -v error -select_streams a -show_entries stream=codec_type -of csv=p=0 "$video")

[[ "$w"x"$h" == 1080x1920 ]] && pass format "${w}x${h}" || fail format "${w}x${h}, want 1080x1920"
[[ $fps == 30/1 || $fps == 60/1 ]] && pass fps "$fps" || fail fps "$fps, want 30/1 or 60/1"
[[ -n $audio ]] && pass audio "audio stream present" || fail audio "no audio stream"
if awk -v d="$dur" -v lo="$min" -v hi="$max" 'BEGIN { exit !(d >= lo - 0.1 && d <= hi + 0.1) }'; then
  pass duration "${dur}s in ${min}-${max}s"
else
  fail duration "${dur}s, want ${min}-${max}s"
fi

# --- Dead air: no black or silent stretch >= 0.5s anywhere ------------------------------
black=$(ffmpeg -hide_banner -nostats -i "$video" -vf blackdetect=d=0.5:pix_th=0.05 -an -f null - 2>&1 \
  | grep -o 'black_start:[0-9.]* black_end:[0-9.]*' || true)
[[ -z $black ]] && pass black "no black stretch >= 0.5s" || fail black "$(echo "$black" | tr '\n' ' ')"
if [[ -n $audio ]]; then
  # The outro may fade; only flag silence that starts before the last second.
  silence=$(ffmpeg -hide_banner -nostats -i "$video" -af silencedetect=n=-50dB:d=0.5 -vn -f null - 2>&1 \
    | grep -o 'silence_start: [0-9.]*' | awk -v d="$dur" '$2 < d - 1' || true)
  [[ -z $silence ]] && pass silence "no silent stretch >= 0.5s" || fail silence "$(echo "$silence" | tr '\n' ' ')"
fi

# --- Poster baked as frame 0 (brag step 4) ----------------------------------------------
if [[ -f $dir/brag.jpg ]]; then
  pass poster "brag.jpg present"
else
  fail poster "no brag.jpg poster"
fi

# --- HyperFrames: the composition must exist and pass `check` ---------------------------
comp="$dir/composition"
if [[ -f $comp/hyperframes.json || -f $comp/index.html && -f $comp/package.json ]]; then
  if (cd "$comp" && npx --yes hyperframes check >/dev/null 2>&1); then
    pass hyperframes "npx hyperframes check passes"
  else
    fail hyperframes "npx hyperframes check fails; run it in $comp"
  fi
else
  fail hyperframes "no HyperFrames composition in $comp (brag-slim or hand-rolled render?)"
fi

# --- Brand: every non-neutral color in the composition comes from the app theme ---------
# The theme file is the source of truth (docs/brand/BRAND.md says so); nothing is copied here.
hex_of() { grep -oiE '#[0-9a-f]{6}\b' "$@" 2>/dev/null | sed 's/.*#/#/' | tr a-f A-F | sort -u; }
palette=$(hex_of "$theme")
src=$(find "$dir/composition" "$dir" -maxdepth 3 \( -name '*.html' -o -name '*.css' -o -name 'frame.md' \) \
  -not -path '*/node_modules/*' 2>/dev/null | sort -u)
if [[ -z $src ]]; then
  warn brand "no composition sources to scan"
else
  off=()
  for c in $(hex_of $src); do
    grep -qx "$c" <<<"$palette" && continue
    r=$((16#${c:1:2})) g=$((16#${c:3:2})) b=$((16#${c:5:2}))
    hi=$(printf '%s\n' $r $g $b | sort -n | tail -1); lo=$(printf '%s\n' $r $g $b | sort -n | head -1)
    ((hi - lo <= 12)) && continue # greys and near-blacks are neutral
    off+=("$c")
  done
  if ((${#off[@]} == 0)); then
    pass brand "all accent colors are theme tokens"
  else
    fail brand "off-palette colors: ${off[*]} (theme: mobile/src/theme/index.ts)"
  fi
fi

# --- Evidence set: fixed stills every grader scores from --------------------------------
ev="$dir/grade"
mkdir -p "$ev/stills"
rm -f "$ev"/stills/*.png
for t in 0.5 1.0 2.0; do
  ffmpeg -v error -y -ss "$t" -i "$video" -frames:v 1 "$ev/stills/hook-${t}s.png"
done
for pct in 10 20 30 40 50 60 70 80 90 98; do
  t=$(awk -v d="$dur" -v p="$pct" 'BEGIN { printf "%.2f", d * p / 100 }')
  ffmpeg -v error -y -ss "$t" -i "$video" -frames:v 1 "$ev/stills/p${pct}-${t}s.png"
done
ffmpeg -v error -y -pattern_type glob -i "$ev/stills/p*.png" \
  -vf "scale=270:-1,tile=5x2:padding=8:color=0x0A0A0D" -frames:v 1 "$ev/contact-sheet.png"
pass evidence "$ev/contact-sheet.png + $(ls "$ev/stills" | wc -l | tr -d ' ') stills"

echo "sha256 $(shasum -a 256 "$video" | cut -d' ' -f1)"
echo "duration $dur"
((fails == 0)) && echo "GATES: PASS" || echo "GATES: FAIL ($fails)"
exit $((fails > 0))
