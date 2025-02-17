# ---------------------------------------------------------------------------- #
#                      SPDX-License-Identifier: GPL-2.0                        #
#                      SPDX-License-Identifier: MIT                            #
#                                                                              #
#                +--------------------------------------------+                #
#                |   █████╗ ██╗   ██╗██████╗  █████╗ ███████╗ |                #
#                |  ██╔══██╗██║   ██║██╔══██╗██╔══██╗██╔════╝ |                #
#                |  ███████║██║   ██║██████╔╝███████║█████╗   |                #
#                |  ██╔══██║██║   ██║██╔══██╗██╔══██║██╔══╝   |                #
#                |  ██║  ██║╚██████╔╝██║  ██║██║  ██║███████╗ |                #
#                |  ╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝ |                #
#                +--------------------------------------------+                #
#                                                                              #
#                         Distributed Systems Runtime                          #
#                                                                              #
# ---------------------------------------------------------------------------- #
# Dual Licensed: GNU GENERAL PUBLIC LICENSE 2.0                                #
# Dual Licensed: MIT License                                                   #
# Copyright 2023 The Aurae Authors (The Nivenly Foundation)                    #
# ---------------------------------------------------------------------------- #
dir := $(dir $(lastword $(MAKEFILE_LIST)))
include $(dir)../hack/_common.mk

aurae_ebpf    =  /var/lib/aurae/ebpf
cargo         =  cargo
uname_m       =  $(shell uname -m)

default: all ## Build all eBPF probes (debug)

all: build ## Build all eBPF probes (debug)

install: ## Install the eBPF probes to /var/lib/aurae/probes (release only)
	@mkdir -p $(aurae_ebpf)
	@cp -v  ./target/bpfel-unknown-none/release/instrument* $(aurae_ebpf)

.PHONY: build ## Build all eBPF probes (debug)
build: nightly bpf-linker
	$(cargo) +nightly build --target=bpfel-unknown-none -Z build-std=core

.PHONY: release ## Build all eBPF probes
release: nightly bpf-linker
	$(cargo) +nightly build --package ebpf-probes --target=bpfel-unknown-none -Z build-std=core --release

.PHONY: nightly
nightly: ## Add nightly toolchain (needed for eBPF)
	rustup toolchain list | grep -qc 'nightly-$(uname_m)-unknown-linux-gnu' || \
		rustup toolchain install nightly-$(uname_m)-unknown-linux-gnu --component rust-src

.PHONY: bpf-linker
bpf-linker: ## Install bpf-linker (needed for eBPF)
	$(cargo) install --list | grep -qc 'bpf-linker' || $(cargo) install bpf-linker

clean: ## Clean the target artifacts
	cargo clean
