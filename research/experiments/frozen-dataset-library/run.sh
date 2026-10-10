#!/bin/sh
# Temporary CI experiment; does not replace the project's canonical gate.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
HERE=$ROOT/research/experiments/frozen-dataset-library
OUT=${OUT:-${RUNNER_TEMP:-/tmp}/frozen-dataset-library}
WFC=${WFC:-$ROOT/build/whitefoot/wf-0c0a2eda83ae/whitefootc}
mkdir -p "$OUT"
printf 'case\tphase\texit_status\texpected\twall_seconds\n' > "$OUT/results.tsv"

compile_case() {
    name=$1
    keys=$2
    operations=$3
    live_flag=0
    [ "$4" = True ] && live_flag=1
    copy_flag=0
    [ "$5" = True ] && copy_flag=1
    cat > "$OUT/$name.wf" <<EOF
const initial_keys: u64 = ${keys}_u64;

const operation_count: u64 = ${operations}_u64;

const wrong_live: u8 = ${live_flag}_u8;

const wrong_copy: u8 = ${copy_flag}_u8;
EOF
    code=0
    /usr/bin/time -p -o "$OUT/$name.compile.time" \
        timeout 300 "$WFC" -o "$OUT/$name" \
        "$HERE/map.wf" "$HERE/witness.wf" "$HERE/main.wf" "$OUT/$name.wf" \
        > "$OUT/$name.compile.log" 2>&1 || code=$?
    cat "$OUT/$name.compile.log"
    seconds=$(awk '$1 == "real" { print $2 }' "$OUT/$name.compile.time")
    printf '%s\tcompile\t%s\t0\t%s\n' "$name" "$code" "$seconds" >> "$OUT/results.tsv"
    if [ "$code" -ne 0 ]; then
        printf 'STOP: %s compiler status %s; inspect and minimize, do not work around.\n' "$name" "$code"
        exit "$code"
    fi
}

sample() {
    name=$1
    repetition=$2
    expected=$3
    code=0
    /usr/bin/time -p -o "$OUT/$name.$repetition.time" \
        env WF_DRIVERS=2 WF_WORKERS=1 timeout 60 "$OUT/$name" \
        > "$OUT/$name.$repetition.stdout" 2> "$OUT/$name.$repetition.stderr" || code=$?
    seconds=$(awk '$1 == "real" { print $2 }' "$OUT/$name.$repetition.time")
    printf '%s\trun-%s\t%s\t%s\t%s\n' "$name" "$repetition" "$code" "$expected" "$seconds" >> "$OUT/results.tsv"
    cat "$OUT/$name.$repetition.stdout" "$OUT/$name.$repetition.stderr"
    if [ "$code" -ne "$expected" ]; then
        printf 'FAIL: %s expected %s, observed %s\n' "$name" "$expected" "$code"
        exit 81
    fi
    # Require all ten distinct fields in order and an independently matching
    # reported status. A crash, empty output or another negative is no pass.
    if ! awk -F= -v expected="$expected" '
        length($0) != 23 || $0 !~ /^[0-9][0-9]=[0-9]+$/ { bad=1 }
        $1 + 0 != NR - 1 { bad=1 }
        $1 == "08" && $2 + 0 != expected { bad=1 }
        END { exit (bad || NR != 10) }
    ' "$OUT/$name.$repetition.stdout"; then
        printf 'FAIL: %s missing or malformed measurement report\n' "$name"
        exit 82
    fi
}

compile_case smoke 64 6 False False
for repetition in 1 2 3; do
    sample smoke "$repetition" 0
done
if ! awk -F '\t' '
    $1 == "smoke" && $2 ~ /^run-/ {
        value=$5 + 0
        if (count == 0 || value < low) low=value
        if (count == 0 || value > high) high=value
        count++
    }
    END {
        printf "small-sample wall_seconds min=%s max=%s spread=%s\n", low, high, high-low
        exit (count != 3 || high > 10 || high-low > 5)
    }
' "$OUT/results.tsv"; then
    printf 'STOP: inspect the small sample before choosing a larger CI run.\n'
    exit 83
fi

compile_case small 1024 12 False False
sample small 1 0
compile_case large 8192 12 False False
sample large 1 0
compile_case changes 8192 24 False False
sample changes 1 0
compile_case wrong-live 8192 12 True False
sample wrong-live 1 70
compile_case wrong-copy 8192 12 False True
sample wrong-copy 1 72

cat "$OUT/results.tsv"
