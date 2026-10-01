#!/usr/bin/env bash
# Synthesizes the reel's original trailer score with ffmpeg: no samples, no licensed music.
# Deterministic: rerun to rebuild every file in assets/score/. Timing lives in index.html.
set -euo pipefail
cd "$(dirname "$0")"
out=assets/score
mkdir -p "$out"
sr=48000
gen() { ffmpeg -v error -y -f lavfi -i "aevalsrc=exprs='$2':s=$sr:d=$3" "${@:4}" -ac 2 "$out/$1.wav"; }

# Sub hit: a pitch-dropping boom (90 -> 35 Hz) with a short noise transient.
gen hit '0.95*sin(2*PI*(35*t+(55/6)*(1-exp(-6*t))))*exp(-2.6*t)+0.35*(random(0)*2-1)*exp(-35*t)' 1.8 \
  -af 'lowpass=f=2400,volume=1.4'

# Riser: a sine sweep 180 -> 1400 Hz plus noise, both swelling to a hard stop.
gen riser '(0.35*sin(2*PI*(180*t+(1220/3.6)*t*t))+0.25*(random(0)*2-1))*pow(t/1.8,2.2)' 1.8 \
  -af 'highpass=f=150,volume=1.2'

# Braam: a detuned, saw-like brass stack (A1, E2, A2) through a low-pass, hard attack, long decay.
saw() { local f=$1 e=""; for k in 1 2 3 4 5 6 7 8; do e+="+sin(2*PI*$k*$f*t)/$k"; done; echo "(0$e)"; }
gen braam "0.22*($(saw 55)+$(saw 55.4)+0.8*$(saw 82.4)+0.6*$(saw 110.3))*min(t/0.04,1)*exp(-0.75*t)" 3.6 \
  -af 'lowpass=f=950,acompressor=threshold=-14dB:ratio=3,volume=1.1'

# Drone: E1/B1/E2 sines with a slow swell, under the whole reel (the timeline gates its level).
gen drone '(0.5*sin(2*PI*41.2*t)+0.35*sin(2*PI*61.7*t)+0.25*sin(2*PI*82.4*t))*(0.8+0.2*sin(2*PI*0.25*t))' 23 \
  -af 'volume=0.9'

ls -la "$out"
