.DEFAULT_GOAL := help

# ---------------------------------------------------------------------------
# Octos (merged workspace)
# ---------------------------------------------------------------------------

CARGO ?= cargo
FEATURES ?= api
OCTOS_DIR ?= $(CURDIR)/.octos
HOST ?= 127.0.0.1
PORT ?= 50080
SERVE_FLAGS ?= --solo

.PHONY: help init serve dashboard-build web-build app-build dev test \
        dev-web dev-gallery dev\:website build\:website

help: ## Show available local-development commands.
	@awk 'BEGIN {FS = ":.*##"}; /^[a-zA-Z][a-zA-Z0-9_-]*:.*##/ {printf "  %-18s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

init: ## Interactively create project-local .octos/config.json.
	$(CARGO) run -p octos-cli -- init --cwd "$(CURDIR)"

serve: ## Start the local API server (default: password-free local login).
	$(CARGO) run -p octos-cli --features "$(FEATURES)" -- serve --cwd "$(CURDIR)" --data-dir "$(OCTOS_DIR)" --host "$(HOST)" --port "$(PORT)" $(SERVE_FLAGS)

dashboard-build: ## Build the embedded /admin/ dashboard.
	./scripts/build-dashboard.sh

web-build: ## Initialize octos-web and build the embedded /app/ client.
	git submodule update --init octos-web
	./scripts/build-web-app.sh

app-build: dashboard-build web-build ## Build all embedded browser assets.

dev: app-build serve ## Build browser assets, then start the local web app.

# NOTE: the workspace now also contains the Zed/gpui-component crates, so this
# runs their test suites too. Use `$(CARGO) test -p octos-cli` (or another
# `-p` selection) to test only the Octos side.
test: ## Run the Rust workspace test suite.
	$(CARGO) test --workspace

# ---------------------------------------------------------------------------
# Zed / gpui-component (targets that predate the Octos merge)
# ---------------------------------------------------------------------------

dev-web: ## Build the wasm story bundle and run the website dev server.
	$(MAKE) -C crates/story-web build-wasm-dev build-web
	$(MAKE) dev\:website

dev-gallery: ## Run the gpui-kit story gallery dev server.
	$(MAKE) -C crates/story-web dev

# `--cwd` moves only the spawned process, so the site's own package.json and
# astro.config stay inside website/ and the repository root stays Rust-only.
dev\:website: ## Run the website (website/) dev server.
	bun run --cwd website dev

build\:website: ## Build the website (website/).
	bun run --cwd website build
