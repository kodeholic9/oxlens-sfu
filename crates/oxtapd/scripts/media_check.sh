#!/usr/bin/env bash
# author: kodeholic (powered by Claude)
# 재생기 호환 실측(context 202611/20261107a 미결) — ffmpeg 가 진짜 VP8 · H264 · Opus 를 짓고(해상도가 바뀌는 판 포함 — 시뮬캐스트 층 바뀜 흉내)
#   → oxpeer lib 시험이 RTP 로 쪼개 수신 정리층에 넣어 원본과 바이트 대조 → oxtapd 예제 mkvcheck 가 결과물 MkvSink 로 파일을 짓고
#   → ffprobe(꼴 · 코덱 · 크기) · ffmpeg 전부 디코드(오류 0)로 판정한다. ffmpeg(libvpx · libx264 · libopus) 가 있어야 돈다.
set -euo pipefail
cd "$(dirname "$0")/.."
D="$PWD/target/media-check"
rm -rf "$D" && mkdir -p "$D"
src() { ffmpeg -v error -f lavfi -i "testsrc2=size=$1:rate=30" -t 2 "${@:2}"; }
src 640x360 -c:v libvpx -deadline realtime -g 30 -b:v 800k "$D/vp8_a.ivf"
src 320x180 -c:v libvpx -deadline realtime -g 30 -b:v 300k "$D/vp8_b.ivf"
src 640x360 -c:v libx264 -profile:v baseline -tune zerolatency -g 30 -x264-params aud=1 -f h264 "$D/h264_a.h264"
src 320x180 -c:v libx264 -profile:v baseline -tune zerolatency -g 30 -x264-params aud=1 -f h264 "$D/h264_b.h264"
ffmpeg -v error -f lavfi -i "sine=frequency=440:sample_rate=48000" -t 2 -c:a libopus -frame_duration 20 -b:a 32k "$D/opus.ogg"

OXPEER_MEDIA_DIR="$D" cargo test --offline --manifest-path ../oxpeer/Cargo.toml --lib media_fixture -- --ignored --nocapture --test-threads 1
cargo run --offline -q --example mkvcheck -- "$D"

fail=0
for f in vp8 h264 opus; do
  m="$D/$f.mkv"
  echo "== $f.mkv"
  ffprobe -v error -show_entries stream=codec_name,width,height,sample_rate,channels -of compact "$m"
  err=$(ffmpeg -v error -i "$m" -f null - 2>&1 || true)
  n=$(ffprobe -v error -count_frames -select_streams 0 -show_entries stream=nb_read_frames -of csv=p=0 "$m")
  echo "   디코드 프레임 $n · 오류 줄 $(printf '%s' "$err" | grep -c . || true)"
  [ -n "$err" ] && { echo "$err" | head -5; fail=1; }
  dur=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$m")
  last=$(ffprobe -v error -select_streams 0 -show_entries frame=pts_time -of csv=p=0 "$m" | tail -1)
  echo "   길이 $dur · 마지막 프레임 $last"
  awk -v d="$dur" -v l="$last" 'BEGIN { exit !(d != "N/A" && d + 0 >= l + 0 && d - l < 0.1) }' || { echo "   ✗ 길이가 없거나 마지막 프레임과 안 맞는다"; fail=1; }
  if [ "$f" != opus ]; then
    # 시간 이동 — 찾기 표(Cues)로 3초 근처 키프레임부터 디코드가 시작돼야 한다
    first=$(ffprobe -v error -read_intervals 3%+#1 -select_streams v -show_entries frame=pts_time,key_frame -of csv=p=0 "$m" | head -1)
    echo "   3초로 이동 → 첫 프레임 (키, 시각) $first"
    awk -F, -v x="$first" 'BEGIN { split(x, a, ","); exit !(a[1] == 1 && a[2] + 0 >= 2.9 && a[2] + 0 <= 3.1) }' || { echo "   ✗ 시간 이동이 3초 키프레임에 닿지 않는다"; fail=1; }
    sizes=$(ffprobe -v error -select_streams v -show_entries frame=width,height -of csv=p=0 "$m" | uniq | tr '\n' ' ')
    echo "   화면 크기 차례: $sizes"
    [ "$(echo "$sizes" | wc -w)" -ge 2 ] || { echo "   ✗ 해상도 바뀜이 디코드에 안 보인다"; fail=1; }
  fi
done
# Matroska 사람마다 영상 · 음성 한 파일(202611/20261110e) — 같은 표본(H264 해상도 바뀜 + Opus)을 MkvSink 한 묶음으로 → 두 스트림 · 끝까지 디코드
m="$D/av.mkv"
echo "== av.mkv"
ffprobe -v error -show_entries stream=codec_name,width,height,sample_rate -of compact "$m"
err=$(ffmpeg -v error -i "$m" -f null - 2>&1 || true)
v=$(ffprobe -v error -count_frames -select_streams v -show_entries stream=nb_read_frames -of csv=p=0 "$m")
a=$(ffprobe -v error -count_frames -select_streams a -show_entries stream=nb_read_frames -of csv=p=0 "$m")
echo "   영상 $v · 음성 $a · 오류 줄 $(printf '%s' "$err" | grep -c . || true)"
[ -n "$err" ] && { echo "$err" | head -3; fail=1; }
[ "${v:-0}" -eq 120 ] && [ "${a:-0}" -eq 101 ] || { echo "   ✗ 한 파일에 영상 120 · 음성 101 이 아니다"; fail=1; }
echo "== media_check $fail"
exit $fail
