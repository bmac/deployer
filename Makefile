build-all: local-build linux-build

linux-build:
	vagrant provision

local-build:
	cargo build --release

clean:
	rm -rf target

.PHONY: build-all build-linux build-local clean
