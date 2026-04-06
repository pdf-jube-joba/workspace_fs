TASK ?= build
REPO ?= example_repository
CARGO ?= cargo
NPM ?= npm

KNOWN_TARGETS := help install build serve test
REPO_ARG := $(filter-out $(KNOWN_TARGETS),$(MAKECMDGOALS))
ifneq ($(REPO_ARG),)
REPO := $(firstword $(REPO_ARG))
endif

.PHONY: help install build serve test $(REPO_ARG)

help:
	@printf '%s\n' 'usage: make <install|build|serve|test> [repository-path]'
	@printf '%s\n' 'examples:'
	@printf '%s\n' '  make install'
	@printf '%s\n' '  make build example_repository'
	@printf '%s\n' '  make serve ../md_dir'
	@printf '%s\n' '  make serve REPO=../md_dir'

install:
	$(NPM) --prefix default_plugins/md_preview install

build:
	$(CARGO) run --manifest-path workspace_fs/Cargo.toml -- $(abspath $(REPO)) --task-only $(TASK)

serve:
	$(CARGO) run --manifest-path workspace_fs/Cargo.toml -- $(abspath $(REPO)) --task $(TASK)

test:
	$(CARGO) test --manifest-path workspace_fs/Cargo.toml --bin workspace_fs

$(REPO_ARG):
	@:
