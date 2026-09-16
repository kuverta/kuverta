COMPOSE := docker compose -f docker/docker-compose.yml

.DEFAULT_GOAL := help
.PHONY: help dev-up dev-down dev-reset dev-logs dev-shell ai-up ai-model \
        paperless-up build test test-all lint fmt check e2e app app-real \
        triage-ui triage test-js test-e2e \
        fill-mailbox fill-dev scannerd-pi scannerd-to-pi clean

help: ## Show this help
	@grep -hE '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) | sort | \
	  awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

## -- dev services ----------------------------------------------------------

dev-up: ## Start the dev stack: IMAP + fixtures + SMTP sink + Paperless-ngx
	$(COMPOSE) up -d
	@$(COMPOSE) logs mailseed --no-log-prefix | tail -3
	@echo "  Paperless-ngx: http://localhost:8000  (admin/admin, slow on first start)"

dev-down: ## Stop the dev services (keeps the mailbox)
	$(COMPOSE) down

dev-reset: ## Destroy and reseed the dev mailbox from scratch
	$(COMPOSE) down -v
	$(COMPOSE) up -d

dev-logs: ## Follow the Dovecot log
	$(COMPOSE) logs -f dovecot

ai-up: ## Start Ollama in a container (see docker/README.md before using this)
	$(COMPOSE) --profile ai up -d

ai-model: ## Pull the chat model and the embedding model into the Ollama container
	@# The chat model kuverta classify and eval default to: the smallest that
	@# held its accuracy on senders it had never seen (decisions §19).
	$(COMPOSE) --profile ai exec ollama ollama pull llama3.2:3b
	@# The embedding model `kuverta eval` scores against prompting. Plan §4 asks
	@# for both to be tried before either is committed to.
	$(COMPOSE) --profile ai exec ollama ollama pull nomic-embed-text

# Kept as a name people already type; Paperless is in the default stack now.
paperless-up: dev-up ## Start Paperless-ngx on http://localhost:8000 (admin/admin)

## -- build and test --------------------------------------------------------

# -j 2 throughout: a full-parallelism build of this workspace gets OOM-killed on
# this machine, and a killed build reads as a mystery rather than as running out
# of memory.
build: ## Build everything
	cargo build -j 2 --workspace

test: ## Run tests (integration tests skip if the dev server is down)
	cargo test -j 2 --workspace
	$(MAKE) test-js

test-e2e: ## Run the browser tests: the triage page and the settings sheet in Chromium
	@# Kept out of `make test`: they need a browser download and take seconds.
	@# Chromium is installed only when Playwright does not already have it.
	cd kuverta-bird && (test -d node_modules || npm ci --no-audit --no-fund) \
	  && npx playwright install chromium && npm run e2e

test-js: ## Run the triage surface's tests (no mail client needed)
	@# jsdom is the one dependency, for the view's tests; installed from the lock
	@# file, and only when it is missing, so a normal run stays offline.
	cd kuverta-bird && (test -d node_modules || npm ci --no-audit --no-fund) && npm test

test-all: dev-up ## Run tests with the dev server required, as CI does
	KUVERTA_REQUIRE_DEV_SERVER=1 cargo test -j 2 --workspace

lint: ## Clippy with warnings as errors
	cargo clippy -j 2 --workspace --all-targets -- -D warnings

fmt: ## Format
	cargo fmt --all

check: ## What CI runs
	cargo fmt --all -- --check
	$(MAKE) lint
	$(MAKE) test-all

## -- the app ---------------------------------------------------------------

# --release matters for the list: a debug build triples the cost of moving rows
# across the Rust/JS bridge, which is the one thing the spike found to be tight.
# See docs/spike-tauri-list.md.
app: ## Open the triage window on the scratch store
	./run.sh --dev

# Tauri serves one directory as the web root, so the shared surface has to be
# inside it. The copy is verbatim and the script verifies it, so the files the
# app loads are the files `make test-js` runs.
triage-ui: ## Copy the shared triage surface into the desktop app
	kuverta-bird/tools/install-into.sh apps/desktop/ui

triage: triage-ui ## Open the window on the scratch store, ready to triage
	@echo
	@echo "  Opening on .devdata. Click 'triage' in the header."
	@echo "  No mail in there yet?  make dev-up && make e2e"
	@echo
	./run.sh --dev

app-real: ## Open it on the real data directory
	./run.sh

## -- manual smoke test -----------------------------------------------------

