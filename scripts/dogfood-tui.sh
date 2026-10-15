#!/usr/bin/env bash
# dogfood-tui.sh — behavior-driven dogfood for the cold-window TUI's terminal
# behavior. `cargo test` runs without a controlling terminal, so the
# interactive surface (command palette, help overlay, focus model) never
# executes there. This harness allocates a real PTY via tmux, drives the
# binary with keystrokes, and asserts on the rendered frame — the user
# experience that only exists under a TTY.
#
# It also asserts the inverse contract: with stdout redirected (no TTY), the
# TUI surfaces must refuse gracefully instead of leaking alternate-screen
# escapes into a pipe (the regression that motivated the TTY guards).
#
# Why tmux: `tmux capture-pane -p` returns the visible frame as plain text,
# so there is no ANSI-stripping code to maintain or lint. tmux ships on the
# GitHub-hosted runners; without it the PTY scenarios SKIP (the no-TTY
# refusal scenarios still run).
#
# No errexit, by design: every scenario reports through ok/bad, and the
# summary decides the exit status.
set -uo pipefail

MYDIR="${0%"${0##*/}"}"
readonly MYDIR="${MYDIR:-./}"
# shellcheck source=scripts/logging.sh
. "${MYDIR%/}/logging.sh"

readonly OPTIONAL_DEPENDENCIES="tmux"
readonly COLS=120 ROWS=40
readonly SOCKET="skrills_df_${$}"
readonly SESS="cw"

readonly GREEN='\033[0;32m' RED='\033[0;31m' CYAN='\033[0;36m' DIM='\033[2m' NC='\033[0m'
PASS=0
FAIL=0

usage() {
  log "Usage: ${0##*/} [-h] [-x]"
  printf '  %-4s %s\n' \
    "-h" "Show this help and exit" \
    "-x" "Trace execution (set -x)"
  printf '  %s\n' \
    "Env:  BIN_PATH=path/to/skrills (default: target/release/skrills)" \
    "Exit: 0 pass (or PTY scenarios skipped), 1 a contract regressed," \
    "      2 setup error (binary missing)."
  exit 0
}

# Return 0 when every optional tool is present; log each missing one.
optcheck() {
  local utility missing=""
  for utility in ${OPTIONAL_DEPENDENCIES}; do
    command -v "${utility}" >/dev/null 2>&1 || missing="${missing:+"${missing} "}${utility}"
  done
  case "${missing}" in
    "") return 0 ;;
  esac
  for utility in ${missing}; do
    log 3 "${utility} not installed; its scenarios are skipped"
  done
  return 1
}

# Report lines go to stdout with printf: they are this harness's output.
feature() { printf "\n${CYAN}Feature:${NC} %s\n" "${1}"; }
scenario() { printf "  ${DIM}Scenario:${NC} %s\n" "${1}"; }
ok() {
  printf "    ${GREEN}PASS${NC} %s\n" "${1}"
  PASS=$((PASS + 1))
}
bad() {
  printf "    ${RED}FAIL${NC} %s\n" "${1}"
  FAIL=$((FAIL + 1))
  case "${2:-}" in
    "") ;;
    *) printf "      ${DIM}| %s${NC}\n" "${2}" ;;
  esac
}

tm() { tmux -L "${SOCKET}" "$@"; }

cleanup() {
  tm kill-server >/dev/null 2>&1 || true
  rm -rf "${WORK}"
}

pane() { tm capture-pane -p -t "${SESS}" 2>/dev/null; }

# wait_for <token> <timeout_s>: 0 found, 1 timeout, 2 session died early.
wait_for() {
  local token="${1}" end=$((SECONDS + ${2}))
  while [ "${SECONDS}" -lt "${end}" ]; do
    pane | grep -qF "${token}" && return 0
    tm has-session -t "${SESS}" 2>/dev/null || return 2
    sleep 0.3
  done
  return 1
}

# Drive cold-window --tui through a tmux PTY.
feature_tui_pty() {
  feature "cold-window --tui renders and responds to keys under a real terminal"

  unset TMUX # never nest inside an existing tmux server
  tm new-session -d -s "${SESS}" -x "${COLS}" -y "${ROWS}" \
    "env HOME='${HOME_DIR}' RUST_LOG=off '${BIN_PATH}' cold-window --tui \
       --skill-dir skills --tick-rate-ms 100 --alert-budget 100000; echo \$? > '${RCFILE}'"

  scenario "the dashboard reaches first paint (panes + hint bar visible)"
  local paint_rc=0
  wait_for help 12 || paint_rc=${?}
  case "${paint_rc}" in
    0)
      ok "hint bar painted ('? help' visible)"
      pane | grep -qF "Alerts" && ok "alert pane rendered (real TUI, not a prompt)" ||
        bad "no Alerts pane in frame" "$(pane | tr -s ' ' | head -1)"
      ;;
    2)
      bad "TUI process exited before first paint" "rc=$(cat "${RCFILE}" 2>/dev/null)"
      bad "alert pane unreachable (process gone)"
      ;;
    *)
      bad "TUI never reached first paint within budget" "$(pane | tr -s ' ' | tail -2)"
      bad "alert pane unreachable (no paint)"
      ;;
  esac

  scenario "pressing '?' opens the help overlay"
  tm send-keys -l -t "${SESS}" '?'
  if wait_for "Global" 6; then
    ok "help overlay shows keybinding sections (e.g. 'Global')"
  else bad "help overlay did not render section headers" "$(pane | tr -s ' ' | tail -2)"; fi

  scenario "pressing ':' opens the command palette"
  tm send-keys -l -t "${SESS}" '?' # toggle help closed (no Esc ambiguity)
  sleep 0.4
  tm send-keys -l -t "${SESS}" ':'
  if wait_for "Up/Down" 6; then
    ok "palette shows its contextual hint ('Up/Down select')"
  else bad "command palette hint did not appear" "$(pane | tr -s ' ' | tail -2)"; fi

  scenario "Ctrl-C quits cleanly and restores the terminal"
  tm send-keys -t "${SESS}" C-c
  end=$((SECONDS + 6))
  gone=1
  while [ "${SECONDS}" -lt "${end}" ]; do
    tm has-session -t "${SESS}" 2>/dev/null || {
      gone=0
      break
    }
    sleep 0.3
  done
  if [ "${gone}" -eq 0 ]; then
    ok "process quit on Ctrl-C"
  else
    bad "process still running after Ctrl-C"
    tm kill-session -t "${SESS}" 2>/dev/null || true
  fi
  rc="$(cat "${RCFILE}" 2>/dev/null || printf missing)"
  [ "${rc}" = "0" ] && ok "exited 0 (graceful shutdown)" || bad "exit code '${rc}' (expected 0)"
}

