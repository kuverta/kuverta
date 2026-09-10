COMPOSE := docker compose -f docker/docker-compose.yml

.DEFAULT_GOAL := help
.PHONY: help dev-up dev-down dev-reset dev-logs dev-shell ai-up ai-model \
        paperless-up build test test-all lint fmt check e2e spike spike-window clean

help: ## Show this help
	@grep -hE '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) | sort | \
	  awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

## -- dev services ----------------------------------------------------------

dev-up: ## Start the dev IMAP server and load the fixtures
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

build: ## Build everything
	cargo build --workspace

test: ## Run tests (integration tests skip if the dev server is down)
	cargo test --workspace

test-all: dev-up ## Run tests with the dev server required, as CI does
	FUCKMAIL_REQUIRE_DEV_SERVER=1 cargo test --workspace

lint: ## Clippy with warnings as errors
	cargo clippy --workspace --all-targets -- -D warnings

fmt: ## Format
	cargo fmt --all

check: ## What CI runs
	cargo fmt --all -- --check
	$(MAKE) lint
	$(MAKE) test-all

## -- spikes ----------------------------------------------------------------

# --release matters: a debug build triples the measured IPC cost, which is the
# one number this spike exists to find out.
spike: ## Tauri virtualized-list risk test; prints frame timings and a verdict
	cargo build --release -p fuckmail-desktop
	FUCKMAIL_SPIKE_AUTOEXIT=1 ./target/release/fuckmail-desktop

spike-window: ## Same, but leave the window open to scroll by hand
	cargo build --release -p fuckmail-desktop
	./target/release/fuckmail-desktop

## -- manual smoke test -----------------------------------------------------

e2e: dev-up build ## Register the dev account in a scratch store and sync it
	@rm -rf .devdata
	@FUCKMAIL_DATA_DIR=.devdata ./target/debug/fuckmail add-account \
	    --email dev@fuckmail.test --label "Dev server" \
	    --host 127.0.0.1 --port 10143 --security plaintext
	@FUCKMAIL_DEV_PASSWORD=devpass FUCKMAIL_DATA_DIR=.devdata \
	    ./target/debug/fuckmail sync --password-env FUCKMAIL_DEV_PASSWORD
	@FUCKMAIL_DATA_DIR=.devdata ./target/debug/fuckmail status
	@echo
	@FUCKMAIL_DATA_DIR=.devdata ./target/debug/fuckmail list

clean: ## Remove build output and the scratch store
	cargo clean
	rm -rf .devdata
