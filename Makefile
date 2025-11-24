# Common developer and demo tasks for codex-mcp-skills
# Set CARGO_HOME to a writable path to avoid sandbox/root perms issues.
CARGO ?= cargo
CARGO_HOME ?= .cargo
HOME_DIR ?= $(CURDIR)/.home-tmp
BIN ?= codex-mcp-skills
BIN_PATH ?= target/release/$(BIN)
MDBOOK ?= mdbook

.PHONY: help fmt lint check test build build-min serve-help emit-autoload \
	demo-fixtures demo-list demo-list-pinned demo-pin demo-unpin demo-autopin \
	demo-history demo-sync-agents demo-sync demo-emit-autoload demo-all \
	docs book book-serve clean clean-demo ci

help:
	@echo "Targets:"
	@echo "  fmt                format workspace"
	@echo "  lint               clippy with -D warnings"
	@echo "  check              cargo check all targets"
	@echo "  test               cargo test --all-features"
	@echo "  build              release build with features"
	@echo "  build-min          release build without default features"
	@echo "  serve-help         binary --help smoke check"
	@echo "  emit-autoload      sample emit-autoload run"
	@echo "  demo-all           run CLI demos (list/pin/history/sync, etc.)"
		@echo "  book               build mdBook then open in default browser"
	@echo "  book-serve         live-reload mdBook on localhost:3000"
	@echo "  clean              cargo clean"
	@echo "  clean-demo         remove demo HOME sandbox"
	@echo "  ci                 fmt + lint + test"

fmt:
	CARGO_HOME=$(CARGO_HOME) $(CARGO) fmt --all

lint:
	CARGO_HOME=$(CARGO_HOME) $(CARGO) clippy --workspace --all-targets -- -D warnings

check:
	CARGO_HOME=$(CARGO_HOME) $(CARGO) check --workspace --all-targets

test:
	CARGO_HOME=$(CARGO_HOME) $(CARGO) test --workspace --all-features

build:
	CARGO_HOME=$(CARGO_HOME) $(CARGO) build --workspace --all-features --release

build-min:
	CARGO_HOME=$(CARGO_HOME) $(CARGO) build --workspace --no-default-features --release

serve-help:
	CARGO_HOME=$(CARGO_HOME) $(CARGO) run --quiet --bin $(BIN) -- --help >/dev/null

emit-autoload:
	CARGO_HOME=$(CARGO_HOME) $(CARGO) run --quiet --bin $(BIN) -- emit-autoload --prompt "sample" --diagnose --max-bytes 512 >/dev/null

docs:
	CARGO_HOME=$(CARGO_HOME) RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --all-features --no-deps
	@doc_index="$(CURDIR)/target/doc/codex_mcp_skills/index.html"; \
	if [ -f "$$doc_index" ]; then \
	  if command -v xdg-open >/dev/null 2>&1; then xdg-open "$$doc_index" >/dev/null 2>&1 || true; \
	  elif command -v open >/dev/null 2>&1; then open "$$doc_index" >/dev/null 2>&1 || true; \
	  elif command -v start >/dev/null 2>&1; then start "$$doc_index" >/dev/null 2>&1 || true; \
	  else echo "Docs at $$doc_index"; fi; \
	else echo "Docs built, index not found at $$doc_index"; fi

book:
		@if ! command -v $(MDBOOK) >/dev/null 2>&1; then \
		  echo "mdbook not found; installing to $(CARGO_HOME)/bin"; \
		  CARGO_HOME=$(CARGO_HOME) $(CARGO) install mdbook --locked >/dev/null; \
		fi
		CARGO_HOME=$(CARGO_HOME) $(MDBOOK) build book
	@book_index="$(CURDIR)/book/book/index.html"; \
	if [ -f "$$book_index" ]; then \
	  if command -v xdg-open >/dev/null 2>&1; then xdg-open "$$book_index" >/dev/null 2>&1 || true; \
	  elif command -v open >/dev/null 2>&1; then open "$$book_index" >/dev/null 2>&1 || true; \
	  elif command -v start >/dev/null 2>&1; then start "$$book_index" >/dev/null 2>&1 || true; \
	  else echo "Book at $$book_index"; fi; \
	else echo "Book built, index not found at $$book_index"; fi

book-serve:
	@if ! command -v $(MDBOOK) >/dev/null 2>&1; then \
	  echo "mdbook not found; installing to $(CARGO_HOME)/bin"; \
	  CARGO_HOME=$(CARGO_HOME) $(CARGO) install mdbook --locked >/dev/null; \
	fi
	CARGO_HOME=$(CARGO_HOME) $(MDBOOK) serve book --open --hostname 127.0.0.1 --port 3000

# --- Demo helpers ---------------------------------------------------------

demo-fixtures:
	@mkdir -p $(HOME_DIR)/.codex/skills/demo
	@mkdir -p $(HOME_DIR)/.codex
	@echo "demo skill content" > $(HOME_DIR)/.codex/skills/demo/SKILL.md
	@echo "# Agents" > $(HOME_DIR)/.codex/AGENTS.md
	@echo "Prepared demo HOME at $(HOME_DIR)"

demo-list: demo-fixtures build
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) $(BIN_PATH) list >/dev/null

demo-list-pinned: demo-fixtures build
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) $(BIN_PATH) list-pinned >/dev/null

demo-pin: demo-fixtures build
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) $(BIN_PATH) pin demo/SKILL.md >/dev/null

demo-unpin: demo-fixtures build
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) $(BIN_PATH) unpin demo/SKILL.md >/dev/null

demo-autopin: demo-fixtures build
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) $(BIN_PATH) auto-pin --enable >/dev/null

demo-history: demo-fixtures build
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) $(BIN_PATH) history --limit 5 >/dev/null

demo-sync-agents: demo-fixtures build
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) $(BIN_PATH) sync-agents --path $(HOME_DIR)/.codex/AGENTS.md >/dev/null

demo-sync: demo-fixtures build
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) $(BIN_PATH) sync >/dev/null || true

demo-emit-autoload: demo-fixtures build
	HOME=$(HOME_DIR) CARGO_HOME=$(CARGO_HOME) $(BIN_PATH) emit-autoload --prompt demo --diagnose --max-bytes 256 >/dev/null

demo-all: demo-fixtures build demo-list demo-pin demo-list-pinned demo-unpin demo-list-pinned demo-autopin demo-history demo-sync-agents demo-sync demo-emit-autoload

clean:
	CARGO_HOME=$(CARGO_HOME) $(CARGO) clean

clean-demo:
	@rm -rf $(HOME_DIR)
	@echo "Removed demo HOME $(HOME_DIR)"

ci: fmt lint test
