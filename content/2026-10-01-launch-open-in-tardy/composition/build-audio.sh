#!/usr/bin/env bash
# Synthesizes the reel's original music bed with ffmpeg: no samples, no licensed music.
# 120 BPM (beat = 0.5s), A major add9 arpeggio over A / F#m / D / E, kick and hats from the
# drop at 4.0s, a noise riser into the drop, fade over the last second. Deterministic: rerun
# to rebuild assets/score/music.wav. Cut points in index.html sit on this grid.
set -euo pipefail
cd "$(dirname "$0")"
out=assets/score
mkdir -p "$out"
rm -f "$out"/*.wav
sr=48000
dur=20

# Chord root (semitones from A) changes every 2 bars (4s): A, F#, D, E.
root='if(eq(mod(floor(t/4),4),0),0,if(eq(mod(floor(t/4),4),1),-3,if(eq(mod(floor(t/4),4),2),-7,-5)))'
# Arp step pattern (8ths), no thirds so it fits every chord: 0 7 12 14 19 14 12 7.
step='mod(floor(t/0.25),8)'
semi="if(eq($step,0),0,if(eq($step,1),7,if(eq($step,2),12,if(eq($step,3),14,if(eq($step,4),19,if(eq($step,5),14,if(eq($step,6),12,7)))))))"
k='mod(t,0.5)'   # time since the last beat
drop='gte(t,4)*lt(t,19)'

arp="0.16*sin(2*PI*220*pow(2,($root+$semi)/12)*t)*exp(-11*mod(t,0.25))"
pad="0.07*(sin(2*PI*110*pow(2,$root/12)*t)+0.7*sin(2*PI*165*pow(2,$root/12)*t)+0.5*sin(2*PI*246.9*pow(2,$root/12)*t))*min(mod(t,4)/0.6,1)"
kick="$drop*0.9*sin(2*PI*(45*$k+(110/18)*(1-exp(-18*$k))))*exp(-9*$k)"
sub="$drop*0.22*sin(2*PI*55*pow(2,$root/12)*t)*(1-0.75*exp(-7*$k))"
hats="$drop*0.07*(random(0)*2-1)*exp(-70*mod(t-0.25,0.5))"
riser="between(t,2.6,3.98)*0.14*(random(1)*2-1)*pow(max(t-2.6,0)/1.4,2.5)"
fade="min(1,max(0,(20-t)/1.0))"

ffmpeg -v error -y -f lavfi -i "aevalsrc=exprs='($arp+$pad+$kick+$sub+$hats+$riser)*$fade':s=$sr:d=$dur" \
  -af 'acompressor=threshold=-16dB:ratio=3:attack=5:release=120,alimiter=limit=0.89' -ac 2 "$out/music.wav"
ls -la "$out"
