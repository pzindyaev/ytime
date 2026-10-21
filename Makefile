PREFIX ?= $(HOME)/.local
BINDIR := $(PREFIX)/bin
BIN    := target/release/ytime

.DEFAULT_GOAL := help

.PHONY: help build release run test clippy fmt check install uninstall clean

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}'

build: ## Build (debug)
	cargo build

release: ## Build (release, optimized)
	cargo build --release

run: build ## Run the app (debug)
	cargo run

test: ## Run unit tests
	cargo test

clippy: ## Lint
	cargo clippy --all-targets

fmt: ## Format code
	cargo fmt

check: fmt clippy test ## Format, lint, test

install: release ## Install the release binary ($(BINDIR)/ytime; PREFIX=… to override)
	install -Dm755 $(BIN) $(BINDIR)/ytime
	@echo "installed $(BINDIR)/ytime"

uninstall: ## Remove the installed binary
	rm -f $(BINDIR)/ytime

clean: ## Remove build artifacts
	cargo clean
