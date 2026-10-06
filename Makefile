# firn's canonical checks. `make check` is the gate a revision passes before
# it merges into main; CI runs the same targets.

PY ?= python3

# Every path is relative to this Makefile, so each target works from any
# working directory.
ROOT := $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
BUILD := $(ROOT)/build

# Redis's source archive is shared between suite runs and checked on reuse.
REDIS_COMPAT_CACHE ?= $(BUILD)/redis-compat-cache

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
# build/whitefoot/<release>/. Only a work branch may pin an experiment
# release (AGENTS.md, rule 4; make pin-ready refuses it), and
# `make WHITEFOOTC=<path> firn` builds with a locally built compiler instead.
PIN := $(ROOT)/whitefoot.pin
PIN_LINE := ^release = wf-(exp-)?[0-9a-f]{12}$$
RELEASE := $(shell sed -n -E 's/^release = (wf-(exp-)?[0-9a-f]{12})$$/\1/p' $(PIN) 2>/dev/null)
RELEASE_COMMIT := $(lastword $(subst -, ,$(RELEASE)))
RELEASES := https://github.com/Ming-Research/Whitefoot/releases/download
HOST := $(shell uname -s)-$(shell uname -m)
ASSET := $(if $(filter Linux-x86_64,$(HOST)),whitefootc-linux-x86_64.tar.gz,$(if $(filter Darwin-arm64,$(HOST)),whitefootc-macos-arm64.tar.gz))
WHITEFOOT := $(BUILD)/whitefoot/$(RELEASE)
PINNED_WHITEFOOTC := $(WHITEFOOT)/whitefootc
WHITEFOOTC := $(PINNED_WHITEFOOTC)

# firn is one module program; any of its sources changes the build.
FIRN_GRAPH := $(ROOT)/firn/modules.wfg
FIRN_SOURCES := $(shell find $(ROOT)/firn -name '*.wf' -o -name '*.wfm' -o -name '*.wfg')

.PHONY: check compiler firn firn-test test redis-suite firn-lto design-lint design-ready pin-ready

check: compiler firn test redis-suite design-lint

compiler: $(PINNED_WHITEFOOTC)

$(PIN):
	@echo "whitefoot.pin is missing; it names the Whitefoot compiler release (AGENTS.md, Upgrading Whitefoot)" >&2
	@exit 1

$(PINNED_WHITEFOOTC): $(PIN)
	@test "$$(grep -c '' $(PIN))" = 1 && grep -qE '$(PIN_LINE)' $(PIN) || { echo "whitefoot.pin must hold exactly one line: release = wf-<12-character commit hash>" >&2; exit 1; }
	@test -n "$(ASSET)" || { echo "Whitefoot publishes no compiler for $(HOST)" >&2; exit 1; }
	@rm -rf $(WHITEFOOT).part && mkdir -p $(WHITEFOOT).part
	@cd $(WHITEFOOT).part && for file in $(ASSET) SHA256SUMS whitefoot-release.json; do \
		curl -fsSL --retry 3 -o $$file $(RELEASES)/$(RELEASE)/$$file || { \
			echo "cannot download $$file of $(RELEASE); make it with: gh workflow run compiler-release.yml -R Ming-Research/Whitefoot -f commit=$(RELEASE_COMMIT)$(if $(findstring wf-exp-,$(RELEASE)), -f experiment=true) (AGENTS.md, Upgrading Whitefoot)" >&2; \
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

# The network cases use the overlap lowering with every eligible call
# offered, as they did in Whitefoot's program tests.
firn-test: $(BUILD)/firn-test

$(BUILD)/firn-test: $(PIN) $(WHITEFOOTC) $(FIRN_SOURCES)
	$(WHITEFOOTC) --par --par-call-grain off --graph $(FIRN_GRAPH) --entry firn --cache $(BUILD)/firn-test-cache -o $@

test: firn-test
	FIRN=$(BUILD)/firn-test cargo test --manifest-path $(ROOT)/tests/Cargo.toml --locked

# Redis's released suite runs against the cached server build. Every listed
# pass must still pass; new passes are candidates to add in the same PR.
redis-suite: $(BUILD)/firn
	$(PY) -B $(ROOT)/tests/redis-suite/ratchet.py --self-test
	REDIS_COMPAT_CACHE="$(REDIS_COMPAT_CACHE)" $(ROOT)/tests/redis-suite/run.sh --out "$(BUILD)/redis-suite" firn "$(BUILD)/firn"
	$(PY) -B $(ROOT)/tests/redis-suite/ratchet.py check "$(BUILD)/redis-suite/tests.tsv" --passing $(ROOT)/tests/redis-suite/passing.tsv --unstable $(ROOT)/tests/redis-suite/unstable.tsv

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

# A revision bound for main pins a release of a commit on Whitefoot's main,
# never an experiment release (AGENTS.md, rule 4); CI runs this with
# design-ready on ready pull requests and main.
pin-ready:
	@if grep -q '^release = wf-exp-' $(PIN); then \
		echo "whitefoot.pin names the experiment release $(RELEASE); pin a release of a commit on Whitefoot's main before this reaches main" >&2; \
		exit 1; fi
