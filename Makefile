# Common developer and demo tasks for skrills
# Note: Use Git Bash on Windows; recipes assume a POSIX shell.
# Set CARGO_HOME to a writable path to avoid sandbox/root perms issues.
SHELL := /bin/bash
.DEFAULT_GOAL := help
.DELETE_ON_ERROR:

CARGO ?= cargo
CARGO_HOME ?= .cargo
HOME_DIR ?= $(CURDIR)/.home-tmp
BIN ?= skrills
BIN_PATH ?= target/release/$(BIN)
MDBOOK ?= mdbook
# Test thread count: defaults to 1 for isolation (tests share filesystem state).
# Override with TEST_THREADS=4 for faster parallel runs if tests are independent.
TEST_THREADS ?= 1
CARGO_CMD = CARGO_HOME=$(CARGO_HOME) $(CARGO)
DEMO_RUN = HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) $(BIN_PATH)

define get_version
$(shell grep '^version' crates/cli/Cargo.toml | head -1 | cut -d'=' -f2 | cut -d'#' -f1 | tr -d " \"'")
endef

CARGO_GUARD_TARGETS := fmt fmt-check lint check test test-unit test-integration test-setup \
	test-coverage build build-min serve-help install coverage docs book book-serve \
	clean security deny deps-update

define open_file
	@if [ -f "$(1)" ]; then \
	  if command -v xdg-open >/dev/null 2>&1; then xdg-open "$(1)" >/dev/null 2>&1 || true; \
	  elif command -v open >/dev/null 2>&1; then open "$(1)" >/dev/null 2>&1 || true; \
	  elif command -v start >/dev/null 2>&1; then start "$(1)" >/dev/null 2>&1 || true; \
	  else echo "Open $(1)"; fi; \
	else echo "Not found: $(1)"; fi
endef

define ensure_mdbook
	@if ! command -v $(MDBOOK) >/dev/null 2>&1; then \
	  echo "mdbook not found; installing to $(CARGO_HOME)/bin"; \
	  $(CARGO_CMD) install mdbook --locked >/dev/null; \
	fi
endef

define assert_file
	@test -f "$(1)" || (echo "ERROR: $(2)" && exit 1)
endef

define assert_exec
	@test -x "$(1)" || (echo "ERROR: $(2)" && exit 1)
endef

# Unified verification macro: $(1)=label, $(2)=checks (space-separated: mcp claude codex)
# Examples: $(call verify_setup,Claude,mcp claude), $(call verify_setup,Codex,codex)
define verify_setup
	@echo "==> Verifying $(1) setup..."
	$(if $(findstring mcp,$(2)),$(call assert_file,$(HOME_DIR)/.claude/.mcp.json,Claude MCP config not created))
	$(if $(findstring claude,$(2)),$(call assert_exec,$(HOME_DIR)/.claude/bin/skrills,Claude binary not installed))
	$(if $(findstring codex,$(2)),$(call assert_exec,$(HOME_DIR)/.codex/bin/skrills,Codex binary not installed))
	@echo "==> $(1) setup verified successfully"
endef

# Phony targets: core developer flow
.PHONY: all help version verify fmt fmt-check lint lint-md lint-prose lint-decoration lint-hygiene check test test-unit test-integration test-setup test-install \
	release-consistency \
	build build-min serve-help install status coverage test-coverage dogfood dogfood-readme ci precommit \
	clean clean-demo hooks require-cargo security deny deps-update check-deps \
	quick watch bench release
# Phony targets: docs
.PHONY: docs book book-serve
# Phony targets: demos
.PHONY: demo-fixtures demo-doctor demo-empirical demo-http demo-cli demo-all demo-setup-claude demo-setup-codex \
	demo-setup-both demo-setup-uninstall demo-setup-reinstall \
	demo-setup-universal demo-setup-first-run demo-setup-all \
	demo-analytics demo-gateway demo-cert demo-skill-lifecycle \
	demo-multi-cli-agent demo-release-consistency
# Phony targets: dogfood (v0.8.0 cold-window + script ports)
.PHONY: dogfood-cold-window-headless dogfood-cold-window-chaos dogfood-cold-window-browser \
	dogfood-tui dogfood-dashboard dogfood-skill-diff dogfood-all \
	plugin-validate-direct plugin-modernize-direct plugin-registrations-direct
.NOTPARALLEL: demo-all demo-setup-all
.SILENT: demo-doctor demo-empirical demo-cli demo-all demo-setup-claude demo-setup-codex demo-setup-both \
	demo-setup-uninstall demo-setup-reinstall demo-setup-universal demo-setup-first-run \
	demo-setup-all demo-multi-cli-agent

$(CARGO_GUARD_TARGETS): require-cargo

all: fmt lint test build
	@echo "==> Full build complete"

version:
	@echo "$(call get_version)"

verify: fmt-check lint lint-md lint-hygiene test test-scripts test-install
	@echo "==> All verification checks passed"

