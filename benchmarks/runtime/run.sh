#!/usr/bin/env bash
# Build and time every benchmark in Raven, C, Go, Rust, and Python.
# RAVEN selects the compiler (default: raven on PATH). CC selects a 64-bit C
# compiler (default: clang). SKIP_PYTHON=1 skips Python.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
out="${OUT:-$here/.build}"
raven="${RAVEN:-raven}"
cc="${CC:-clang}"
if [ -z "${CC:-}" ] && ! command -v clang >/dev/null 2>&1 && [ -x "/c/Program Files/LLVM/bin/clang.exe" ]; then
    cc="/c/Program Files/LLVM/bin/clang.exe"
fi
mkdir -p "$out"

ms() { local s e; s=$(date +%s%N); "$@" > /dev/null; e=$(date +%s%N); echo $(( (e - s) / 1000000 )); }
best() {
    [ -f "$1" ] || { echo failed; return; }
    local b=999999999 t
    for _ in 1 2 3; do t=$(ms "$@"); [ "$t" -lt "$b" ] && b=$t; done
    echo "$b"
}

printf "%-10s %10s %10s %10s %10s %10s %10s\n" bench raven c go rust python raven-build
for f in "$here"/raven/*.rv; do
    n=$(basename "$f" .rv)
    rm -f "$out/$n-rv.exe" "$out/$n-c.exe" "$out/$n-go.exe" "$out/$n-rs.exe"
    rb=$(ms "$raven" build "$f" -o "$out/$n-rv.exe")
    [ -f "$out/$n-rv.exe" ] || rb=failed
    "$cc" -O2 -o "$out/$n-c.exe" "$here/c/$n.c" -lm 2>/dev/null || "$cc" -O2 -o "$out/$n-c.exe" "$here/c/$n.c" 2>/dev/null || rm -f "$out/$n-c.exe"
    (cd "$here/go" && go build -o "$out/$n-go.exe" "$n.go")
    rustc -C opt-level=3 -o "$out/$n-rs.exe" "$here/rust/$n.rs" 2>/dev/null
    r=$(best "$out/$n-rv.exe")
    c=$(best "$out/$n-c.exe")
    g=$(best "$out/$n-go.exe")
    rs=$(best "$out/$n-rs.exe")
    if [ "${SKIP_PYTHON:-0}" = 1 ]; then p=-; else p=$(ms python "$here/python/$n.py"); fi
    printf "%-10s %10s %10s %10s %10s %10s %10s\n" "$n" "$r" "$c" "$g" "$rs" "$p" "$rb"
done
echo "(times in ms, best of 3; python single run; raven-build is compile time)"
