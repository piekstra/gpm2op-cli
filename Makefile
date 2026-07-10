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
