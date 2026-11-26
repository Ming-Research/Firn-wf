# firn's canonical checks. `make check` is the gate a revision passes before
# it merges into main; CI runs the same targets, split among parallel jobs
# (.github/workflows/check.yml), which `make gate-sync` keeps complete.

PY ?= python3

# Every path is relative to this Makefile, so each target works from any
# working directory.
ROOT := $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
BUILD := $(ROOT)/build

# The Redis 7.2.16 source archive, checked on reuse, and the reference tools
# built from it (tests/redis-suite/reference.sh) are shared between suite runs.
REDIS_COMPAT_CACHE ?= $(BUILD)/redis-compat-cache

# The live design trees in Design-skill's older form: every root node file
# directly under design/ except the log and the constitution, so a tree is
# linted in the same change that adds it. firn's tree is now in the new form
# (design/decisions/ and design/principles/, whose forms the engine of
# wf-agents checks) and no older node file is left, so this is empty and
# design-lint and design-ready pass by saying there is no live tree;
# design/log.md stays as the record of the older tree's changes.
DESIGN_TREES := $(filter-out log constitution,$(basename $(notdir $(wildcard $(ROOT)/design/*.md))))

# The revision a design-tree change is reviewed against. CI selects it per
# event with design/skill/review-base.sh.
DESIGN_REVIEW_BASE ?= origin/main

# The pinned Whitefoot compiler: whitefoot.pin, make compiler, pin-ready and
# WHITEFOOTC come from the whitefoot-kit submodule, shared with the other
# projects written in Whitefoot (whitefoot-kit/downstream.md).
include $(ROOT)/whitefoot-kit/whitefoot.mk

# The tests, as CI and the engine of wf-agents name them: each a target that
# CI runs in a step by itself through whitefoot-kit/ci/run-tests.py, which
# reports it as JUnit XML (whitefoot-kit/downstream.md, "Tests in CI").
include $(ROOT)/whitefoot-kit/tests.mk

# firn is one module program; any of its sources changes the build, Halo's
# among them, which deps/halo-wf brings.
FIRN_GRAPH := $(ROOT)/firn/modules.wfg
FIRN_SOURCES := $(shell find $(ROOT)/firn $(ROOT)/deps/halo-wf/lib -name '*.wf' -o -name '*.wfm' -o -name '*.wfg' 2>/dev/null)

# The Redis suite runs as this many run.sh shards side by side.
SUITE_SHARDS ?= 2

.PHONY: check firn firn-test test list-ceiling redis-suite aof-sync-fault firn-lto design-lint design-ready gate-sync safety-witnesses network-case bench-client-case redis-suite-unit

# The gate's targets: check.yml runs each in a job of its own, which
# gate-sync requires.
CHECK_TARGETS := compiler firn test list-ceiling redis-suite design-lint gate-sync safety-witnesses

check: $(CHECK_TARGETS)

# The gate's targets but the compiler's download, which is a job's setup; the
# target of check.yml's aof-sync-fault job; and the two checks of
# design-readiness.yml.
TESTS := $(filter-out compiler,$(CHECK_TARGETS)) aof-sync-fault design-ready pin-ready

# A single test inside a whole one, named on its own (whitefoot-kit's
# tests.mk, TEST_FAMILIES): for a name that matches PATTERN, a word
# PATTERN=TARGET runs `make TARGET TEST_NAME=<name>`, and the results name it
# exactly as given. The whole names above stay what the full check runs.
#   network::<case>        a case of the cargo test `network` (tests/), by its
#                          path, such as network::rewrite_faults::<name>
#   bench-client::<case>   a test of the benchmark client
#                          (research/experiments/redis-bench/workload)
#   redis-suite/<unit>     a unit of the Redis suite, such as
#                          redis-suite/unit/type/hash
TEST_FAMILIES := network::*=network-case bench-client::*=bench-client-case redis-suite/*=redis-suite-unit

# How many times run.sh runs a unit of the Redis suite again past a test that
# timed out. The full check keeps run.sh's 5; the fast workflow sets 1.
SUITE_RETRIES ?= 5

# Real server bug patterns: each faulty transcription must receive its named
# source diagnostic and each corrected program must build with this pin.
safety-witnesses: compiler $(PIN)
	PY="$(PY)" sh $(ROOT)/research/experiments/safety-witnesses/check.sh --compiler "$(WHITEFOOTC)" --out "$(BUILD)/safety-witnesses"

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

# Keep ptrace and process-stop experiments out of the ordinary suite's load:
# its continuous-write rewrite cases must not compete with /proc stop polling.
# Both selections are required; the serial selection must run all four cases.
# The tests of the benchmark client (research/experiments/redis-bench/workload)
# run last: the benchmark's numbers are claims of the research records.
# list-ceiling is a test of its own, which the gate runs beside this one.
test: firn-test
	@redis_bin=$$(REDIS_COMPAT_CACHE="$(REDIS_COMPAT_CACHE)" $(ROOT)/tests/redis-suite/reference.sh) && \
	FIRN=$(BUILD)/firn-test REDIS_SERVER="$$redis_bin/redis-server" \
	  cargo test --manifest-path $(ROOT)/tests/Cargo.toml --locked -- --skip rewrite_faults:: --show-output
	@redis_bin=$$(REDIS_COMPAT_CACHE="$(REDIS_COMPAT_CACHE)" $(ROOT)/tests/redis-suite/reference.sh) || exit $$?; \
	log=$$(mktemp) || exit $$?; trap 'rm -f "$$log"' EXIT; \
	FIRN=$(BUILD)/firn-test REDIS_SERVER="$$redis_bin/redis-server" \
	  cargo test --manifest-path $(ROOT)/tests/Cargo.toml --locked --test network rewrite_faults:: -- \
	  --test-threads=1 --nocapture >"$$log" 2>&1; status=$$?; cat "$$log"; \
	[ $$status -eq 0 ] && grep -q '^test result: ok\. 4 passed' "$$log"
	cargo test --manifest-path $(ROOT)/research/experiments/redis-bench/workload/Cargo.toml --locked \
	  --target-dir $(BUILD)/workload-target

# One network case, named network::<its path in the cargo test `network`>.
# It runs alone, on one test thread, as the serial rewrite_faults:: cases
# need; a name no case has, or an ignored case, runs nothing and fails.
network-case: firn-test
	@case=$${TEST_NAME#network::}; \
	[ -n "$$case" ] && [ "$$case" != "$$TEST_NAME" ] || { echo "network-case: name a case as network::<path>" >&2; exit 2; }; \
	redis_bin=$$(REDIS_COMPAT_CACHE="$(REDIS_COMPAT_CACHE)" $(ROOT)/tests/redis-suite/reference.sh) || exit $$?; \
	log=$$(mktemp) || exit $$?; trap 'rm -f "$$log"' EXIT; \
	FIRN=$(BUILD)/firn-test REDIS_SERVER="$$redis_bin/redis-server" \
	  cargo test --manifest-path $(ROOT)/tests/Cargo.toml --locked --test network -- \
	  --exact "$$case" --test-threads=1 --nocapture >"$$log" 2>&1; status=$$?; cat "$$log"; \
	[ $$status -eq 0 ] || exit $$status; \
	grep -q '^test result: ok\. 1 passed' "$$log" \
	  || { echo "network-case: the cargo test network ran no case named $$case" >&2; exit 1; }

# One test of the benchmark client, named bench-client::<its path>.
bench-client-case:
	@case=$${TEST_NAME#bench-client::}; \
	[ -n "$$case" ] && [ "$$case" != "$$TEST_NAME" ] || { echo "bench-client-case: name a test as bench-client::<path>" >&2; exit 2; }; \
	log=$$(mktemp) || exit $$?; trap 'rm -f "$$log"' EXIT; \
	cargo test --manifest-path $(ROOT)/research/experiments/redis-bench/workload/Cargo.toml --locked \
	  --target-dir $(BUILD)/workload-target -- --exact "$$case" >"$$log" 2>&1; status=$$?; cat "$$log"; \
	[ $$status -eq 0 ] || exit $$status; \
	grep -q '^test result: ok\. 1 passed' "$$log" \
	  || { echo "bench-client-case: the benchmark client ran no test named $$case" >&2; exit 1; }

# Direct Whitefoot fixtures exercise the real private list command bodies at
# an eight-element ceiling, without a production test hook or a giant network
# dataset. The assembler copies sources only into this disposable build tree;
# the cache beside it lets a rebuild compile only what an edit changed.
LIST_CEILING_SOURCES := $(ROOT)/tests/list_ceiling.py $(ROOT)/tests/list_ceiling.wf

$(BUILD)/list-ceiling/check: $(PIN) $(WHITEFOOTC) $(FIRN_SOURCES) $(LIST_CEILING_SOURCES)
	$(PY) -B $(ROOT)/tests/list_ceiling.py --out $(BUILD)/list-ceiling
	$(WHITEFOOTC) --graph $(BUILD)/list-ceiling/firn/modules.wfg --entry list_ceiling --cache $(BUILD)/list-ceiling-cache -o $@

list-ceiling: $(BUILD)/list-ceiling/check
	$(BUILD)/list-ceiling/check

# Redis's released suite runs against the cached server build, split into
# shards that run side by side and merge into one run. Every listed pass must
# still pass; new passes are candidates to add in the same PR.
redis-suite: $(BUILD)/firn
	$(PY) -B $(ROOT)/tests/redis-suite/ratchet.py --self-test
	REDIS_COMPAT_CACHE="$(REDIS_COMPAT_CACHE)" $(ROOT)/tests/redis-suite/run-shards.sh --shards $(SUITE_SHARDS) --out "$(BUILD)/redis-suite" --retries $(SUITE_RETRIES) --known-hangs $(ROOT)/tests/redis-suite/hung.tsv firn "$(BUILD)/firn"
	$(PY) -B $(ROOT)/tests/redis-suite/ratchet.py check "$(BUILD)/redis-suite/tests.tsv" --passing $(ROOT)/tests/redis-suite/passing.tsv --unstable $(ROOT)/tests/redis-suite/unstable.tsv

# One unit of the suite, named redis-suite/<unit> as the suite lists it, such
# as redis-suite/unit/type/hash: run by itself, and checked against the
# ratchet's tests of that unit alone.
redis-suite-unit: $(BUILD)/firn
	@unit=$${TEST_NAME#redis-suite/}; \
	[ -n "$$unit" ] && [ "$$unit" != "$$TEST_NAME" ] || { echo "redis-suite-unit: name a unit as redis-suite/<unit>" >&2; exit 2; }; \
	case "$$unit" in *[!A-Za-z0-9_/.-]*|*..*) echo "redis-suite-unit: $$unit is not a unit's name" >&2; exit 2 ;; esac; \
	out="$(BUILD)/redis-suite-units/$$unit"; rm -rf "$$out"; \
	REDIS_COMPAT_CACHE="$(REDIS_COMPAT_CACHE)" $(ROOT)/tests/redis-suite/run.sh --out "$$out" --units "$$unit" \
	  --retries $(SUITE_RETRIES) --known-hangs $(ROOT)/tests/redis-suite/hung.tsv firn "$(BUILD)/firn" || exit $$?; \
	grep -qxF "$$unit" "$$out/suite-units.txt" || { echo "redis-suite-unit: the suite has no unit $$unit" >&2; exit 1; }; \
	$(PY) -B $(ROOT)/tests/redis-suite/ratchet.py check "$$out/tests.tsv" --unit "$$unit" \
	  --passing $(ROOT)/tests/redis-suite/passing.tsv --unstable $(ROOT)/tests/redis-suite/unstable.tsv

# The append-only file's sync failing on a real device, firn against Redis
# 7.2.16: the sync and rewrite fault cases of tests/aof_sync_fault.rs. Not part of `make check`:
# the cases are ignored there because they need a device only root can make,
# so the aof-sync-fault job of .github/workflows/check.yml makes it first with
#   sudo tests/aof_sync_fault_device.sh setup MOUNT UID:GID
# and exports FIRN_SYNC_FAULT_MOUNT=MOUNT and REDIS_SERVER, the path of a
# redis-server 7.2.16. The cases fail and heal the one device, so they run on
# one test thread, one after the other. Without one of the variables, or
# unless all eight cases ran and passed, the target fails. The output is kept so
# that each case's account of what every server did shows, and the pass is
# read from the run itself.
aof-sync-fault: firn-test
	@test -n "$(FIRN_SYNC_FAULT_MOUNT)" || { echo "aof-sync-fault: set FIRN_SYNC_FAULT_MOUNT to the mount tests/aof_sync_fault_device.sh setup made" >&2; exit 1; }
	@test -d "$(FIRN_SYNC_FAULT_MOUNT)" || { echo "aof-sync-fault: FIRN_SYNC_FAULT_MOUNT=$(FIRN_SYNC_FAULT_MOUNT) is not a directory; run tests/aof_sync_fault_device.sh setup first" >&2; exit 1; }
	@test -n "$(REDIS_SERVER)" || { echo "aof-sync-fault: set REDIS_SERVER to a redis-server 7.2.16, the oracle" >&2; exit 1; }
	@test -x "$(REDIS_SERVER)" || { echo "aof-sync-fault: REDIS_SERVER=$(REDIS_SERVER) is not an executable" >&2; exit 1; }
	@log=$$(mktemp) && trap 'rm -f "$$log"' EXIT && \
	FIRN=$(BUILD)/firn-test FIRN_SYNC_FAULT_MOUNT="$(FIRN_SYNC_FAULT_MOUNT)" REDIS_SERVER="$(REDIS_SERVER)" \
	  cargo test --manifest-path $(ROOT)/tests/Cargo.toml --locked --test network -- \
	  --ignored --exact --nocapture --test-threads=1 \
	  aof_sync_fault::aof_sync_failure_refuses_writes_and_recovers_on_a_failing_device \
	  aof_sync_fault::aof_sync_failure_keeps_a_write_acknowledged_at_a_page_end \
	  aof_sync_fault::aof_rewrite_temporary_enospc_keeps_acknowledged_state \
	  aof_sync_fault::aof_rewrite_temporary_eio_keeps_acknowledged_state \
	  aof_sync_fault::aof_rewrite_current_append_failure_observes_publication \
	  aof_sync_fault::aof_rewrite_switch_directory_sync_failure_refuses_until_recovery \
	  aof_sync_fault::aof_rewrite_switch_directory_sync_einval_completes \
	  aof_sync_fault::aof_rewrite_close_sync_failure_is_isolated_and_recovers \
	  >"$$log" 2>&1; status=$$?; cat "$$log"; \
	[ $$status -eq 0 ] && grep -q '^test result: ok\. 8 passed' "$$log"

# The server as it is measured: the program and the runtime optimized
# together under full link-time optimization.
firn-lto: $(BUILD)/firn-lto

$(BUILD)/firn-lto: $(PIN) $(WHITEFOOTC) $(FIRN_SOURCES)
	$(WHITEFOOTC) --full-lto --graph $(FIRN_GRAPH) --entry firn -o $@

# Design-skill's own tests, then the lint of every live tree in the older
# form. With none left (see DESIGN_TREES), both targets print that there is
# no live tree and pass.
design-lint:
	@$(PY) -B -m unittest discover -s $(ROOT)/design/skill -p 'test_lint.py'
	@$(if $(DESIGN_TREES),$(PY) -B $(ROOT)/design/skill/lint.py --root $(ROOT)/design --trees $(DESIGN_TREES) --base "$(DESIGN_REVIEW_BASE)",echo "design lint: no live tree")

design-ready:
	@$(if $(DESIGN_TREES),$(PY) -B $(ROOT)/design/skill/lint.py --root $(ROOT)/design --trees $(DESIGN_TREES) --base "$(DESIGN_REVIEW_BASE)" --require-approval,echo "design ready: no live tree")

# CI runs every target of the gate: each step of check.yml that runs one,
# `make TARGET` for setup or `whitefoot-kit/ci/run-tests.py TARGET` for a
# test, is one line, and the targets they name, less the host setup of `make
# toolchain` and the aof-sync-fault job's own target, are exactly
# CHECK_TARGETS. The comparison must also refuse a workflow without the
# redis-suite step and one with an extra `make firn-lto`.
gate-sync:
	@workflow=$(ROOT)/.github/workflows/check.yml; \
	want="$$(printf '%s\n' $(CHECK_TARGETS) | sort -u | tr '\n' ' ')"; \
	runs() { sed -n -E 's#^ +run: (make|whitefoot-kit/ci/run-tests\.py) ([a-z-]+)$$#\2#p' | grep -vx -e toolchain -e aof-sync-fault | sort -u | tr '\n' ' '; }; \
	[ "$$(runs < $$workflow)" = "$$want" ] || { echo "gate-sync: check.yml runs [$$(runs < $$workflow)] but make check runs [$$want]" >&2; exit 1; }; \
	[ "$$(grep -v 'run: whitefoot-kit/ci/run-tests.py redis-suite$$' $$workflow | runs)" != "$$want" ] \
		&& [ "$$({ cat $$workflow; echo '        run: make firn-lto'; } | runs)" != "$$want" ] \
		|| { echo "gate-sync: the comparison accepted a changed workflow" >&2; exit 1; }; \
	echo "gate-sync: check.yml runs every target of make check: $$want"
