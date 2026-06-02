# Terrana developer tasks. Run `make help` for the full list.
#
# Override the sample file/port for `make run`:
#   make run FILE=path/to/data.csv PORT=9000 ARGS="--lat lat --lon lon"

FILE ?= testdata/observations.csv
PORT ?= 8080
ARGS ?=
DATASET ?= 100k

.DEFAULT_GOAL := help

.PHONY: help
help: ## Show this help
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

.PHONY: build
build: ## Build (debug)
	cargo build

.PHONY: release
release: ## Build optimized release binary
	cargo build --release

.PHONY: run
run: ## Run the server against $(FILE) on $(PORT) (e.g. make run FILE=data.csv)
	cargo run -- serve $(FILE) --port $(PORT) $(ARGS)

.PHONY: gen
gen: ## Generate standard benchmark datasets (10k/100k/1m) into testdata/
	python3 testdata/generate.py --preset bench

.PHONY: gen-250m
gen-250m: ## Generate the 250M-row benchmark dataset (~15 GB) into testdata/
	python3 testdata/generate.py --preset 250m

.PHONY: bench
bench: ## Run the benchmark suite (e.g. make bench DATASET=1m)
	./testdata/bench.sh $(DATASET)

.PHONY: test
test: ## Run fast unit tests (offline)
	cargo test

.PHONY: test-all
test-all: ## Run unit + integration tests (starts the server; needs network)
	cargo test --all -- --include-ignored

.PHONY: fmt
fmt: ## Format the code
	cargo fmt --all

.PHONY: fmt-check
fmt-check: ## Check formatting without modifying files
	cargo fmt --all --check

.PHONY: lint
lint: ## Run clippy with warnings denied (all targets)
	cargo clippy --all-targets -- -D warnings

.PHONY: check
check: ## Type-check all targets
	cargo check --all-targets

.PHONY: ci
ci: fmt-check lint test ## Run the offline CI gate (fmt + clippy + unit tests)

.PHONY: install
install: ## Install the terrana binary from this checkout
	cargo install --path .

.PHONY: package
package: ## List the files that would be published to crates.io
	cargo package --list

.PHONY: publish-dry
publish-dry: ## Dry-run a crates.io publish (verifies build + metadata; needs network)
	cargo publish --dry-run

.PHONY: doc
doc: ## Build and open the API docs
	cargo doc --no-deps --open

.PHONY: clean
clean: ## Remove build artifacts
	cargo clean
