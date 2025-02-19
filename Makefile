# ---------------------------------------------------------------------------- #
#                |   █████╗ ██╗   ██╗██████╗  █████╗ ███████╗ |                #
#                |  ██╔══██╗██║   ██║██╔══██╗██╔══██╗██╔════╝ |                #
#                |  ███████║██║   ██║██████╔╝███████║█████╗   |                #
#                |  ██╔══██║██║   ██║██╔══██╗██╔══██║██╔══╝   |                #
#                |  ██║  ██║╚██████╔╝██║  ██║██║  ██║███████╗ |                #
#                |  ╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝ |                #
#                +--------------------------------------------+                #
#                                                                              #
#                         Distributed Systems Runtime                          #
# ---------------------------------------------------------------------------- #
# Copyright 2022 - 2024, the aurae contributors                                #
# SPDX-License-Identifier: Apache-2.0                                          #
# ---------------------------------------------------------------------------- #
# Variables and Settings
branch       ?=  main
message      ?=  Default commit message. Aurae Runtime environment.
cargo         =  cargo
oci           =  docker
ociopts       =  DOCKER_BUILDKIT=1
uid           =  $(shell id -u)
uname_m       =  $(shell uname -m)
cri_version   =  release-1.26
clh_version   =  30.0
vm_kernel     =  6.1.6
vm_image      =  https://cloud-images.ubuntu.com/focal/current/focal-server-cloudimg-amd64.img
ifeq ($(uid), 0)
root_cargo    =  cargo
else
root_cargo    =  sudo -E `which cargo`
endif

# Configuration Options
export GIT_PAGER = cat

#------------------------------------------------------------------------------#

# Aliases

# This section is not intended for "custom target content" and is only intended
# as alias targets for existing targets. Do not add a "body" to any targets here
# as it will introduce a bespoke build process. You MUST call existing targets.
# If you do not like the existing targets, you are welcome to try to change them
# following the normal contribution workflows.

# Keep all as the first command to have it be the default as per convention
.PHONY: all
all: install ## alias for install

# ⚠️ DO NOT REMOVE ⚠️
.PHONY: nova
nova: auraed aer auraescript ## The official Kris Nóva alias for her workflow that only exists in her head.
# ⚠️ DO NOT REMOVE ⚠️

dir := $(dir $(lastword $(MAKEFILE_LIST)))
include $(dir)/hack/_common.mk

#------------------------------------------------------------------------------#

# Notes:
# - make will not run the same step multiple times
# - The ideal order for cargo to reuse artifacts is build -> lint -> test
# - Different cargo target variants (nightly is a variant) do not produce compatible artifacts
# - Cargo's `install` artifacts are not usable for build, lint, or test (and vice versa)
# - Clippy seems to cache the results of only 1 target at a time

# Super commands

.PHONY: clean
clean: clean-certs clean-gens clean-crates ## Clean the repo

.PHONY: lint
lint: auraed-lint not-auraed-lint ## Run all lints

.PHONY: test
test: auraed-build auraed-test not-auraed-build not-auraed-test ## Builds, lints, and tests (does not include ignored tests)

.PHONY: test-all
test-all: auraed-build auraed-test-all not-auraed-build not-auraed-test-all ## Run lints and tests (includes ignored tests)

.PHONY: build
build: auraed-build auraed-lint not-auraed-build not-auraed-lint ## Build and lint

.PHONY: install
install: lint test auraed-debug auraescript-debug aer-debug ## Lint, test, and install (debug) 🎉

.PHONY: docs
docs: docs-crates docs-stdlib docs-other ## Assemble all the /docs for the website locally.

.PHONY: prcheck
prcheck: build lint test-all docs docs-lint ## Meant to mimic the GHA checks (includes ignored tests)

#------------------------------------------------------------------------------#

# Setup Commands

.PHONY: pki
pki: install-certs ## Alias for install-certs
.PHONY: certs
certs: clean-certs ## Generate x509 mTLS certs in /pki directory
	mkdir -p pki
	./hack/certgen
