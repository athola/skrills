#!/usr/bin/env bash
# entrypoint.sh: run skrills validate and emit GitHub annotations.
# Expected env vars (set by action.yml):
#   INPUT_TARGETS: validation target (claude, codex, copilot, all, both)
#   INPUT_STRICT: "true" to fail on errors, "false" for annotations only
#   INPUT_PATH: skills directory path
set -euo pipefail

targets="${INPUT_TARGETS:-all}"
strict="${INPUT_STRICT:-true}"
skill_path="${INPUT_PATH:-skills/}"

# ---- sanity checks ---------------------------------------------------------
if ! command -v skrills >/dev/null 2>&1; then
  echo "::error::skrills binary not found on PATH. Check the install step."
  exit 1
fi

if [ ! -d "$skill_path" ]; then
  echo "::warning::Skills directory '${skill_path}' does not exist. Nothing to validate."
  {
    echo "total=0"
    echo "errors=0"
    echo "warnings=0"
  } >> "$GITHUB_OUTPUT"
  exit 0
fi

# ---- run validation (JSON output) ------------------------------------------
raw_out="${RUNNER_TEMP:-/tmp}/skrills-validate.raw"
json_out="${RUNNER_TEMP:-/tmp}/skrills-validate.json"
err_out="${RUNNER_TEMP:-/tmp}/skrills-validate.err"

# skrills emits tracing logs (e.g. "Skill discovery complete") onto stdout,
# interleaved with the JSON document jq parses below. RUST_LOG=off should
# silence them, but CI runners that export RUST_LOG can re-enable logging,
# so we ALSO strip the logs from the captured output (see below). Stderr
# goes to its own file so it never mixes into the JSON stream.
# Capture exit code; skrills validate currently always exits 0 but may change.
set +e
RUST_LOG=off skrills validate \
  --skill-dir "$skill_path" \
  --target "$targets" \
  --format json \
  > "$raw_out" 2> "$err_out"
validate_exit=$?
set -e

# Strip tracing log lines from stdout, leaving a clean JSON document.
# Pretty-printed JSON lines start with whitespace or a bracket; tracing
# logs start with a bare ISO-8601 timestamp ("2026-06-14T05:00:00Z ...").
# Dropping timestamp-led lines removes the logs wherever they land,
# regardless of whether RUST_LOG=off was honored.
grep -vE '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z' "$raw_out" > "$json_out" || true

# If the command itself failed (not validation errors, but a crash), bail out.
if [ $validate_exit -ne 0 ] && [ ! -s "$json_out" ]; then
  echo "::error::skrills validate exited with code ${validate_exit}"
  cat "$err_out" >&2 || true
  exit 1
fi

# ---- parse JSON and emit annotations ---------------------------------------
# Requires jq. GitHub-hosted runners include it; self-hosted may not.
if ! command -v jq >/dev/null 2>&1; then
  echo "::error::jq is required to parse validation output but was not found."
  exit 1
fi

# Defensive: if the stripped output still is not valid JSON, surface the
# raw output so the failure is debuggable instead of a bare jq error.
if ! jq empty "$json_out" >/dev/null 2>&1; then
  echo "::error::skrills validate did not produce valid JSON after log stripping."
  echo "----- raw validate output (first 20 lines) -----" >&2
  head -n 20 "$raw_out" >&2 || true
  exit 1
fi

total=$(jq 'length' "$json_out")
error_count=0
warning_count=0

# Iterate over each validation result.
for idx in $(seq 0 $(( total - 1 ))); do
  result=$(jq ".[$idx]" "$json_out")
  file=$(echo "$result" | jq -r '.path')
  name=$(echo "$result" | jq -r '.name')
  num_issues=$(echo "$result" | jq '.issues | length')

  for iidx in $(seq 0 $(( num_issues - 1 ))); do
    issue=$(echo "$result" | jq ".issues[$iidx]")
    severity=$(echo "$issue" | jq -r '.severity')
    message=$(echo "$issue" | jq -r '.message')
    line=$(echo "$issue" | jq -r '.line // empty')
    suggestion=$(echo "$issue" | jq -r '.suggestion // empty')
    target_name=$(echo "$issue" | jq -r '.target')

    # Build the full annotation message.
    ann_msg="[${target_name}] ${message}"
    if [ -n "$suggestion" ]; then
      ann_msg="${ann_msg} (suggestion: ${suggestion})"
    fi

    # Map severity to GitHub annotation level.
    case "$severity" in
      Error)
        level="error"
        error_count=$(( error_count + 1 ))
        ;;
      Warning)
        level="warning"
        warning_count=$(( warning_count + 1 ))
        ;;
      *)
        level="notice"
        ;;
    esac

    # Emit the annotation.
    if [ -n "$line" ]; then
      echo "::${level} file=${file},line=${line},title=skrills validate (${name})::${ann_msg}"
    else
      echo "::${level} file=${file},title=skrills validate (${name})::${ann_msg}"
    fi
  done
done

# ---- summary ----------------------------------------------------------------
{
  echo "total=${total}"
  echo "errors=${error_count}"
  echo "warnings=${warning_count}"
} >> "$GITHUB_OUTPUT"

echo "--- Validation Summary ---"
echo "Skills validated: ${total}"
echo "Errors:           ${error_count}"
echo "Warnings:         ${warning_count}"

if [ "$error_count" -gt 0 ] && [ "$strict" = "true" ]; then
  echo "::error::Validation failed with ${error_count} error(s) in strict mode."
  exit 1
fi

exit 0