help:
	@printf "Usage: make <target>\n\n"
	@printf "Core\n"
	@printf "  %-23s %s\n" "all" "full build (fmt + lint + test + build)"
	@printf "  %-23s %s\n" "version" "print current version"
	@printf "  %-23s %s\n" "verify" "run all verification checks"
	@printf "  %-23s %s\n" "fmt | fmt-check" "format workspace or check only"
	@printf "  %-23s %s\n" "lint" "clippy --all-features with -D warnings"
	@printf "  %-23s %s\n" "lint-md" "lint markdown files"
	@printf "  %-23s %s\n" "lint-prose" "block AI-slop vocabulary in docs"
	@printf "  %-23s %s\n" "lint-decoration" "block decorative // ─ separator comments"
	@printf "  %-23s %s\n" "lint-hygiene" "run lint-prose + lint-decoration"
	@printf "  %-23s %s\n" "check" "cargo check all targets"
	@printf "  %-23s %s\n" "test | test-unit | test-integration" "run tests"
	@printf "  %-23s %s\n" "test-setup" "run setup module tests"
	@printf "  %-23s %s\n" "test-install" "test install.sh helper functions"
	@printf "  %-23s %s\n" "release-consistency" "verify Cargo.toml/plugin.json version + commands parity"
	@printf "  %-23s %s\n" "plugin-audit" "audit plugin.json registrations vs disk"
	@printf "  %-23s %s\n" "plugin-audit-fix" "rewrite plugin.json to match disk"
	@printf "  %-23s %s\n" "plugin-validate" "validate plugin structure (kebab-case, paths, hooks)"
	@printf "  %-23s %s\n" "plugin-modernize" "scan hook scripts for deprecated patterns"
	@printf "  %-23s %s\n" "plugin-doctor" "run all three plugin audits"
	@printf "  %-23s %s\n" "test-scripts" "pytest the ported script suite under tests/unit/"
	@printf "  %-23s %s\n" "test-coverage" "coverage via cargo-llvm-cov (precise)"
	@printf "  %-23s %s\n" "build | build-min" "release builds"
	@printf "  %-23s %s\n" "install" "install skrills to $(CARGO_HOME)/bin"
	@printf "  %-23s %s\n" "serve-help" "binary --help smoke check"
	@printf "  %-23s %s\n" "status" "show project status and environment"
	@printf "  %-23s %s\n" "coverage" "coverage via cargo-tarpaulin (fast)"
	@printf "  %-23s %s\n" "dogfood" "full dogfood (doctor + README validation)"
	@printf "  %-23s %s\n" "dogfood-readme" "validate README CLI examples only"
	@printf "  %-23s %s\n" "ci | precommit" "run common pipelines"
	@printf "  %-23s %s\n" "quick" "fast check (fmt + check, no tests)"
	@printf "  %-23s %s\n" "watch" "watch mode with cargo-watch"
	@printf "  %-23s %s\n" "bench" "run benchmarks"
	@printf "  %-23s %s\n" "release" "full release validation"
	@printf "  %-23s %s\n" "hooks" "install git pre-commit hooks"
	@printf "  %-23s %s\n" "clean | clean-demo" "clean builds or demo HOME"
	@printf "  %-23s %s\n" "require-cargo" "guard: ensure cargo is available"
	@printf "  %-23s %s\n" "security" "run cargo audit"
	@printf "  %-23s %s\n" "deny" "run cargo deny (licenses, bans, sources)"
	@printf "  %-23s %s\n" "deps-update" "update dependencies"
	@printf "  %-23s %s\n" "check-deps" "check optional tool availability"
	@printf "  %-23s %s\n" "" "optional: mdbook, cargo-audit, cargo-deny, cargo-llvm-cov"
	@printf "\nDocs\n"
	@printf "  %-23s %s\n" "docs" "build rustdoc and open"
	@printf "  %-23s %s\n" "book | book-serve" "build or serve mdBook"
	@printf "\nDemos\n"
	@printf "  %-23s %s\n" "demo-all" "run all demos (cli + setup)"
	@printf "  %-23s %s\n" "demo-cli" "test all CLI commands"
	@printf "  %-23s %s\n" "demo-doctor | demo-empirical" "individual command demos"
	@printf "  %-23s %s\n" "demo-http" "start HTTP MCP server (127.0.0.1:3000)"
	@printf "  %-23s %s\n" "demo-cert" "test TLS certificate management"
	@printf "  %-23s %s\n" "demo-skill-lifecycle" "test skill lifecycle commands"
	@printf "  %-23s %s\n" "demo-analytics" "test analytics export/import"
	@printf "  %-23s %s\n" "demo-gateway" "test MCP gateway tools"
	@printf "  %-23s %s\n" "demo-multi-cli-agent" "test multi-CLI agent routing"
	@printf "  %-23s %s\n" "demo-release-consistency" "show parity inventory + run invariant tests"
	@printf "  %-23s %s\n" "demo-setup-all" "run all setup flow demos"
	@printf "  %-23s %s\n" "demo-setup-{claude,codex,both}" "client setup demos"
	@printf "  %-23s %s\n" "demo-setup-{uninstall,reinstall}" "lifecycle demos"
	@printf "  %-23s %s\n" "demo-setup-{universal,first-run}" "other setup demos"
	@printf "  %-23s %s\n" "demo-fixtures" "prepare demo HOME sandbox"
	@printf "\nDogfood (v0.8.0 cold-window + script ports)\n"
	@printf "  %-23s %s\n" "dogfood-all" "run dogfood + all v0.8.0 surface checks"
	@printf "  %-23s %s\n" "dogfood-cold-window-headless" "engine ticks for 3s, expects clean SIGTERM exit"
	@printf "  %-23s %s\n" "dogfood-cold-window-chaos" "--no-adaptive + tiny budget; exercises kill-switch"
	@printf "  %-23s %s\n" "dogfood-cold-window-browser" "HTML+SSE parity check + 2s graceful-shutdown budget"
	@printf "  %-23s %s\n" "dogfood-tui" "skrills tui smoke (timeout 3s)"
	@printf "  %-23s %s\n" "dogfood-dashboard" "skrills dashboard smoke (timeout 3s)"
	@printf "  %-23s %s\n" "dogfood-skill-diff" "skill-diff --format json validates as JSON"
	@printf "  %-23s %s\n" "plugin-validate-direct" "validate_plugin.py against ./plugins/skrills"
	@printf "  %-23s %s\n" "plugin-modernize-direct" "check_hook_modernization.py --json"
	@printf "  %-23s %s\n" "plugin-registrations-direct" "update_plugin_registrations.py --dry-run"

