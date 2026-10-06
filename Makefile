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

# The pinned Whitefoot compiler: whitefoot.pin, make compiler, pin-ready and
# WHITEFOOTC come from the whitefoot-kit submodule, shared with the other
# projects written in Whitefoot (whitefoot-kit/downstream.md).
include $(ROOT)/whitefoot-kit/whitefoot.mk

# firn is one module program; any of its sources changes the build, Halo's
# among them, which deps/halo-wf brings.
FIRN_GRAPH := $(ROOT)/firn/modules.wfg
FIRN_SOURCES := $(shell find $(ROOT)/firn $(ROOT)/deps/halo-wf/lib -name '*.wf' -o -name '*.wfm' -o -name '*.wfg')

.PHONY: check firn firn-test test redis-suite firn-lto design-lint design-ready

check: compiler firn test redis-suite design-lint

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
	REDIS_COMPAT_CACHE="$(REDIS_COMPAT_CACHE)" $(ROOT)/tests/redis-suite/run.sh --out "$(BUILD)/redis-suite" --known-hangs $(ROOT)/tests/redis-suite/hung.tsv firn "$(BUILD)/firn"
	$(PY) -B $(ROOT)/tests/redis-suite/ratchet.py check "$(BUILD)/redis-suite/tests.tsv" --passing $(ROOT)/tests/redis-suite/passing.tsv --unstable $(ROOT)/tests/redis-suite/unstable.tsv

# The server as it is measured: the program and the runtime optimized
# together under full link-time optimization.
firn-lto: $(BUILD)/firn-lto

$(BUILD)/firn-lto: $(PIN) $(WHITEFOOTC) $(FIRN_SOURCES)
	$(WHITEFOOTC) --full-lto --graph $(FIRN_GRAPH) --entry firn -o $@

# Design-skill's own tests, then the lint of every live tree.
design-lint:
	@$(PY) -B -m unittest discover -s $(ROOT)/design/skill -p 'test_lint.py'
	@$(if $(DESIGN_TREES),$(PY) -B $(ROOT)/design/skill/lint.py --root $(ROOT)/design --trees $(DESIGN_TREES) --base "$(DESIGN_REVIEW_BASE)",echo "design lint: no live tree")

design-ready:
	@$(if $(DESIGN_TREES),$(PY) -B $(ROOT)/design/skill/lint.py --root $(ROOT)/design --trees $(DESIGN_TREES) --base "$(DESIGN_REVIEW_BASE)" --require-approval,echo "design ready: no live tree")