.PHONY: install-certs
install-certs: certs ## Install certs in /etc/aurae
ifeq ($(uid), 0)
	mkdir -p /etc/aurae/pki
	cp -v pki/* /etc/aurae/pki
else
	sudo -E mkdir -p /etc/aurae/pki
	sudo -E cp -v pki/* /etc/aurae/pki
endif
	@echo "Install PKI Auth Material [/etc/aurae]"

.PHONY: config
config: certs ## Set up default config
	mkdir -p $(HOME)/.aurae
	cp -v auraescript/default.config.toml $(HOME)/.aurae/config
	sed -i 's|~|$(HOME)|g' $(HOME)/.aurae/config
	mkdir -p $(HOME)/.aurae/pki
	cp -v pki/* $(HOME)/.aurae/pki

.PHONY: os-deps ## install os dependencies for building aurae
os-deps: ; ./hack/install-build-deps.sh

#------------------------------------------------------------------------------#

# Clean Commands

.PHONY: clean-crates
clean-crates: ## Clean target directory
	$(cargo) clean

.PHONY: clean-certs
clean-certs: ## Clean the cert material
	rm -rvf pki/*

.PHONY: clean-gen
clean-gens: ## Clean gen directories
	rm -rf proto/gen/*
	rm -rf auraescript/gen/*

#------------------------------------------------------------------------------#

# Protobuf Commands

GEN_TS_PATTERN = auraescript/gen/v0/%.ts
GEN_RS_PATTERN = proto/gen/aurae.%.v0.rs
GEN_SERDE_RS_PATTERN = proto/gen/aurae.%.v0.serde.rs
GEN_TONIC_RS_PATTERN = proto/gen/aurae.%.v0.tonic.rs

PROTOS = $(wildcard api/v0/*/*.proto)
PROTO_DIRS = $(filter-out api/v0/README.md, $(wildcard api/v0/*))

GEN_RS = $(patsubst api/v0/%,$(GEN_RS_PATTERN),$(PROTO_DIRS))
GEN_RS += $(patsubst api/v0/%,$(GEN_SERDE_RS_PATTERN),$(PROTO_DIRS))
GEN_RS += $(patsubst api/v0/%,$(GEN_TONIC_RS_PATTERN),$(PROTO_DIRS))

GEN_TS = $(patsubst api/v0/%.proto,$(GEN_TS_PATTERN),$(PROTOS))

BUF_VERSION = $(shell buf --version)

$(GEN_TS_PATTERN) $(GEN_RS_PATTERN) $(GEN_SERDE_RS_PATTERN) $(GEN_TONIC_RS_PATTERN): $(PROTOS)
	@if [ "${BUF_VERSION}" != "1.32.0" ]; then echo "Warning: buf 1.32.0 is not installed! Please install v1.32.0 of the 'buf' command line tool: https://docs.buf.build/installation"; exit 1; fi;
	buf lint api
	buf generate -v api

.PHONY: proto-vendor
proto-vendor: proto-vendor-cri proto-vendor-grpc-health ## Copy the upstream protobuf interfaces

.PHONY: proto-vendor-cri
proto-vendor-cri: ## Copy the CRI interface from upstream
	curl https://raw.githubusercontent.com/kubernetes/cri-api/$(cri_version)/pkg/apis/runtime/v1/api.proto -o api/cri/v1/$(cri_version).proto

.PHONY: proto-vendor-grpc-health
proto-vendor-grpc-health: ## Copy the gRPC Health interface from upstream
	curl https://raw.githubusercontent.com/grpc/grpc/master/src/proto/grpc/health/v1/health.proto -o api/grpc/health/v1/health.proto

#------------------------------------------------------------------------------#

PROGS = aer auraed auraescript

define AURAE_template =
.PHONY: $(1)
$(1): $(GEN_RS) $(GEN_TS) $(1)-lint $(1)-debug ## Lint and install $(1) (for use during development)

.PHONY: $(1)-lint
$(1)-lint: $(GEN_RS) $(GEN_TS)
	$$(cargo) clippy  -p $(1) --all-features -- -D clippy::all -D warnings

.PHONY: $(1)-test
$(1)-test: $(GEN_RS) $(GEN_TS) $(1)-lint
	$(cargo) test  -p $(1) --locked

.PHONY: $(1)-test-all
$(1)-test-all: $(GEN_RS) $(GEN_TS) $(1)-lint
	$(root_cargo) test  -p $(1) --locked -- --include-ignored

.PHONY: $(1)-test-integration
$(1)-test-integration: $(GEN_RS) $(GEN_TS) $(1)-lint
	$(root_cargo) test -p $(1) --locked --test '*' -- --include-ignored

.PHONY: $(1)-test-watch
$(1)-test-watch: $(GEN_RS) $(GEN_TS) $(1)-lint # Use cargo-watch to continuously run a test (e.g. make $(1)-test-watch name=path::to::test)
	$(root_cargo) watch -- $(cargo) test -p $(1) --locked $(name) -- --include-ignored --nocapture

.PHONY: $(1)-build
$(1)-build: $(GEN_RS) $(GEN_TS)
	$(cargo) build -p $(1) --locked

.PHONY: $(1)-build-release
$(1)-build-release: $(GEN_RS) $(GEN_TS)
	$(cargo) build -p $(1) --locked --release

.PHONY: $(1)-debug
$(1)-debug: $(GEN_RS) $(GEN_TS) $(1)-lint
	$(cargo) install --path ./$(1) --debug --force --locked

.PHONY: $(1)-release
$(1)-release: $(GEN_RS) $(GEN_TS) $(1)-lint $(1)-test ## Lint, test, and install $(1)
	$(cargo) install --path ./$(1) --force --locked
endef

$(foreach p,$(PROGS),$(eval $(call AURAE_template,$(p),$(if $(findstring auraed,$(p)),))))

#------------------------------------------------------------------------------#

# auraed Commands

.PHONY: auraed-start
auraed-start: ## Starts the installed auraed executable
ifeq ($(uid), 0)
	$(HOME)/.cargo/bin/auraed
else
	sudo -E $(HOME)/.cargo/bin/auraed
endif

#------------------------------------------------------------------------------#

# Commands for not auraed
#	Due to the way cargo & clippy cache artifacts, these commands are leveraged to
#	allow for faster build/lint/test by not switching targets as often

.PHONY: not-auraed-build
not-auraed-build: $(GEN_RS) $(GEN_TS)
	$(cargo) build --workspace --locked --exclude auraed

.PHONY: not-auraed-lint
not-auraed-lint: $(GEN_RS) $(GEN_TS)
	$(cargo) clippy --all-features --workspace --exclude auraed -- -D clippy::all -D warnings

.PHONY: not-auraed-test
not-auraed-test: $(GEN_RS) $(GEN_TS) not-auraed-lint
	$(cargo) test --workspace --locked --exclude auraed

.PHONY: not-auraed-test-all
not-auraed-test-all: $(GEN_RS) $(GEN_TS) not-auraed-lint
	$(cargo) test --workspace --locked --exclude auraed -- --include-ignored

#------------------------------------------------------------------------------#

# Commands for other crates

.PHONY: libs-lint
libs-lint: $(GEN_RS) $(GEN_TS)
	$(cargo) clippy --all-features --workspace --exclude auraed --exclude auraescript --exclude aer  -- -D clippy::all -D warnings

.PHONY: libs-test
libs-test: $(GEN_RS) $(GEN_TS)
	$(cargo) test --workspace --locked --exclude auraed --exclude auraescript --exclude aer

.PHONY: libs-test-all
libs-test-all: $(GEN_RS) $(GEN_TS)
	$(cargo) test --workspace --locked --exclude auraed --exclude auraescript --exclude aer -- --include-ignored

.PHONY: ebpf
ebpf:
	cd ebpf && make release install

#------------------------------------------------------------------------------#

# Documentation Commands

.PHONY: docs-lint
docs-lint: # Check the docs for typos
	vale --no-wrap --glob='!docs/stdlib/v0/*' ./docs

.PHONY: docs-stdlib
## Generate the docs for the stdlib from the .proto files
ifeq (, $(wildcard /usr/local/bin/protoc-gen-doc))
docs-stdlib:
	$(error "No /usr/local/bin/protoc-gen-doc, install from https://github.com/pseudomuto/protoc-gen-doc")
else
docs-stdlib: $(GEN_TS) $(GEN_RS)
	protoc --plugin=/usr/local/bin/protoc-gen-doc -I api/v0/discovery -I api/v0/observe -I api/v0/cells -I api/v0/vms --doc_out=docs/stdlib/v0 --doc_opt=markdown,index.md:Ignore* api/v0/*/*.proto --experimental_allow_proto3_optional
endif

.PHONY: docs-crates
docs-crates: $(GEN_TS) $(GEN_RS) ## Build the crate (documentation)
	$(cargo) doc --no-deps --package auraed
	$(cargo) doc --no-deps --package auraescript
	$(cargo) doc --no-deps --package client
	$(cargo) doc --no-deps --package aer
	cp -rv target/doc/* docs/crate

.PHONY: docs-other
docs-other:
	cp -rv README.md docs/index.md # Special copy for the main README
	cp -rv api/README.md docs/stdlib/index.md # Special copy for the main README

.PHONY: docs-serve
docs-serve: docs ## Run the aurae.io static website locally
ifeq ($(uid), 0)
	./hack/serve.sh
else
	sudo -E ./hack/serve.sh
endif

#------------------------------------------------------------------------------#

# Container Commands

.PHONY: oci-image-build
oci-image-build: ## Build the aurae/auraed OCI images
	$(ociopts) $(oci) build -t $(tag) -f $(ocifile) $(flags) .

.PHONY: oci-run
oci-run: ## Run the aurae/auraed OCI images
	$(ociopts) $(oci) run -v $(shell pwd):/app $(flags) $(tag) $(command)

.PHONY: oci-make
oci-make: ## Run the makefile inside the aurae/auraed OCI images
	$(ociopts) $(oci) run -v $(shell pwd):/app --rm -it $(tag) $(command)

.PHONY: oci-push
oci-push: ## Push to a user repository
	$(ociopts) $(oci) push $(tag)

.PHONY: oci-image-build-raw
oci-image-build-raw: ## Plain Jane oci build
	$(oci) build -t $(tag) -f $(ocifile) $(flags) .

.PHONY: container
container: ## Build the container defined in hack/container
	./hack/container

.PHONY: spawn
spawn: ## Spawn the current auraed binary and start it in a container
	./hack/spawn

.PHONY: busybox
busybox: ## Create a "busybox" OCI bundle in target
	./hack/oci-busybox

.PHONY: alpine
alpine: ## Create an "alpine" OCI bundle in target
	./hack/oci-alpine

#------------------------------------------------------------------------------#

# Hypervisor commands

/opt/aurae/cloud-hypervisor/cloud-hypervisor:
	mkdir -p /opt/aurae/cloud-hypervisor
	curl -LI https://github.com/cloud-hypervisor/cloud-hypervisor/releases/download/v$(clh_version)/cloud-hypervisor-static -o /opt/aurae/cloud-hypervisor/cloud-hypervisor
	chmod +x /opt/aurae/cloud-hypervisor/cloud-hypervisor

hypervisor/guest-kernel/linux-cloud-hypervisor:
	git clone --depth 1 https://github.com/cloud-hypervisor/linux.git -b ch-$(vm_kernel) hypervisor/guest-kernel/linux-cloud-hypervisor

.PHONY: build-guest-kernel
build-guest-kernel: hypervisor/guest-kernel/linux-cloud-hypervisor
	cp hypervisor/guest-kernel/linux-config-x86_64 hypervisor/guest-kernel/linux-cloud-hypervisor/.config
	cd hypervisor/guest-kernel/linux-cloud-hypervisor && KCFLAGS="-Wa,-mx86-used-note=no" make bzImage -j `nproc`
	mkdir -p /var/lib/aurae/vm/kernel
	cp hypervisor/guest-kernel/linux-cloud-hypervisor/arch/x86/boot/compressed/vmlinux.bin /var/lib/aurae/vm/kernel/vmlinux.bin

.PHONY: prepare-image
prepare-image:
	mkdir -p /var/lib/aurae/vm/image
	curl $(vm_image) -o /var/lib/aurae/vm/image/disk.img
	qemu-img convert -p -f qcow2 -O raw /var/lib/aurae/vm/image/disk.img /var/lib/aurae/vm/image/disk.raw
	mkdir -p /mnt
# TODO: read offset from filesystem (parse fdisk -l output)
	mount -t ext4 -o loop,offset=116391936 /var/lib/aurae/vm/image/disk.raw /mnt
# TODO: certificate provisioning
	cp -vR /etc/aurae /mnt/etc/
	mkdir -p /mnt/var/lib/aurae/ebpf
	cp -v ./ebpf/target/bpfel-unknown-none/release/instrument* /mnt/var/lib/aurae/ebpf/
	mkdir -p /mnt/lib/auraed
	cp -v ./target/x86_64-unknown-linux-musl/debug/auraed /mnt/lib/auraed/
	ln -sf ../lib/auraed/auraed /mnt/sbin/init
	umount /mnt

#------------------------------------------------------------------------------#

# CI Commands

.PHONY: ci-release
ci-release: test auraed-build-release auraescript-build-release aer-build-release # Preps release artifacts (for CI use)

.PHONY: ci-stage-release-artifacts
ci-stage-release-artifacts: ci-release ## Preps and stages release artifacts (for CI use)
	mkdir -p /tmp/release
	cp target/$(uname_m)-unknown-linux-musl/release/auraed /tmp/release/auraed-$(tag)-$(uname_m)-unknown-linux-musl
	cp target/release/auraescript /tmp/release/auraescript-$(tag)-$(uname_m)-unknown-linux-gnu

.PHONY: ci-upload-release-artifacts
ci-upload-release-artifacts: ci-release ci-stage-release-artifacts ## Preps, stages, and uploads release artifacts to github (for CI use)
	gh release upload $(tag) /tmp/release/auraed-$(tag)-$(uname_m)-unknown-linux-musl
	gh release upload $(tag) /tmp/release/auraescript-$(tag)-$(uname_m)-unknown-linux-gnu

.PHONY: ci-local
ci-local: ## Tests a github action's workflow locally using `act` (e.g., `make ci-local file=001-tester-ubuntu-make-test.yml`)
	act -W ./.github/workflows/$(file)

#------------------------------------------------------------------------------#

# Other Commands

.PHONY: tlsinfo
tlsinfo: ## Show TLS Info for /var/run/aurae*
	./hack/server-tls-info

.PHONY: fmt
fmt: headers ## Format the entire code base(s)
	./hack/code-format

# writes license headers to files that are missing them. from the root of the
# repo, you can run `DRY_RUN=1 ./hack/headers-write` manually in case you are
# unsure what the script is going to do
.PHONY: headers
headers:
	./hack/headers-write

.PHONY: check-deps
check-deps: $(GEN_TS) $(GEN_RS) ## Check if there are any unused dependencies in Cargo.toml
	$(cargo) +nightly udeps --target $(uname_m)-unknown-linux-musl --package auraed
	$(cargo) +nightly udeps --package auraescript
	$(cargo) +nightly udeps --package client
	$(cargo) +nightly udeps --package aer

.PHONY: check-deny
check-deny: $(GEN_TS) $(GEN_RS) ## Run cargo-deny
	$(cargo) deny check
