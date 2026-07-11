# gpm2op — build contract. `make check` is the pre-push gate.

CARGO ?= cargo

.PHONY: all build test lint fmt fmt-check check install clean

all: check

build:
	$(CARGO) build

test:
	$(CARGO) test

lint:
	$(CARGO) clippy --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

check: fmt-check lint test build

install:
	$(CARGO) install --path .

clean:
	$(CARGO) clean

# Debug build re-signed with the stable pk-cli-codesign identity so macOS
# keychain "Always Allow" grants survive rebuilds (see cli-common/scripts).
dev:
	cargo build
	@if [ -x "$$HOME/Dev/cli-common/scripts/dev-sign.sh" ]; then \
		"$$HOME/Dev/cli-common/scripts/dev-sign.sh" target/debug/gpm2op; \
	else echo "cli-common/scripts/dev-sign.sh not found — binary left ad-hoc signed"; fi
