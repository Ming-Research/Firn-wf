# firn's canonical checks. `make check` is the gate a revision passes before
# it merges into main; CI runs the same targets.

PY ?= python3

# Every path is relative to this Makefile, so each target works from any
# working directory.
ROOT := $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
BUILD := $(ROOT)/build

# The live design trees: every root node file directly under design/ except
# the log, so a tree is linted in the same change that adds it.
DESIGN_TREES := $(filter-out log,$(basename $(notdir $(wildcard $(ROOT)/design/*.md))))

# The revision a design-tree change is reviewed against. CI selects it per
# event with design/skill/review-base.sh.
DESIGN_REVIEW_BASE ?= origin/main

# The pinned Whitefoot compiler. whitefoot.pin holds one line,
# `release = wf-<12-character commit hash>`, or `release = wf-exp-<hash>` for
# an experiment release on an experiment branch, naming a release of
# Ming-Research/Whitefoot; `make compiler` downloads that release's
# whitefootc for this host, checked against its SHA256SUMS and manifest, to
# build/whitefoot/<release>/.
PIN := $(ROOT)/whitefoot.pin
PIN_LINE := ^release = wf-(exp-)?[0-9a-f]{12}$$
RELEASE := $(shell sed -n -E 's/^release = (wf-(exp-)?[0-9a-f]{12})$$/\1/p' $(PIN) 2>/dev/null)
RELEASE_COMMIT := $(lastword $(subst -, ,$(RELEASE)))
RELEASES := https://github.com/Ming-Research/Whitefoot/releases/download
HOST := $(shell uname -s)-$(shell uname -m)
ASSET := $(if $(filter Linux-x86_64,$(HOST)),whitefootc-linux-x86_64.tar.gz,$(if $(filter Darwin-arm64,$(HOST)),whitefootc-macos-arm64.tar.gz))
WHITEFOOT := $(BUILD)/whitefoot/$(RELEASE)
WHITEFOOTC := $(WHITEFOOT)/whitefootc

# firn is one module program; any of its sources changes the build.
FIRN_GRAPH := $(ROOT)/firn/modules.wfg
FIRN_SOURCES := $(shell find $(ROOT)/firn -name '*.wf' -o -name '*.wfm' -o -name '*.wfg')

.PHONY: check compiler firn firn-lto design-lint design-ready

check: compiler firn design-lint

compiler: $(WHITEFOOTC)

$(PIN):
	@echo "whitefoot.pin is missing; it names the Whitefoot compiler release (AGENTS.md, Upgrading Whitefoot)" >&2
	@exit 1

$(WHITEFOOTC): $(PIN)
	@test "$$(grep -c '' $(PIN))" = 1 && grep -qE '$(PIN_LINE)' $(PIN) || { echo "whitefoot.pin must hold exactly one line: release = wf-<12-character commit hash>" >&2; exit 1; }
	@test -n "$(ASSET)" || { echo "Whitefoot publishes no compiler for $(HOST)" >&2; exit 1; }
	@rm -rf $(WHITEFOOT).part && mkdir -p $(WHITEFOOT).part
	@cd $(WHITEFOOT).part && for file in $(ASSET) SHA256SUMS whitefoot-release.json; do \
		curl -fsSL --retry 3 -o $$file $(RELEASES)/$(RELEASE)/$$file || { \
			echo "cannot download $$file of $(RELEASE): dispatch Whitefoot's release workflow for that commit (AGENTS.md, Upgrading Whitefoot)" >&2; \
			exit 1; }; \
	done
	@cd $(WHITEFOOT).part && grep '  $(ASSET)$$' SHA256SUMS | shasum -a 256 -c -
	@cd $(WHITEFOOT).part && $(PY) -c 'import json, sys; m = json.load(open("whitefoot-release.json")); sys.exit(0 if m["tag"] == "$(RELEASE)" and m["commit"].startswith("$(RELEASE_COMMIT)") else "whitefoot-release.json does not describe $(RELEASE)")'
	@cd $(WHITEFOOT).part && tar -xzf $(ASSET) && rm $(ASSET) && test -x whitefootc
	@rm -rf $(WHITEFOOT) && mv $(WHITEFOOT).part $(WHITEFOOT)
	@echo "whitefootc $(RELEASE) for $(HOST) at $(WHITEFOOTC)"

# The server as a developer runs it: an incremental cache keeps each module's
# checked and compiled parts, so a rebuild compiles only what an edit changed.
# Every build depends on the pin itself: a downloaded compiler keeps its
# archive's timestamp, which can be older than a server built before the pin
# moved.
firn: $(BUILD)/firn

$(BUILD)/firn: $(PIN) $(WHITEFOOTC) $(FIRN_SOURCES)
	$(WHITEFOOTC) --graph $(FIRN_GRAPH) --entry firn --cache $(BUILD)/firn-cache -o $@

# The server as it is measured: the program and the runtime optimized
# together under full link-time optimization.
firn-lto: $(BUILD)/firn-lto

$(BUILD)/firn-lto: $(PIN) $(WHITEFOOTC) $(FIRN_SOURCES)
	$(WHITEFOOTC) --full-lto --graph $(FIRN_GRAPH) --entry firn -o $@

# The design skill's own tests, then the lint of every live tree.
design-lint:
	@$(PY) -B -m unittest discover -s $(ROOT)/design/skill -p 'test_lint.py'
	@$(if $(DESIGN_TREES),$(PY) -B $(ROOT)/design/skill/lint.py --root $(ROOT)/design --trees $(DESIGN_TREES) --base "$(DESIGN_REVIEW_BASE)",echo "design lint: no live tree")

design-ready:
	@$(if $(DESIGN_TREES),$(PY) -B $(ROOT)/design/skill/lint.py --root $(ROOT)/design --trees $(DESIGN_TREES) --base "$(DESIGN_REVIEW_BASE)" --require-approval,echo "design ready: no live tree")
