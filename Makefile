CRATE_NAME:=detect-compression

.PHONY: all
all: check

.PHONY: check
check: soft-clean
	cargo test
	cargo fmt -- --check
	cargo clippy -- -D warnings

.PHONY: soft-clean
soft-clean:
	cargo clean -p $(CRATE_NAME)

.PHONY: clean
clean:
	cargo clean