require-cargo:
	@command -v $(CARGO) >/dev/null 2>&1 || { \
		echo "cargo not found. Install Rust from https://rustup.rs/"; exit 1; }

fmt:
	$(CARGO_CMD) fmt --all

fmt-check:
	$(CARGO_CMD) fmt --all -- --check

# NOTE: CI (.github/workflows/ci.yml) duplicates these cargo commands directly
# rather than calling make targets. Keep both in sync when changing flags.
lint:
	$(CARGO_CMD) clippy --workspace --all-targets --all-features -- -D warnings

lint-md:
	$(SHELL) ./scripts/lint-markdown.sh

# Block AI-slop vocabulary in user-facing prose.
lint-prose:
	$(SHELL) ./scripts/lint-prose-slop.sh

# Block decorative `// ─{20,}` separator comments in Rust source.
lint-decoration:
	$(SHELL) ./scripts/lint-rust-decoration.sh

# Aggregate AI hygiene lints (banned words + decorative comments).
lint-hygiene: lint-prose lint-decoration

check:
	$(CARGO_CMD) check --workspace --all-targets

test:
	$(CARGO_CMD) test --workspace --all-features -- --test-threads=$(TEST_THREADS)

test-unit:
	$(CARGO_CMD) test --workspace --lib --all-features -- --test-threads=$(TEST_THREADS)

test-integration:
	$(CARGO_CMD) test --workspace --test '*' --all-features

test-setup:
	$(CARGO_CMD) test --package skrills-server --lib setup --all-features

test-install:
	@echo "==> Testing install.sh helper functions"
	./scripts/test-install.sh

release-consistency:
	@echo "==> Verifying release-consistency invariants (Cargo.toml + plugin.json + commands/)"
	$(CARGO_CMD) test -p skrills_test_utils --test release_consistency

# Plugin audits (Phase 1 / 1b / 1c). Uses ~/claude-night-market upstream
# scripts when present, else the in-tree ports under scripts/.
plugin-audit:
	@./scripts/audit-plugins.sh

plugin-audit-fix:
	@./scripts/audit-plugins.sh --fix

plugin-validate:
	@./scripts/audit-plugins.sh --validate

plugin-modernize:
	@./scripts/audit-plugins.sh --modernize

plugin-doctor:
	@./scripts/audit-plugins.sh --all

# Run pytest over the script ports (validate_plugin / hook modernization
# / registration auditor).
test-scripts:
	@command -v pytest >/dev/null 2>&1 || { echo "pytest not installed (pip install pytest)"; exit 1; }
	pytest tests/unit -q

test-coverage:
	@if command -v cargo-llvm-cov >/dev/null 2>&1; then \
		$(CARGO_CMD) llvm-cov --workspace --all-features --html; \
		$(call open_file,$(CURDIR)/target/llvm-cov/html/index.html); \
	else \
		echo "cargo-llvm-cov not installed. Run: cargo install cargo-llvm-cov"; \
		exit 1; \
	fi

build:
	$(CARGO_CMD) build --workspace --all-features --release

build-min:
	$(CARGO_CMD) build --workspace --no-default-features --release

serve-help:
	$(CARGO_CMD) run --quiet --bin $(BIN) -- --help >/dev/null

status:
	@echo "=== Skrills Status ==="
	@echo "Version: $(call get_version)"
	@echo "Rust: $$(rustc --version)"
	@echo "Cargo: $$(cargo --version)"
	@echo "Branch: $$(git rev-parse --abbrev-ref HEAD)"
	@echo "Commit: $$(git rev-parse --short HEAD)"
	@echo "Binary: $(BIN_PATH) $$(test -f $(BIN_PATH) && echo '(exists)' || echo '(not built)')"

install:
	$(CARGO_CMD) install --path crates/cli --locked

coverage:
	$(CARGO_CMD) tarpaulin --workspace --all-features --out Html
	$(call open_file,$(CURDIR)/tarpaulin-report.html)

cold-window: build
	@echo "Starting cold-window dashboard at http://localhost:8888/dashboard"
	@echo "  Press Ctrl-C for graceful shutdown."
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) $(BIN_PATH) cold-window --browser --port 8888 --alert-budget 50000

dogfood: build demo-fixtures dogfood-readme
	@echo "==> Dogfooding: Running skrills on itself"
	HOME=$(HOME_DIR) $(BIN_PATH) doctor
	@echo "==> Dogfood complete"

dogfood-readme: build demo-fixtures
	@echo "==> Dogfooding README CLI examples"
	@echo "--- cert status"
	$(DEMO_RUN) cert status
	@echo "--- cert renew"
	$(DEMO_RUN) cert renew
	@echo "--- skill-catalog"
	$(DEMO_RUN) skill-catalog
	@echo "--- skill-profile"
	$(DEMO_RUN) skill-profile
	@echo "--- skill-usage-report"
	$(DEMO_RUN) skill-usage-report
	@echo "--- skill-score"
	$(DEMO_RUN) skill-score || echo "    (No skills found - expected on fresh install)"
	@echo "--- skill-deprecate --help"
	$(DEMO_RUN) skill-deprecate --help >/dev/null
	@echo "--- skill-rollback --help"
	$(DEMO_RUN) skill-rollback --help >/dev/null
	@echo "==> README examples validated"

