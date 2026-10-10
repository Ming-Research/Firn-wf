#!/bin/sh
# Explicit acceptance probes, never a dependency of the canonical gate.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
OUT=${OUT:-${RUNNER_TEMP:-/tmp}/frozen-dataset-prototype}
RELEASE=$(sed -n 's/^release = //p' "$ROOT/whitefoot.pin")
WFC=${WFC:-$ROOT/build/whitefoot/$RELEASE/whitefootc}
mkdir -p "$OUT"
printf 'program\tphase\texit_status\n' > "$OUT/results.tsv"

status=0
for program in control revoke-rejected managed-atomic-control managed-next-nested-rejected managed-next-target-rejected; do
    if timeout 300 "$WFC" -o "$OUT/$program" \
        "$ROOT/research/experiments/frozen-dataset-prototype/$program.wf" \
        > "$OUT/$program-compile.log" 2>&1; then
        compiled=0
    else
        compiled=$?
    fi
    cat "$OUT/$program-compile.log"
    printf '%s\tcompile\t%s\n' "$program" "$compiled" >> "$OUT/results.tsv"
    printf '%s compile exit status: %s\n' "$program" "$compiled"

    if [ "$program" = revoke-rejected ]; then
        printf '%s\trun\tnot-run-negative-source\n' "$program" >> "$OUT/results.tsv"
        # Only a source rejection at the registered write counts. In particular,
        # timeout, signal termination and compiler failure are not verdicts.
        if [ "$compiled" -eq 1 ] && \
            grep -Eq 'revoke-rejected\.wf:3:7: error\[TYPE-2\]: ReadonlyWriteTarget' "$OUT/$program-compile.log" && \
            grep -Fq 'set root^.inner = move empty;' "$OUT/$program-compile.log" && \
            [ "$(grep -Ec 'error\[|compiler failure' "$OUT/$program-compile.log")" -eq 1 ]; then
            printf '%s\texpectation\tpass-TYPE-2\n' "$program" >> "$OUT/results.tsv"
        else
            printf '%s\texpectation\tfail-expected-TYPE-2\n' "$program" >> "$OUT/results.tsv"
            printf '%s: expected TYPE-2 at set root^.inner = move empty;\n' "$program"
            if [ "$status" -eq 0 ]; then
                case "$compiled" in
                    0|1) status=81 ;;
                    *) status=$compiled ;;
                esac
            fi
        fi
        continue
    fi

    # Both historical managed-next-*-rejected names now designate positives.
    if [ "$compiled" -ne 0 ]; then
        printf '%s\trun\tnot-run-compile-failed\n' "$program" >> "$OUT/results.tsv"
        printf '%s\texpectation\tfail-expected-compile\n' "$program" >> "$OUT/results.tsv"
        if [ "$status" -eq 0 ]; then
            status=$compiled
        fi
        continue
    fi
    drivers=1
    case "$program" in
        managed-next-*) drivers=2 ;;
    esac
    if WF_DRIVERS="$drivers" WF_WORKERS=1 timeout 60 "$OUT/$program" \
        > "$OUT/$program-run.log" 2>&1; then
        ran=0
    else
        ran=$?
    fi
    cat "$OUT/$program-run.log"
    printf '%s\trun\t%s\n' "$program" "$ran" >> "$OUT/results.tsv"
    printf '%s run exit status: %s\n' "$program" "$ran"
    if [ "$ran" -eq 0 ]; then
        printf '%s\texpectation\tpass-exit-0\n' "$program" >> "$OUT/results.tsv"
    else
        printf '%s\texpectation\tfail-expected-exit-0\n' "$program" >> "$OUT/results.tsv"
        if [ "$status" -eq 0 ]; then
            status=$ran
        fi
    fi
done

exit "$status"
