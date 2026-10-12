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

.PHONY: check firn firn-test test redis-suite aof-sync-fault firn-lto design-lint design-ready

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

# The append-only file's sync failing on a real device, firn against Redis
# 7.2.16. Not part of `make check`: the case is ignored there because it needs
# a device only root can make, so the aof-sync-fault job of
# .github/workflows/check.yml makes it first with
#   sudo tests/aof_sync_fault_device.sh setup MOUNT UID:GID
# and exports FIRN_SYNC_FAULT_MOUNT=MOUNT and REDIS_SERVER, the path of a
# redis-server 7.2.16. Without one of them, or without a case that passed, the
# target fails. The output is kept so that the case's account of the branch
# each server took shows, and the pass is read from the run itself.
aof-sync-fault: firn-test
	@test -n "$(FIRN_SYNC_FAULT_MOUNT)" || { echo "aof-sync-fault: set FIRN_SYNC_FAULT_MOUNT to the mount tests/aof_sync_fault_device.sh setup made" >&2; exit 1; }
	@test -d "$(FIRN_SYNC_FAULT_MOUNT)" || { echo "aof-sync-fault: FIRN_SYNC_FAULT_MOUNT=$(FIRN_SYNC_FAULT_MOUNT) is not a directory; run tests/aof_sync_fault_device.sh setup first" >&2; exit 1; }
	@test -n "$(REDIS_SERVER)" || { echo "aof-sync-fault: set REDIS_SERVER to a redis-server 7.2.16, the oracle" >&2; exit 1; }
	@test -x "$(REDIS_SERVER)" || { echo "aof-sync-fault: REDIS_SERVER=$(REDIS_SERVER) is not an executable" >&2; exit 1; }
	@log=$$(mktemp) && trap 'rm -f "$$log"' EXIT && \
	FIRN=$(BUILD)/firn-test FIRN_SYNC_FAULT_MOUNT="$(FIRN_SYNC_FAULT_MOUNT)" REDIS_SERVER="$(REDIS_SERVER)" \
	  cargo test --manifest-path $(ROOT)/tests/Cargo.toml --locked --test network -- \
	  --ignored --exact --nocapture aof_sync_fault::aof_sync_failure_refuses_writes_and_recovers_on_a_failing_device \
	  >"$$log" 2>&1; status=$$?; cat "$$log"; \
	[ $$status -eq 0 ] && grep -q '^test result: ok\. 1 passed' "$$log"

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