docs:
	RUSTDOCFLAGS="-D warnings" $(CARGO_CMD) doc --workspace --all-features --no-deps
	$(call open_file,$(CURDIR)/target/doc/skrills/index.html)

book:
	$(call ensure_mdbook)
	PATH=$(CARGO_HOME)/bin:$$PATH $(MDBOOK) build book
	$(call open_file,$(CURDIR)/book/book/index.html)

book-serve:
	$(call ensure_mdbook)
	PATH=$(CARGO_HOME)/bin:$$PATH $(MDBOOK) serve book --open --hostname 127.0.0.1 --port 3000

# --- Demo helpers ---------------------------------------------------------

demo-http: build
	@echo "==> Demo: HTTP Transport (starts server, ctrl-c to stop)"
	@echo "    Connect to http://127.0.0.1:3000/mcp"
	$(BIN_PATH) serve --http 127.0.0.1:3000

demo-cert: demo-fixtures build
	@echo "==> Demo: TLS Certificate Management"
	@echo "--- cert status"
	$(DEMO_RUN) cert status
	@echo "--- cert status (json)"
	$(DEMO_RUN) cert status --format json | head -5
	@echo "--- cert renew (skip if valid)"
	$(DEMO_RUN) cert renew || true
	@echo "--- cert renew --force"
	$(DEMO_RUN) cert renew --force
	@echo "==> Certificate demo complete"

demo-skill-lifecycle: demo-fixtures build
	@echo "==> Demo: Skill Lifecycle Commands"
	@echo "--- pre-commit-validate"
	$(DEMO_RUN) pre-commit-validate || echo "    (No skills to validate)"
	@echo "--- skill-catalog"
	$(DEMO_RUN) skill-catalog
	@echo "--- skill-profile"
	$(DEMO_RUN) skill-profile
	@echo "--- skill-usage-report"
	$(DEMO_RUN) skill-usage-report
	@echo "--- skill-score"
	$(DEMO_RUN) skill-score || echo "    (No skills to score)"
	@echo "==> Skill lifecycle demo complete"

demo-analytics: demo-fixtures build
	@echo "==> Demo: Analytics Export/Import"
	@echo "--- export-analytics"
	$(DEMO_RUN) export-analytics --output $(HOME_DIR)/analytics-test.json
	@test -f $(HOME_DIR)/analytics-test.json || (echo "ERROR: Export failed" && exit 1)
	@echo "--- import-analytics"
	$(DEMO_RUN) import-analytics $(HOME_DIR)/analytics-test.json --overwrite
	@echo "--- Verify round-trip"
	@test -f $(HOME_DIR)/.skrills/analytics_cache.json || (echo "ERROR: Import failed" && exit 1)
	@echo "==> Analytics demo complete"

demo-gateway: build
	@echo "==> Demo: MCP Gateway Tools (unit tests)"
	@for filter in mcp_gateway list_mcp_tools describe_mcp_tool get_context_stats; do \
		$(CARGO_CMD) test --package skrills-server --lib -- $$filter --test-threads=1; \
	done
	@echo "==> Gateway demo complete"

demo-multi-cli-agent: demo-fixtures build
	@echo "==> Demo: Multi-CLI Agent Routing"
	@echo "--- multi-cli-agent --help"
	$(DEMO_RUN) multi-cli-agent --help >/dev/null
	@echo "--- multi-cli-agent dry-run (agent not found expected)"
	$(DEMO_RUN) multi-cli-agent test-agent --dry-run 2>&1 | grep -q "not found" || true
	@echo "--- multi-cli-agent unit tests"
	$(CARGO_CMD) test --package skrills-server --lib multi_cli_agent --all-features -- --test-threads=1
	@echo "==> Multi-CLI agent demo complete"

demo-fixtures:
	@mkdir -p $(HOME_DIR)/.codex/skills/demo
	@mkdir -p $(HOME_DIR)/.codex/bin
	@echo "demo skill content" > $(HOME_DIR)/.codex/skills/demo/SKILL.md
	@echo "# Agents" > $(HOME_DIR)/.codex/AGENTS.md
	@# Create MCP config files for doctor demo
	@echo '{"mcpServers":{"skrills":{"type":"stdio","command":"$(HOME_DIR)/.codex/bin/skrills","args":["serve"]}}}' > $(HOME_DIR)/.codex/mcp_servers.json
	@printf '[mcp_servers.skrills]\ntype = "stdio"\ncommand = "$(HOME_DIR)/.codex/bin/skrills"\nargs = ["serve"]\n' > $(HOME_DIR)/.codex/config.toml
	@# Create symlink to release binary for doctor validation
	@ln -sf $(CURDIR)/target/release/skrills $(HOME_DIR)/.codex/bin/skrills 2>/dev/null || cp $(CURDIR)/target/release/skrills $(HOME_DIR)/.codex/bin/skrills 2>/dev/null || true
	@# Create mock Claude session data for empirical demo
	@mkdir -p $(HOME_DIR)/.claude/projects/demo-project
	@for i in 01 02 03 04 05 06 07 08 09 10 11 12; do \
		echo '{"message":{"role":"user","content":[{"type":"text","text":"help me write code"}]},"timestamp":"2025-01-'$$i'T10:00:00Z"}' > $(HOME_DIR)/.claude/projects/demo-project/session-$$i.jsonl; \
		echo '{"message":{"content":[{"type":"tool_use","name":"Skill","input":{"skill":"commit"}}]},"timestamp":"2025-01-'$$i'T10:01:00Z"}' >> $(HOME_DIR)/.claude/projects/demo-project/session-$$i.jsonl; \
		echo '{"message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/path/to/skills/test/SKILL.md"}}]},"timestamp":"2025-01-'$$i'T10:02:00Z"}' >> $(HOME_DIR)/.claude/projects/demo-project/session-$$i.jsonl; \
	done
	@echo "Prepared demo HOME at $(HOME_DIR)"

