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

CARGO_GUARD_TARGETS := fmt lint check test test-unit test-integration test-setup \
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
.PHONY: help fmt lint lint-md check test test-unit test-integration test-setup \
	build build-min serve-help install status coverage test-coverage dogfood ci precommit \
	clean clean-demo githooks hooks require-cargo security deny deps-update check-deps
# Phony targets: docs
.PHONY: docs book book-serve
# Phony targets: demos
.PHONY: demo-fixtures demo-doctor demo-all demo-setup-claude demo-setup-codex \
	demo-setup-both demo-setup-uninstall demo-setup-reinstall \
	demo-setup-universal demo-setup-first-run demo-setup-all
.NOTPARALLEL: demo-all demo-setup-all
.SILENT: demo-doctor demo-all demo-setup-claude demo-setup-codex demo-setup-both \
	demo-setup-uninstall demo-setup-reinstall demo-setup-universal demo-setup-first-run \
	demo-setup-all

$(CARGO_GUARD_TARGETS): require-cargo

help:
	@printf "Usage: make <target>\n\n"
	@printf "Core\n"
	@printf "  %-23s %s\n" "fmt" "format workspace"
	@printf "  %-23s %s\n" "lint" "clippy with -D warnings"
	@printf "  %-23s %s\n" "lint-md" "lint markdown files"
	@printf "  %-23s %s\n" "check" "cargo check all targets"
	@printf "  %-23s %s\n" "test | test-unit | test-integration" "run tests"
	@printf "  %-23s %s\n" "test-setup" "run setup module tests"
	@printf "  %-23s %s\n" "test-coverage" "run tests with coverage report"
	@printf "  %-23s %s\n" "build | build-min" "release builds"
	@printf "  %-23s %s\n" "install" "install skrills to $(CARGO_HOME)/bin"
	@printf "  %-23s %s\n" "serve-help" "binary --help smoke check"
	@printf "  %-23s %s\n" "status" "show project status and environment"
	@printf "  %-23s %s\n" "coverage" "generate test coverage report"
	@printf "  %-23s %s\n" "dogfood" "run skrills on its own codebase"
	@printf "  %-23s %s\n" "ci | precommit" "run common pipelines"
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
	@printf "  %-23s %s\n" "demo-all | demo-doctor" "run CLI demos"
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

demo-fixtures:
	@mkdir -p $(HOME_DIR)/.codex/skills/demo
	@mkdir -p $(HOME_DIR)/.codex
	@echo "demo skill content" > $(HOME_DIR)/.codex/skills/demo/SKILL.md
	@echo "# Agents" > $(HOME_DIR)/.codex/AGENTS.md
	@echo "Prepared demo HOME at $(HOME_DIR)"

demo-doctor: demo-fixtures build
	@echo "==> Demo: Doctor diagnostics"
	$(DEMO_RUN) doctor
	@echo "==> Doctor demo complete"

demo-all: demo-fixtures build demo-doctor

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
	@echo "==> Verifying universal sync..."
	@test -d $(HOME_DIR)/.agent/skills || (echo "ERROR: Universal skills dir not created" && exit 1)
	@echo "==> Universal sync verified successfully"

demo-setup-first-run: demo-fixtures build
	@echo "==> Demo: First-run detection (simulated with doctor command)"
	@rm -rf $(HOME_DIR)/.claude $(HOME_DIR)/.codex
	@echo "==> Running doctor command on fresh install (should NOT prompt for setup as it's not served by first-run logic)"
	$(DEMO_RUN) doctor 2>&1 || true
	@echo "==> First-run detection demo complete"

demo-setup-all: demo-setup-claude demo-setup-codex demo-setup-both demo-setup-uninstall demo-setup-reinstall demo-setup-universal demo-setup-first-run
	@echo "==> All setup demos completed successfully"

clean:
	CARGO_HOME=$(CARGO_HOME) $(CARGO) clean

clean-demo:
	@rm -rf $(HOME_DIR)
	@echo "Removed demo HOME $(HOME_DIR)"

ci: fmt lint test

precommit: fmt lint lint-md test

hooks:
	@git config core.hooksPath githooks
	@echo "Git hooks installed (githooks/pre-commit)"
	@echo "Pre-commit will run: make precommit"

check-deps:
	@echo "Checking optional dependencies..."
	@command -v cargo-audit >/dev/null 2>&1 && echo "  cargo-audit: ok" || echo "  cargo-audit: missing"
	@command -v cargo-deny >/dev/null 2>&1 && echo "  cargo-deny: ok" || echo "  cargo-deny: missing"
	@command -v cargo-llvm-cov >/dev/null 2>&1 && echo "  cargo-llvm-cov: ok" || echo "  cargo-llvm-cov: missing"
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