e2e: dev-up build ## Register the dev account in a scratch store, sync it, and send
	@rm -rf .devdata
	@KUVERTA_DATA_DIR=.devdata ./target/debug/kuverta add-account \
	    --email dev@kuverta.test --label "Dev server" \
	    --host 127.0.0.1 --port 10143 --security plaintext \
	    --smtp-host 127.0.0.1 --smtp-port 1025 --smtp-security plaintext
	@KUVERTA_DEV_PASSWORD=devpass KUVERTA_DATA_DIR=.devdata \
	    ./target/debug/kuverta sync --password-env KUVERTA_DEV_PASSWORD
	@KUVERTA_DATA_DIR=.devdata ./target/debug/kuverta status
	@echo
	@KUVERTA_DATA_DIR=.devdata ./target/debug/kuverta list
	@echo
	@# --no-save-to-sent on purpose: filing a copy would add a message to the
	@# seeded mailbox, and the sync tests assert on its exact contents. The
	@# Sent copy is covered by the dev_server integration test, on its own user.
	@echo "Hallo Jane, hier ist kuverta." | \
	  KUVERTA_DEV_PASSWORD=devpass KUVERTA_DATA_DIR=.devdata \
	    ./target/debug/kuverta send --to "Jane Doe <jane@example.com>" \
	    --subject "Gruesse aus kuverta" --no-save-to-sent \
	    --password-env KUVERTA_DEV_PASSWORD
	@echo "  -> read it at http://localhost:8025"
	@echo
	@# Queued and then withdrawn, so the demo shows the undo window without
	@# mutating the seeded mailbox the sync tests assert on.
	@KUVERTA_DATA_DIR=.devdata ./target/debug/kuverta archive 1
	@KUVERTA_DATA_DIR=.devdata ./target/debug/kuverta queue
	@KUVERTA_DATA_DIR=.devdata ./target/debug/kuverta undo


fill-mailbox: ## Put ~250 varied messages in a test mailbox (HOST= USER= PASS=)
	@test -n "$(HOST)" || (echo "usage: make fill-mailbox HOST=imap.example.de USER=you@example.de PASS=…" && false)
	python3 docker/fill-mailbox.py "$(HOST)" "$(USER)" "$(PASS)" $(or $(COUNT),250)

# Nine fixtures prove sync works and are nowhere near enough to feel like a
# mailbox — which is what triage needs in order to be worth looking at.
fill-dev: dev-up ## Put ~250 varied messages in the dev mailbox, then sync them
	python3 docker/fill-mailbox.py 127.0.0.1:10143 dev@kuverta.test devpass \
	    $(or $(COUNT),250) --plain
	KUVERTA_DEV_PASSWORD=devpass KUVERTA_DATA_DIR=.devdata \
	    ./target/debug/kuverta sync --password-env KUVERTA_DEV_PASSWORD

## -- the scanner -----------------------------------------------------------

# Cross-compiled with cargo-zigbuild, which uses zig as the C compiler and
# linker and can pin the glibc version: 2.36 is what bookworm ships. The default
# target is the Pi Zero W's ARMv6 hard-float; a Pi 3, 4, 5 or Zero 2 W on 64-bit
# Raspberry Pi OS wants PI_TARGET=aarch64-unknown-linux-gnu.2.36. Needs
# `rustup target add <target without .2.36>`, `cargo install cargo-zigbuild`,
# and zig on PATH (ziglang.org; ~/.local/zig is looked in too).
PI_TARGET ?= arm-unknown-linux-gnueabihf.2.36
PI_HOST ?= pi@192.168.8.241
# The target without its glibc suffix: `basename` would strip only `.36`.
PI_BIN = target/$(firstword $(subst ., ,$(PI_TARGET)))/release/scannerd

scannerd-pi: ## Cross-compile scannerd for the Pi (PI_TARGET=, default Zero W ARMv6)
	PATH="$(HOME)/.local/zig:$(PATH)" cargo zigbuild --release -j 2 -p scannerd --target $(PI_TARGET)
	@file $(PI_BIN)

scannerd-to-pi: scannerd-pi ## Copy scannerd and its unit file to ~/scannerd on PI_HOST (installs nothing)
	ssh $(PI_HOST) 'mkdir -p scannerd'
	@# Copied under another name and renamed: Linux refuses to open a running
	@# executable for writing, but lets a rename replace it.
	scp $(PI_BIN) $(PI_HOST):scannerd/scannerd.new
	scp apps/scannerd/scannerd.service apps/scannerd/scannerd.env.example $(PI_HOST):scannerd/
	ssh $(PI_HOST) 'mv scannerd/scannerd.new scannerd/scannerd'
	@echo "  -> on the Pi: ~/scannerd/scannerd --help; installing is in apps/scannerd/readme.md"

clean: ## Remove build output and the scratch store
	cargo clean
	rm -rf .devdata