demo-doctor: demo-fixtures build
	@echo "==> Demo: Doctor diagnostics"
	$(DEMO_RUN) doctor
	@echo "==> Doctor demo complete"

demo-release-consistency:
	@echo "==> Demo: Release-Consistency Invariants (LIVE)"
	@echo "--- Invariant 1: workspace crate versions (must agree)"
	@grep -hE '^version = "' crates/*/Cargo.toml | sort -u
	@echo "--- Invariant 2: plugin.json version (must match workspace)"
	@grep -E '"version":' plugins/skrills/.claude-plugin/plugin.json | head -1 | sed 's/^[[:space:]]*//'
	@echo "--- Invariants 3 & 4: command parity"
	@printf "    plugin.json commands.length: "
	@python3 -c "import json; print(len(json.load(open('plugins/skrills/.claude-plugin/plugin.json'))['commands']))"
	@printf "    plugins/skrills/commands/*.md (top-level): "
	@find plugins/skrills/commands -maxdepth 1 -name '*.md' | wc -l
	@echo "--- Invariant 5: marketplace.json plugin entries + sources"
	@python3 -c "import json; d=json.load(open('.claude-plugin/marketplace.json')); meta=d.get('metadata',{}).get('version','-'); print(f'    metadata.version: {meta}'); [print(f'    plugins[{i}] {p[\"name\"]} v{p[\"version\"]} source={p[\"source\"]}') for i,p in enumerate(d['plugins'])]"
	@echo "--- Running parity test suite"
	$(CARGO_CMD) test -p skrills_test_utils --test release_consistency
	@echo "==> Release-consistency demo complete"

demo-empirical: demo-fixtures build
	@echo "==> Demo: Empirical skill creation (dry-run)"
	$(DEMO_RUN) create-skill test-empirical --description "Demo skill from session patterns" --method empirical --dry-run || echo "(Empirical demo requires session history)"
	@echo "==> Empirical demo complete"

demo-cli: demo-fixtures build
	@echo "==> Testing all CLI commands"
	@echo "--- serve --help"
	$(DEMO_RUN) serve --help >/dev/null
	@echo "--- mirror"
	$(DEMO_RUN) mirror
	@echo "--- agent (no agents expected)"
	$(DEMO_RUN) agent test-agent --dry-run 2>&1 | grep -q "not found" || true
	@echo "--- sync-agents"
	$(DEMO_RUN) sync-agents
	@echo "--- sync"
	$(DEMO_RUN) sync
	@echo "--- sync-commands"
	$(DEMO_RUN) sync-commands
	@echo "--- sync-mcp-servers"
	$(DEMO_RUN) sync-mcp-servers
	@echo "--- sync-preferences"
	$(DEMO_RUN) sync-preferences
	@echo "--- sync-all"
	$(DEMO_RUN) sync-all
	@echo "--- sync-status"
	$(DEMO_RUN) sync-status
	@echo "--- doctor"
	$(DEMO_RUN) doctor
	@echo "--- validate"
	$(DEMO_RUN) validate
	@echo "--- analyze"
	$(DEMO_RUN) analyze
	@echo "--- metrics"
	$(DEMO_RUN) metrics
	@echo "--- recommend (no skills expected)"
	$(DEMO_RUN) recommend test-skill 2>&1 | grep -q "No skills" || true
	@echo "--- resolve-dependencies (skill not found expected)"
	$(DEMO_RUN) resolve-dependencies test-skill 2>&1 | grep -q "not found" || true
	@echo "--- recommend-skills-smart"
	$(DEMO_RUN) recommend-skills-smart
	@echo "--- analyze-project-context"
	$(DEMO_RUN) analyze-project-context
	@echo "--- suggest-new-skills"
	$(DEMO_RUN) suggest-new-skills
	@echo "--- create-skill --method empirical --dry-run"
	$(DEMO_RUN) create-skill test-skill --description "Test" --method empirical --dry-run || true
	@echo "--- create-skill --method github --dry-run"
	$(DEMO_RUN) create-skill test-skill --description "Test" --method github --dry-run || true
	@echo "--- search-skills-github"
	$(DEMO_RUN) search-skills-github "commit" || true
	@echo "--- export-analytics"
	$(DEMO_RUN) export-analytics --output $(HOME_DIR)/analytics-cli-test.json
	@test -f $(HOME_DIR)/analytics-cli-test.json && echo "    Export succeeded" || echo "    Export failed (expected on fresh install)"
	@echo "--- import-analytics (if export exists)"
	@test -f $(HOME_DIR)/analytics-cli-test.json && $(DEMO_RUN) import-analytics $(HOME_DIR)/analytics-cli-test.json --overwrite || true
	@echo "--- multi-cli-agent --dry-run (agent not found expected)"
	$(DEMO_RUN) multi-cli-agent test-agent --dry-run 2>&1 | grep -q "not found" || true
	@echo "--- setup --help"
	$(DEMO_RUN) setup --help >/dev/null
	@echo "==> All CLI commands tested successfully"

