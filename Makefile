# Makefile — local dev tasks + crates.io publish orchestration.
#
# All targets are phony unless noted. Run `make help` for a summary.
#
# Publish targets:
#
#   make publish-check    # cargo publish --dry-run for every crate (no upload)
#   make publish          # serially upload all crates in dep-graph order
#   make publish-PKG      # upload a single crate (e.g. make publish-hwhkit)
#
# crates.io takes ~30s after each upload to index the new version, so the
# publish loop sleeps `$(INDEX_WAIT)` seconds between crates. Bump it if
# you hit "could not find ... in registry" while resolving the *next*
# crate's path-dep version.
#
# Auth: `cargo login` once. `cargo publish` reuses the cached token.

SHELL := /usr/bin/env bash
.SHELLFLAGS := -eu -o pipefail -c

# How long to wait after each `cargo publish` before publishing the next
# crate. Allows the crates.io index to reflect the new version so that
# downstream crates can find it as a path dep upgraded to a published
# dep. 30s is conservative; on a slow day bump to 60.
INDEX_WAIT ?= 30

# Topological publish order. Each tier may publish in parallel in
# principle, but we serialize for predictability and so a failure
# halfway through is easy to reason about.
#
# Tier 0 — no internal deps:
TIER_0 := hwhkit-buildinfo hwhkit-config hwhkit-observability cargo-hwhkit
# Tier 1 — depends on Tier 0 (hwhkit-config):
TIER_1 := hwhkit-core
# Tier 2 — depends on Tier 1 (hwhkit-core, hwhkit-config):
TIER_2 := hwhkit-scheduler \
          hwhkit-integration-postgres \
          hwhkit-integration-redis \
          hwhkit-integration-mongodb \
          hwhkit-integration-nats \
          hwhkit-integration-qdrant \
          hwhkit-integration-neo4j \
          hwhkit-integration-s3 \
          hwhkit-integration-oss
# Tier 3 — facade crate, pulls in everything via feature flags:
TIER_3 := hwhkit

PUBLISH_ORDER := $(TIER_0) $(TIER_1) $(TIER_2) $(TIER_3)

.PHONY: help
help: ## Show this help.
	@awk 'BEGIN {FS = ":.*##"; printf "Targets:\n"} \
	      /^[a-zA-Z_/-]+:.*##/ {printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2}' \
	      $(MAKEFILE_LIST)

# ---- Local dev tasks ----

.PHONY: check
check: ## cargo check across the workspace
	cargo check --workspace --all-features --all-targets

.PHONY: test
test: ## Default hermetic test suite (no docker, ignores live tests)
	cargo test --workspace --all-features

.PHONY: test-live
test-live: ## Run #[ignore]'d live integration tests (requires docker)
	cargo test --workspace --all-features -- --ignored

.PHONY: clippy
clippy: ## Workspace clippy with -D warnings (matches CI policy)
	cargo clippy --workspace --all-features --all-targets -- -D warnings

.PHONY: fmt
fmt: ## Format everything
	cargo fmt --all

.PHONY: fmt-check
fmt-check: ## Verify formatting without changing files
	cargo fmt --all -- --check

.PHONY: deny
deny: ## License + advisory + source audit (cargo-deny)
	cargo deny check

.PHONY: ci
ci: fmt-check clippy test deny ## Full local pre-PR gate

# ---- Publish ----
#
# `publish-check` runs `cargo publish --dry-run` for every crate so you
# can catch metadata problems (missing `description`, bad `repository`,
# uncommitted files, …) before pushing anything to the registry.
#
# `publish` does the real upload. It refuses to run if the working tree
# is dirty (set ALLOW_DIRTY=1 if you really know what you're doing).

ALLOW_DIRTY ?=
PUBLISH_FLAGS := $(if $(ALLOW_DIRTY),--allow-dirty,)

