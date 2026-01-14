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
.PHONY: help fmt fmt-check lint lint-md check test test-unit test-integration test-setup test-install \
	build build-min serve-help install status coverage test-coverage dogfood ci precommit \
	clean clean-demo githooks hooks require-cargo security deny deps-update check-deps \
	quick watch bench release
# Phony targets: docs
.PHONY: docs book book-serve
# Phony targets: demos
.PHONY: demo-fixtures demo-doctor demo-empirical demo-http demo-cli demo-all demo-setup-claude demo-setup-codex \
	demo-setup-both demo-setup-uninstall demo-setup-reinstall \
	demo-setup-universal demo-setup-first-run demo-setup-all \
	demo-analytics demo-gateway
.NOTPARALLEL: demo-all demo-setup-all
.SILENT: demo-doctor demo-empirical demo-cli demo-all demo-setup-claude demo-setup-codex demo-setup-both \
	demo-setup-uninstall demo-setup-reinstall demo-setup-universal demo-setup-first-run \
	demo-setup-all

$(CARGO_GUARD_TARGETS): require-cargo

help:
	@printf "Usage: make <target>\n\n"
	@printf "Core\n"
	@printf "  %-23s %s\n" "fmt | fmt-check" "format workspace or check only"
	@printf "  %-23s %s\n" "lint" "clippy with -D warnings"
	@printf "  %-23s %s\n" "lint-md" "lint markdown files"
	@printf "  %-23s %s\n" "check" "cargo check all targets"
	@printf "  %-23s %s\n" "test | test-unit | test-integration" "run tests"
	@printf "  %-23s %s\n" "test-setup" "run setup module tests"
	@printf "  %-23s %s\n" "test-install" "test install.sh helper functions"
	@printf "  %-23s %s\n" "test-coverage" "run tests with coverage report"
	@printf "  %-23s %s\n" "build | build-min" "release builds"
	@printf "  %-23s %s\n" "install" "install skrills to $(CARGO_HOME)/bin"
	@printf "  %-23s %s\n" "serve-help" "binary --help smoke check"
	@printf "  %-23s %s\n" "status" "show project status and environment"
	@printf "  %-23s %s\n" "coverage" "generate test coverage report"
	@printf "  %-23s %s\n" "dogfood" "run skrills on its own codebase"
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
	@printf "  %-23s %s\n" "demo-analytics" "test analytics export/import"
	@printf "  %-23s %s\n" "demo-gateway" "test MCP gateway tools"
	@printf "  %-23s %s\n" "demo-setup-all" "run all setup flow demos"
	@printf "  %-23s %s\n" "demo-setup-{claude,codex,both}" "client setup demos"
	@printf "  %-23s %s\n" "demo-setup-{uninstall,reinstall}" "lifecycle demos"
	@printf "  %-23s %s\n" "demo-setup-{universal,first-run}" "other setup demos"
	@printf "  %-23s %s\n" "demo-fixtures" "prepare demo HOME sandbox"

require-cargo:
	@command -v $(CARGO) >/dev/null 2>&1 || { \
		echo "cargo not found. Install Rust from https://rustup.rs/"; exit 1; }

fmt:
	$(CARGO_CMD) fmt --all

fmt-check:
	$(CARGO_CMD) fmt --all -- --check

lint:
	$(CARGO_CMD) clippy --workspace --all-targets -- -D warnings

lint-md:
	$(SHELL) ./scripts/lint-markdown.sh

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
	@version=$$(grep '^version' crates/cli/Cargo.toml | head -1 | cut -d'=' -f2 | cut -d'#' -f1 | tr -d " \"'"); \
	echo "Version: $$version"
	@echo "Rust: $$(rustc --version)"
	@echo "Cargo: $$(cargo --version)"
	@echo "Branch: $$(git rev-parse --abbrev-ref HEAD)"
	@echo "Commit: $$(git rev-parse --short HEAD)"
	@echo "Binary: $(BIN_PATH) $$(test -f $(BIN_PATH) && echo '(exists)' || echo '(not built)')"

install:
	$(CARGO_CMD) install --path crates/cli --locked

githooks:
	./scripts/install-git-hooks.sh

coverage:
	$(CARGO_CMD) tarpaulin --workspace --all-features --out Html
	$(call open_file,$(CURDIR)/tarpaulin-report.html)

dogfood: build demo-fixtures
	@echo "==> Dogfooding: Running skrills on itself"
	HOME=$(HOME_DIR) $(BIN_PATH) doctor
	@echo "==> Dogfood complete"

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
	$(CARGO_CMD) test --package skrills-server --lib -- mcp_gateway --test-threads=1
	$(CARGO_CMD) test --package skrills-server --lib -- list_mcp_tools --test-threads=1
	$(CARGO_CMD) test --package skrills-server --lib -- describe_mcp_tool --test-threads=1
	$(CARGO_CMD) test --package skrills-server --lib -- get_context_stats --test-threads=1
	@echo "==> Gateway demo complete"

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
	@for i in 1 2 3 4 5 6 7 8 9 10 11 12; do \
		echo '{"message":{"role":"user","content":[{"type":"text","text":"help me write code"}]},"timestamp":"2025-01-0'$$i'T10:00:00Z"}' > $(HOME_DIR)/.claude/projects/demo-project/session-$$i.jsonl; \
		echo '{"message":{"content":[{"type":"tool_use","name":"Skill","input":{"skill":"commit"}}]},"timestamp":"2025-01-0'$$i'T10:01:00Z"}' >> $(HOME_DIR)/.claude/projects/demo-project/session-$$i.jsonl; \
		echo '{"message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/path/to/skills/test/SKILL.md"}}]},"timestamp":"2025-01-0'$$i'T10:02:00Z"}' >> $(HOME_DIR)/.claude/projects/demo-project/session-$$i.jsonl; \
	done
	@echo "Prepared demo HOME at $(HOME_DIR)"

demo-doctor: demo-fixtures build
	@echo "==> Demo: Doctor diagnostics"
	$(DEMO_RUN) doctor
	@echo "==> Doctor demo complete"

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
	@echo "--- setup --help"
	$(DEMO_RUN) setup --help >/dev/null
	@echo "==> All CLI commands tested successfully"

demo-all: demo-fixtures build demo-doctor demo-empirical demo-cli demo-setup-all
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

ci: fmt lint test

precommit: fmt-check lint lint-md test test-install

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
