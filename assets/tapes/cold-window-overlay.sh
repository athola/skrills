#!/usr/bin/env bash
# Burn timed subtitle captions into the cold-window TUI demo GIF.
#
# VHS cannot render captions while a full-screen TUI owns the terminal, so
# the in-TUI narration is overlaid here as a post-processing step. The tape
# (cold-window.tape) supplies the typed-comment intro; this script supplies
# the captions that play *over* the live surface.
#
# Rerun after every reshoot:  bash assets/tapes/cold-window-overlay.sh
# The caption windows below are keyed to the tape's Sleep timings; if you
# retune the tape, re-measure with: ffprobe -show_entries format=duration.
set -euo pipefail

GIF="${1:-assets/gifs/cold-window.gif}"
FONT="${FONT:-/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf}"
TMP="$(mktemp --suffix=.gif)"
trap 'rm -f "$TMP"' EXIT

[ -f "$GIF" ] || { echo "missing GIF: $GIF" >&2; exit 1; }
[ -f "$FONT" ] || { echo "missing font: $FONT" >&2; exit 1; }

# Caption schedule: "START END TEXT" (seconds). Windows sit inside each
# phase with gaps at the boundaries so no caption bleeds across a cut.
CAPTIONS=(
  "13.2 16.4 Live surface -- metrics refresh on every tick"
  "16.9 19.6 Token meter steps past the 2.0K ceiling -- WARNING fires"
  "20.0 23.8 ?  reveals the full keymap"
  "24.3 28.1 :  opens the command palette"
  "28.5 32.4 Tab cycles focus:  Alerts > Hints > Research"
  "32.8 36.6 Enter opens the alert governance detail"
  "37.5 39.5 Ctrl-C:  graceful shutdown within budget"
)

# Build the drawtext chain. Single quotes protect filtergraph-level
# separators (',' ';') but NOT drawtext's own option separator ':', which
# truncates a caption at its first colon. Escape backslashes then colons so
# captions like "Tab cycles focus: ..." render in full.
chain=""
for entry in "${CAPTIONS[@]}"; do
  start="${entry%% *}"; rest="${entry#* }"
  end="${rest%% *}"; text="${rest#* }"
  text="${text//\\/\\\\}"; text="${text//:/\\:}"
  chain+="drawtext=fontfile=${FONT}:text='${text}':fontcolor=white:fontsize=22:"
  chain+="box=1:boxcolor=0x1e1e2e@0.88:boxborderw=12:x=(w-text_w)/2:y=h-86:"
  chain+="enable='between(t,${start},${end})',"
done

# palettegen/paletteuse keeps the GIF crisp after the overlay pass.
ffmpeg -y -i "$GIF" -filter_complex \
  "[0:v]${chain}split[s0][s1];[s0]palettegen=stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3" \
  "$TMP" >/dev/null 2>&1

mv "$TMP" "$GIF"
trap - EXIT
echo "captions burned into $GIF"