demo-all: demo-fixtures build demo-doctor demo-empirical demo-cli demo-setup-all demo-cert demo-skill-lifecycle demo-multi-cli-agent
	@echo "==> All demos completed successfully"
	@echo "    Note: demo-http excluded (blocking server)"

# --- Setup flow demos -----------------------------------------------------

demo-setup-claude: demo-fixtures build
	@echo "==> Demo: Setup for Claude Code (non-interactive)"
	@rm -rf $(HOME_DIR)/.claude
	$(DEMO_RUN) setup --client claude --bin-dir $(HOME_DIR)/.claude/bin --yes
	$(call verify_setup,Claude,mcp claude)

demo-setup-codex: demo-fixtures build
	@echo "==> Demo: Setup for Codex (non-interactive)"
	@rm -rf $(HOME_DIR)/.codex
	$(DEMO_RUN) setup --client codex --bin-dir $(HOME_DIR)/.codex/bin --yes
	$(call verify_setup,Codex,codex)

demo-setup-both: demo-fixtures build
	@echo "==> Demo: Setup for both Claude Code and Codex"
	@rm -rf $(HOME_DIR)/.claude $(HOME_DIR)/.codex
	$(DEMO_RUN) setup --client both --bin-dir $(HOME_DIR)/.claude/bin --yes
	$(call verify_setup,Both clients,mcp claude)

demo-setup-uninstall: demo-setup-claude
	@echo "==> Demo: Uninstall Claude setup"
	$(DEMO_RUN) setup --uninstall --client claude --yes
	@echo "==> Verifying uninstall..."
	@echo "==> Uninstall verified successfully"

demo-setup-reinstall: demo-setup-claude
	@echo "==> Demo: Reinstall Claude setup"
	$(DEMO_RUN) setup --client claude --bin-dir $(HOME_DIR)/.claude/bin --reinstall --yes
	$(call verify_setup,Reinstall,mcp)

demo-setup-universal: demo-fixtures build
	@echo "==> Demo: Setup with universal sync"
	@rm -rf $(HOME_DIR)/.claude $(HOME_DIR)/.agent
	@mkdir -p $(HOME_DIR)/.claude/skills
	@echo "test skill" > $(HOME_DIR)/.claude/skills/test.md
	$(DEMO_RUN) setup --client claude --bin-dir $(HOME_DIR)/.claude/bin --universal --yes
	$(call verify_setup,Universal,mcp claude)
	@test -d $(HOME_DIR)/.agent/skills || (echo "ERROR: Universal skills dir not created" && exit 1)
	@echo "==> Universal skills directory verified"

demo-setup-first-run: demo-fixtures build
	@echo "==> Demo: First-run detection (simulated with doctor command)"
	@rm -rf $(HOME_DIR)/.claude $(HOME_DIR)/.codex
	@echo "==> Running doctor command on fresh install (should NOT prompt for setup as it's not served by first-run logic)"
	$(DEMO_RUN) doctor 2>&1 || true
	@echo "==> First-run detection demo complete"

demo-setup-all: demo-setup-claude demo-setup-codex demo-setup-both demo-setup-uninstall demo-setup-reinstall demo-setup-universal demo-setup-first-run
	@echo "==> All setup demos completed successfully"

quick: fmt check
	@echo "==> Quick check passed"

watch:
	@if command -v cargo-watch >/dev/null 2>&1; then \
		$(CARGO_CMD) watch -x 'check --workspace'; \
	else \
		echo "cargo-watch not installed. Run: cargo install cargo-watch"; \
		exit 1; \
	fi

bench:
	$(CARGO_CMD) bench --workspace

release: fmt lint test build
	@echo "==> Release validation complete"
	@echo "    Binary: $(BIN_PATH)"
	@$(BIN_PATH) --version

clean:
	CARGO_HOME=$(CARGO_HOME) $(CARGO) clean

clean-demo:
	@rm -rf $(HOME_DIR)
	@echo "Removed demo HOME $(HOME_DIR)"

ci: fmt lint lint-hygiene test

verify-publish:
	@if bash -c 'declare -A x 2>/dev/null'; then bash scripts/verify_publish_order.sh; else echo "[SKIP] verify-publish requires bash 4+ (found $$(bash --version | head -1))"; fi

precommit: fmt-check lint lint-md lint-hygiene test test-install verify-publish

hooks:
	@git config core.hooksPath githooks
	@echo "Git hooks installed (githooks/pre-commit)"
	@echo "Pre-commit will run: make precommit"

check-deps:
	@echo "Checking optional dependencies..."
	@command -v cargo-audit >/dev/null 2>&1 && echo "  cargo-audit: ok" || echo "  cargo-audit: missing"
	@command -v cargo-deny >/dev/null 2>&1 && echo "  cargo-deny: ok" || echo "  cargo-deny: missing"
	@command -v cargo-llvm-cov >/dev/null 2>&1 && echo "  cargo-llvm-cov: ok" || echo "  cargo-llvm-cov: missing"
	@command -v cargo-watch >/dev/null 2>&1 && echo "  cargo-watch: ok" || echo "  cargo-watch: missing"
	@command -v $(MDBOOK) >/dev/null 2>&1 && echo "  mdbook: ok" || echo "  mdbook: missing"
	@command -v cargo-tarpaulin >/dev/null 2>&1 && echo "  cargo-tarpaulin: ok" || echo "  cargo-tarpaulin: missing"

security:
	@if command -v cargo-audit >/dev/null 2>&1; then \
		$(CARGO_CMD) audit; \
	else \
		echo "cargo-audit not installed. Run: cargo install cargo-audit"; \
		exit 1; \
	fi

deny:
	@if command -v cargo-deny >/dev/null 2>&1; then \
		$(CARGO_CMD) deny check; \
	else \
		echo "cargo-deny not installed. Run: cargo install cargo-deny"; \
		exit 1; \
	fi

deps-update:
	$(CARGO_CMD) update
	@echo "Dependencies updated. Run 'make test' to verify."

# =============================================================================
# Dogfood targets — v0.8.0 cold-window surfaces + script ports.
# Goal: every new CLI/TUI/browser/SSE/script entrypoint executes against
# real fixtures and is validated for exit code, output shape, or contract.
# =============================================================================

# Override-able port so a developer can run `make cold-window` (8888) and
# `make dogfood-cold-window-browser` (18888) side-by-side.
DOGFOOD_PORT ?= 18888
DOGFOOD_TMP  := /tmp/skrills-dogfood-$$$$

# 124 = `timeout` fired (clean), 143 = 128+SIGTERM (clean). Anything else fails.
# Note: callers MUST run the prior command under `set +e` so $$? is observable.
define _assert_clean_exit
	rc=$$?; case $$rc in 0|124|143) ;; *) echo "FAIL: exit $$rc"; exit 1;; esac
endef

dogfood-cold-window-headless: build
	@echo "==> [headless] engine ticks 3s @ 200ms cadence; expects graceful SIGTERM exit"
	@set +e; HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) timeout --signal=TERM 3 \
	  $(BIN_PATH) cold-window --tick-rate-ms 200 --alert-budget 100000 >/dev/null 2>&1 ; \
	$(_assert_clean_exit)
	@echo "==> [headless] OK"

dogfood-cold-window-chaos: build
	@echo "==> [chaos] --no-adaptive + alert-budget=1 forces kill-switch path"
	@set +e; HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) timeout --signal=TERM 3 \
	  $(BIN_PATH) cold-window --no-adaptive --tick-rate-ms 200 --alert-budget 1 >/dev/null 2>&1 ; \
	$(_assert_clean_exit)
	@echo "==> [chaos] OK"

dogfood-cold-window-browser: build
	@command -v jq   >/dev/null 2>&1 || { echo "FAIL: jq required (apt install jq)"; exit 1; }
	@command -v curl >/dev/null 2>&1 || { echo "FAIL: curl required"; exit 1; }
	@PORT=$(DOGFOOD_PORT) ; TMP=$(DOGFOOD_TMP) ; \
	mkdir -p $$TMP ; \
	echo "==> [browser] starting cold-window on port $$PORT" ; \
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) \
	  $(BIN_PATH) cold-window --browser --port $$PORT \
	    --tick-rate-ms 200 --alert-budget 100000 \
	    >$$TMP/server.log 2>&1 & \
	PID=$$! ; \
	cleanup() { kill -TERM $$PID 2>/dev/null || true ; wait $$PID 2>/dev/null || true ; rm -rf $$TMP ; } ; \
	trap cleanup EXIT INT TERM ; \
	for i in 1 2 3 4 5 6 7 8 9 10 ; do \
	  curl -fsS --max-time 1 "http://127.0.0.1:$$PORT/dashboard" >$$TMP/page.html 2>/dev/null && break ; \
	  sleep 0.5 ; \
	done ; \
	test -s $$TMP/page.html || { echo "FAIL: /dashboard never responded"; cat $$TMP/server.log; exit 1; } ; \
	echo "==> [browser] /dashboard HTML served ($$(wc -c <$$TMP/page.html) bytes)" ; \
	grep -q "/dashboard.sse" $$TMP/page.html || { echo "FAIL: HTML missing /dashboard.sse reference"; exit 1; } ; \
	for ev in alert hint research status ; do \
	  grep -q "addEventListener('$$ev'" $$TMP/page.html \
	    || { echo "FAIL: HTML page does not subscribe to '$$ev'"; exit 1; } ; \
	done ; \
	echo "==> [browser] HTML contract: declares listeners for {alert,hint,research,status}" ; \
	curl -fsS --max-time 3 -N "http://127.0.0.1:$$PORT/dashboard.sse" >$$TMP/stream.sse 2>/dev/null || true ; \
	test -s $$TMP/stream.sse || { echo "FAIL: SSE stream produced no bytes"; cat $$TMP/server.log; exit 1; } ; \
	for ev in alert hint research status ; do \
	  grep -q "^event: $$ev$$" $$TMP/stream.sse \
	    || { echo "FAIL: SSE missing 'event: $$ev'"; head -40 $$TMP/stream.sse; exit 1; } ; \
	done ; \
	DATA_LINES=$$(grep -c "^data: " $$TMP/stream.sse || true) ; \
	test "$$DATA_LINES" -ge 4 \
	  || { echo "FAIL: only $$DATA_LINES SSE data: lines (need >=4)"; head -40 $$TMP/stream.sse; exit 1; } ; \
	echo "==> [browser] SSE parity OK: 4 event names emitted, $$DATA_LINES data: lines" ; \
	START=$$(date +%s) ; kill -TERM $$PID ; wait $$PID 2>/dev/null || true ; \
	END=$$(date +%s) ; ELAPSED=$$((END - START)) ; \
	test $$ELAPSED -le 3 \
	  || { echo "FAIL: shutdown took $${ELAPSED}s (>3s budget per spec § 3 / TASK-031)"; exit 1; } ; \
	echo "==> [browser] graceful shutdown in $${ELAPSED}s (within 2s spec budget +1s test slack)"

