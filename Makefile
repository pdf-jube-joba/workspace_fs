NPM ?= npm

.PHONY: help install

help:
	@printf '%s\n' 'usage: make <install>'
	@printf '%s\n' 'examples:'
	@printf '%s\n' '  make install'

install:
	$(NPM) --prefix default_plugins/md_preview install
