COMPOSE := docker compose -f docker/docker-compose.yml

.DEFAULT_GOAL := help
.PHONY: help dev-up dev-down dev-reset dev-logs dev-shell ai-up ai-model \
        paperless-up build test test-all lint fmt check e2e app app-real \
        triage-ui \
        fill-mailbox clean

help: ## Show this help
	@grep -hE '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) | sort | \
	  awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

## -- dev services ----------------------------------------------------------

dev-up: ## Start the dev IMAP servers and SMTP sink, and load the fixtures
	$(COMPOSE) up -d
	@$(COMPOSE) logs mailseed --no-log-prefix | tail -3

dev-down: ## Stop the dev services (keeps the mailbox)
	$(COMPOSE) down

dev-reset: ## Destroy and reseed the dev mailbox from scratch
	$(COMPOSE) down -v
	$(COMPOSE) up -d

dev-logs: ## Follow the Dovecot log
	$(COMPOSE) logs -f dovecot

ai-up: ## Start Ollama in a container (see docker/README.md before using this)
	$(COMPOSE) --profile ai up -d

ai-model: ## Pull the classification model into the Ollama container
	$(COMPOSE) --profile ai exec ollama ollama pull qwen3:8b

paperless-up: ## Start Paperless-ngx on http://localhost:8000 (admin/admin)
	$(COMPOSE) --profile paperless up -d

## -- build and test --------------------------------------------------------

# -j 2 throughout: a full-parallelism build of this workspace gets OOM-killed on
# this machine, and a killed build reads as a mystery rather than as running out
# of memory.
build: ## Build everything
	cargo build -j 2 --workspace

test: ## Run tests (integration tests skip if the dev server is down)
	cargo test -j 2 --workspace

test-all: dev-up ## Run tests with the dev server required, as CI does
	FUCKMAIL_REQUIRE_DEV_SERVER=1 cargo test -j 2 --workspace

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

# The shared surface lives in the fuckbird repository; this copies it in. The
# copy is verbatim and the script verifies it, so the files the app loads are
# the files that repository's tests run.
FUCKBIRD ?= ../fuckbird

triage-ui: ## Install the shared triage surface from the fuckbird repo
	@test -x $(FUCKBIRD)/tools/install-into.sh || \
	  (echo "fuckbird not found at $(FUCKBIRD) — set FUCKBIRD=/path/to/fuckbird" && false)
	$(FUCKBIRD)/tools/install-into.sh apps/desktop/ui

app-real: ## Open it on the real data directory
	./run.sh

## -- manual smoke test -----------------------------------------------------

e2e: dev-up build ## Register the dev account in a scratch store, sync it, and send
	@rm -rf .devdata
	@FUCKMAIL_DATA_DIR=.devdata ./target/debug/fuckmail add-account \
	    --email dev@fuckmail.test --label "Dev server" \
	    --host 127.0.0.1 --port 10143 --security plaintext \
	    --smtp-host 127.0.0.1 --smtp-port 1025 --smtp-security plaintext
	@FUCKMAIL_DEV_PASSWORD=devpass FUCKMAIL_DATA_DIR=.devdata \
	    ./target/debug/fuckmail sync --password-env FUCKMAIL_DEV_PASSWORD
	@FUCKMAIL_DATA_DIR=.devdata ./target/debug/fuckmail status
	@echo
	@FUCKMAIL_DATA_DIR=.devdata ./target/debug/fuckmail list
	@echo
	@# --no-save-to-sent on purpose: filing a copy would add a message to the
	@# seeded mailbox, and the sync tests assert on its exact contents. The
	@# Sent copy is covered by the dev_server integration test, on its own user.
	@echo "Hallo Jane, hier ist fuckmail." | \
	  FUCKMAIL_DEV_PASSWORD=devpass FUCKMAIL_DATA_DIR=.devdata \
	    ./target/debug/fuckmail send --to "Jane Doe <jane@example.com>" \
	    --subject "Gruesse aus fuckmail" --no-save-to-sent \
	    --password-env FUCKMAIL_DEV_PASSWORD
	@echo "  -> read it at http://localhost:8025"
	@echo
	@# Queued and then withdrawn, so the demo shows the undo window without
	@# mutating the seeded mailbox the sync tests assert on.
	@FUCKMAIL_DATA_DIR=.devdata ./target/debug/fuckmail archive 1
	@FUCKMAIL_DATA_DIR=.devdata ./target/debug/fuckmail queue
	@FUCKMAIL_DATA_DIR=.devdata ./target/debug/fuckmail undo


fill-mailbox: ## Put ~250 varied messages in a test mailbox (HOST= USER= PASS=)
	@test -n "$(HOST)" || (echo "usage: make fill-mailbox HOST=imap.example.de USER=you@example.de PASS=…" && false)
	python3 docker/fill-mailbox.py "$(HOST)" "$(USER)" "$(PASS)" $(or $(COUNT),250)

clean: ## Remove build output and the scratch store
	cargo clean
	rm -rf .devdata
