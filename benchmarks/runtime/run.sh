#!/usr/bin/env bash
# Build and time every benchmark in Raven, C, Go, and Python.
# RAVEN selects the compiler (default: raven on PATH). SKIP_PYTHON=1 skips Python.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
out="${OUT:-$here/.build}"
raven="${RAVEN:-raven}"
mkdir -p "$out"

ms() { local s e; s=$(date +%s%N); "$@" > /dev/null; e=$(date +%s%N); echo $(( (e - s) / 1000000 )); }
best() {
    local b=999999999 t
    for _ in 1 2 3; do t=$(ms "$@"); [ "$t" -lt "$b" ] && b=$t; done
    echo "$b"
}

printf "%-10s %10s %10s %10s %10s %10s\n" bench raven c go python raven-build
for f in "$here"/raven/*.rv; do
    n=$(basename "$f" .rv)
    rm -f "$out/$n-rv.exe"
    rb=$(ms "$raven" build "$f" -o "$out/$n-rv.exe")
    [ -f "$out/$n-rv.exe" ] || rb=failed
    gcc -O2 -o "$out/$n-c.exe" "$here/c/$n.c" 2>/dev/null
    (cd "$here/go" && go build -o "$out/$n-go.exe" "$n.go")
    if [ -f "$out/$n-rv.exe" ]; then r=$(best "$out/$n-rv.exe"); else r=-; fi
    c=$(best "$out/$n-c.exe")
    g=$(best "$out/$n-go.exe")
    if [ "${SKIP_PYTHON:-0}" = 1 ]; then p=-; else p=$(ms python "$here/python/$n.py"); fi
    printf "%-10s %10s %10s %10s %10s %10s\n" "$n" "$r" "$c" "$g" "$p" "$rb"
done
echo "(times in ms, best of 3; python single run; raven-build is compile time)"