feature_tui_skipped() {
  feature "cold-window --tui renders and responds to keys under a real terminal"
  scenario "PTY scenarios skipped"
  printf "    ${DIM}SKIP tmux not installed; install tmux to dogfood the interactive TUI${NC}\n"
}

refusal() { # $1=label  $2..=command
  local label="${1}"
  shift
  scenario "${label} refuses gracefully when stdout is not a TTY"
  local out err rc
  local scratch
  scratch="$(mktemp -d "${WORK}/refusal.XXXXXX")"
  out="${scratch}/out"
  err="${scratch}/err"
  # No `set +e`/`set -e` pair: this script runs without errexit by design, and
  # every scenario reports through ok/bad. The old trailing `set -e` turned
  # errexit on for the rest of the run after the first refusal, so a later
  # failing check ended the script mid-feature with no summary.
  rc=0
  HOME="${HOME_DIR}" "$@" </dev/null >"${out}" 2>"${err}" || rc=${?}
  [ "${rc}" -ne 0 ] && ok "exited non-zero (${rc}) instead of rendering" ||
    bad "exited 0 — should refuse without a TTY"
  grep -qiE "tty|terminal" "${out}" "${err}" && ok "explains the TTY requirement" ||
    bad "no TTY explanation in output" "$(tail -1 "${err}")"
  # The cardinal sin: leaking alternate-screen control codes into a pipe.
  if grep -q $'\x1b\[?1049' "${out}"; then
    bad "leaked alternate-screen escapes into stdout"
  else ok "did not leak alternate-screen escapes into the pipe"; fi
}

main() {
  case "${1:-}" in
    *[uU][sS][aA][gG][eE] | *[hH][eE][lL][pP]) usage ;;
  esac
  local opt
  while getopts ":hx" opt; do
    case "${opt}" in
      h) usage ;;
      x) set -x ;;
      *)
        log 3 "Unknown flag: -${OPTARG}"
        exit 64
        ;;
    esac
  done

  REPO_ROOT="$(cd "${MYDIR%/}/.." && pwd)"
  BIN_PATH="${BIN_PATH:-target/release/skrills}"
  # Make relative paths repo-relative once, so an absolute BIN_PATH (make passes
  # one when CARGO_TARGET_DIR is set) is used as given rather than prefixed.
  case "${BIN_PATH}" in
    /*) ;;
    *) BIN_PATH="${REPO_ROOT}/${BIN_PATH}" ;;
  esac

  (
    cd "${REPO_ROOT}" || exit 2
    [ -x "${BIN_PATH}" ] || {
      log 3 "ERROR: binary not found at ${BIN_PATH} (run: make build)"
      exit 2
    }

    WORK="$(mktemp -d)"
    trap cleanup EXIT

    # A configured HOME so the interactive first-run setup prompt does not block
    # the TUI before it paints. (First-run behavior is dogfooded separately.)
    HOME_DIR="${WORK}/home"
    mkdir -p "${HOME_DIR}/.codex"
    printf '[mcp_servers.skrills]\ncommand = "skrills"\n' >"${HOME_DIR}/.codex/config.toml"
    RCFILE="${WORK}/rc"

    if optcheck; then
      feature_tui_pty
    else
      feature_tui_skipped
    fi

    feature "TTY-only surfaces refuse non-interactive use without leaking escapes"
    # No PTY needed: redirect stdout and assert a graceful, escape-free refusal.
    refusal "cold-window --tui" "${BIN_PATH}" cold-window --tui --tick-rate-ms 100 --alert-budget 100000
    refusal "tui" "${BIN_PATH}" tui --skill-dir skills
    refusal "dashboard" "${BIN_PATH}" dashboard --skill-dir skills

    printf '\n'
    banner "TUI dogfood: ${PASS} passed, ${FAIL} failed"
    case "${FAIL}" in
      0) exit 0 ;;
      *) exit 1 ;;
    esac
  )
}

main "$@"