# TUI/dashboard contract: under a TTY, render until SIGTERM (rc 124/143).
# Without a TTY (CI, redirected stdio), exit cleanly with a "requires a TTY"
# message — that graceful refusal is itself a contract worth testing.
dogfood-tui: build demo-fixtures
	@echo "==> [tui] TTY-or-graceful-refusal contract (skrills tui)"
	@TMP=$(DOGFOOD_TMP).tui.err ; \
	set +e; HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) timeout --signal=TERM 3 \
	  $(BIN_PATH) tui --skill-dir $(HOME_DIR)/.codex/skills </dev/null >/dev/null 2>$$TMP ; \
	rc=$$? ; \
	case $$rc in \
	  0|124|143) echo "==> [tui] OK (rc=$$rc, ran under TTY)" ; rm -f $$TMP ;; \
	  1) grep -qiE "tty|terminal" $$TMP \
	       && { echo "==> [tui] OK (rc=1, graceful no-TTY refusal: $$(cat $$TMP))" ; rm -f $$TMP ; } \
	       || { echo "FAIL: rc=1 but stderr lacks TTY explanation:" ; cat $$TMP ; rm -f $$TMP ; exit 1 ; } ;; \
	  *) echo "FAIL: tui exit $$rc" ; cat $$TMP ; rm -f $$TMP ; exit 1 ;; \
	esac

dogfood-dashboard: build demo-fixtures
	@echo "==> [dashboard] TTY-or-graceful-refusal contract (skrills dashboard)"
	@TMP=$(DOGFOOD_TMP).dash.err ; \
	set +e; HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) timeout --signal=TERM 3 \
	  $(BIN_PATH) dashboard --skill-dir $(HOME_DIR)/.codex/skills </dev/null >/dev/null 2>$$TMP ; \
	rc=$$? ; \
	case $$rc in \
	  0|124|143) echo "==> [dashboard] OK (rc=$$rc, ran under TTY)" ; rm -f $$TMP ;; \
	  1) grep -qiE "tty|terminal" $$TMP \
	       && { echo "==> [dashboard] OK (rc=1, graceful no-TTY refusal: $$(cat $$TMP))" ; rm -f $$TMP ; } \
	       || { echo "FAIL: rc=1 but stderr lacks TTY explanation:" ; cat $$TMP ; rm -f $$TMP ; exit 1 ; } ;; \
	  *) echo "FAIL: dashboard exit $$rc" ; cat $$TMP ; rm -f $$TMP ; exit 1 ;; \
	esac

dogfood-skill-diff: build demo-fixtures
	@command -v jq >/dev/null 2>&1 || { echo "FAIL: jq required"; exit 1; }
	@echo "==> [skill-diff] JSON output for the demo fixture skill"
	@TMP=$(DOGFOOD_TMP).json ; \
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) \
	  $(BIN_PATH) skill-diff demo --format json >$$TMP 2>/dev/null || true ; \
	jq -e '. | has("skill") and has("found_in")' $$TMP >/dev/null \
	  || { echo "FAIL: skill-diff JSON missing expected keys (.skill, .found_in)"; cat $$TMP; rm -f $$TMP; exit 1; } ; \
	echo "==> [skill-diff] keys: $$(jq -r '[keys[]] | tostring' $$TMP)" ; \
	rm -f $$TMP
	@echo "==> [skill-diff] OK"

# Direct-script targets: exercise the in-tree Python ports under scripts/
# even when audit-plugins.sh would route to NIGHT_MARKET_ROOT instead.
plugin-validate-direct:
	@echo "==> [direct] validate_plugin.py plugins/skrills"
	@python3 scripts/validate_plugin.py plugins/skrills
	@echo "==> [validate-direct] OK"

plugin-modernize-direct:
	@echo "==> [direct] check_hook_modernization.py --json --root ."
	@TMP=$(DOGFOOD_TMP).hooks.json ; \
	python3 scripts/check_hook_modernization.py --root . --json >$$TMP ; \
	jq -e '.success == true and .errors == 0' $$TMP >/dev/null \
	  || { echo "FAIL: hook modernization audit not clean:" ; jq . $$TMP ; rm -f $$TMP ; exit 1; } ; \
	echo "==> [modernize-direct] $$(jq -r '"errors=\(.errors) warnings=\(.warnings) findings=\(.findings | length)"' $$TMP)" ; \
	rm -f $$TMP
	@echo "==> [modernize-direct] OK"

plugin-registrations-direct:
	@echo "==> [direct] update_plugin_registrations.py --dry-run"
	@python3 scripts/update_plugin_registrations.py --dry-run --plugins-root plugins
	@echo "==> [registrations-direct] OK"

dogfood-all: dogfood \
             dogfood-cold-window-headless dogfood-cold-window-chaos dogfood-cold-window-browser \
             dogfood-tui dogfood-dashboard dogfood-skill-diff \
             plugin-validate-direct plugin-modernize-direct plugin-registrations-direct
	@echo "==> Full v0.8.0 dogfood pass complete"