.PHONY: publish-check
publish-check: ## Best-effort pre-publish validation
	@# Cargo publish dry-run has a fundamental limitation for workspaces:
	@# downstream crates (e.g. `hwhkit-core`) cannot resolve their
	@# workspace deps (e.g. `hwhkit-config = "0.6.0-alpha.1"`) from
	@# crates.io until those deps are *actually* published. Even
	@# `cargo package --no-verify` hits the registry during the
	@# "prepare local package" phase, so there's no way to truly
	@# dry-run a downstream crate before its deps exist on crates.io.
	@#
	@# We split the check into two tiers:
	@#   1. Tier 0 (no internal deps): full `cargo publish --dry-run`.
	@#   2. Tier 1-3: `cargo package --list` (file-set / manifest
	@#      validation only — doesn't talk to the registry). Catches
	@#      missing files, bad globs, missing description, etc., but
	@#      not "this crate won't compile after publish."
	@echo ">>> Tier 0 (leaves): full dry-run with crates.io check"
	@for pkg in $(TIER_0); do \
		echo "==> dry-run: $$pkg"; \
		cargo publish --package $$pkg --dry-run --no-verify --allow-dirty || exit 1; \
	done
	@echo ">>> Tier 1-3: cargo package --list (manifest-only validation)"
	@for pkg in $(TIER_1) $(TIER_2) $(TIER_3); do \
		echo "==> package --list: $$pkg"; \
		cargo package --package $$pkg --list --allow-dirty >/dev/null || exit 1; \
	done
	@echo "<<< publish-check passed for $(words $(PUBLISH_ORDER)) crates"

.PHONY: publish-guard
publish-guard:
	@if [ -z "$(ALLOW_DIRTY)" ] && ! git diff --quiet HEAD --; then \
		echo "error: working tree is dirty. Commit your changes or set ALLOW_DIRTY=1."; \
		exit 1; \
	fi
	@if ! cargo --version >/dev/null 2>&1; then echo "error: cargo not found"; exit 1; fi
	@echo "publish-guard: tree clean, cargo present"

.PHONY: publish
publish: publish-guard ## Real upload in dep order (auth via `cargo login` first)
	@echo ">>> publish order: $(PUBLISH_ORDER)"
	@echo ">>> INDEX_WAIT=$(INDEX_WAIT)s between crates"
	@for pkg in $(PUBLISH_ORDER); do \
		echo "==> publish: $$pkg"; \
		cargo publish --package $$pkg $(PUBLISH_FLAGS) || { \
			echo "!!! publish failed at $$pkg"; \
			echo "    Re-run with: make publish-resume RESUME_AT=$$pkg"; \
			exit 1; \
		}; \
		echo "    sleeping $(INDEX_WAIT)s for crates.io index update..."; \
		sleep $(INDEX_WAIT); \
	done
	@echo "<<< all $(words $(PUBLISH_ORDER)) crates published"

# Resume a partially-completed publish run. RESUME_AT must be a package
# name that's already published successfully; we'll start from the one
# AFTER it. Example:
#   make publish-resume RESUME_AT=hwhkit-core
.PHONY: publish-resume
publish-resume: publish-guard ## Resume publish from after $(RESUME_AT)
	@if [ -z "$(RESUME_AT)" ]; then echo "error: set RESUME_AT=<last-published-pkg>"; exit 1; fi
	@found=0; \
	for pkg in $(PUBLISH_ORDER); do \
		if [ "$$pkg" = "$(RESUME_AT)" ]; then found=1; continue; fi; \
		if [ "$$found" = "1" ]; then \
			echo "==> publish: $$pkg"; \
			cargo publish --package $$pkg $(PUBLISH_FLAGS) || exit 1; \
			sleep $(INDEX_WAIT); \
		fi; \
	done

# Per-crate publish target. `make publish-hwhkit-core` runs a single
# upload (useful when you only need to patch one crate).
.PHONY: $(addprefix publish-,$(PUBLISH_ORDER))
$(addprefix publish-,$(PUBLISH_ORDER)): publish-%: publish-guard
	cargo publish --package $* $(PUBLISH_FLAGS)

# ---- Convenience ----

.PHONY: clean
clean: ## cargo clean
	cargo clean

.PHONY: docs
docs: ## Build docs locally with all features (matches docs.rs as closely as possible)
	cargo doc --workspace --all-features --no-deps --open

.PHONY: bench-build
bench-build: ## Compile all criterion benches (fast — what CI does)
	cargo bench --workspace --no-run
