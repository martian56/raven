#!/usr/bin/env bash
# Generate the compile-time benchmark corpus: a set of valid Raven v2
# modules plus an entry file that imports and exercises every one of them.
#
# The corpus is deterministic. Running this script reproduces byte for
# byte the committed .rv files under corpus/. Re-run after changing the
# shape or count of modules, then commit the regenerated output.
#
# Usage: benchmarks/compile-time/generate.sh [module_count]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORPUS_DIR="$SCRIPT_DIR/corpus"
MODULES_DIR="$CORPUS_DIR/modules"
COUNT="${1:-62}"

rm -rf "$CORPUS_DIR"
mkdir -p "$MODULES_DIR"

# Each module emits a self contained mix of language constructs whose
# concrete types are keyed off the module index, so the type checker and
# monomorphizer see distinct instantiations rather than one repeated shape.
emit_module() {
    local i="$1"
    local f="$MODULES_DIR/mod${i}.rv"
    local base=$(( (i * 7) % 23 + 1 ))

    cat > "$f" <<EOF
import std/string

struct Vec2_${i} { x: Int, y: Int }

impl Vec2_${i} {
    fun add(self, o: Vec2_${i}) -> Vec2_${i} {
        return Vec2_${i} { x: self.x + o.x, y: self.y + o.y }
    }
    fun scale(self, k: Int) -> Vec2_${i} {
        return Vec2_${i} { x: self.x * k, y: self.y * k }
    }
    fun magnitude_sq(self) -> Int {
        return self.x * self.x + self.y * self.y
    }
}

struct Node_${i} { id: Int, weight: Int, label: String }

impl Node_${i} {
    fun heavier(self, other: Node_${i}) -> Bool {
        return self.weight > other.weight
    }
    fun retag(self, n: String) -> Node_${i} {
        return Node_${i} { id: self.id, weight: self.weight, label: n }
    }
}

enum Shape_${i} {
    Circle,
    Square,
    Triangle,
    Hexagon,
}

fun classify_${i}(s: Shape_${i}) -> Int {
    return match s {
        Circle -> ${base},
        Square -> ${base} * 2,
        Triangle -> ${base} * 3,
        Hexagon -> ${base} * 6,
    }
}

enum Outcome_${i} {
    Pass,
    Fail,
    Skip,
}

fun grade_${i}(score: Int) -> Outcome_${i} {
    if score >= 60 {
        return Outcome_${i}.Pass
    }
    if score >= 30 {
        return Outcome_${i}.Skip
    }
    return Outcome_${i}.Fail
}

fun grade_points_${i}(o: Outcome_${i}) -> Int {
    return match o {
        Pass -> 100,
        Fail -> 0,
        Skip -> 50,
    }
}

struct Pair_${i}<T> { first: T, second: T }

impl<T> Pair_${i}<T> {
    fun left(self) -> T = self.first
    fun right(self) -> T = self.second
}

struct Wrap_${i}<T> { inner: T }

impl<T> Wrap_${i}<T> {
    fun get(self) -> T = self.inner
    fun mapped<U>(self, f: fun(T) -> U) -> U = f(self.inner)
}

trait Score_${i} {
    fun score(self) -> Int
}

impl Score_${i} for Node_${i} {
    fun score(self) -> Int = self.id * 10 + self.weight
}

impl Score_${i} for Vec2_${i} {
    fun score(self) -> Int = self.magnitude_sq()
}

fun ranked_${i}<T: Score_${i}>(x: T) -> Int = x.score()

fun identity_${i}<T>(x: T) -> T = x

fun maybe_${i}(n: Int) -> Option<Int> {
    if n > ${base} {
        return Some(n * 2)
    }
    return None
}

fun unwrap_or_${i}(x: Option<Int>, fallback: Int) -> Int {
    return match x {
        None -> fallback,
        Some(v) -> v,
    }
}

fun accumulate_${i}(limit: Int) -> Int {
    let total = 0
    let i = 0
    while i < limit {
        if i % 2 == 0 {
            total = total + i
        } else {
            total = total - 1
        }
        i = i + 1
    }
    return total
}

fun sweep_${i}(limit: Int) -> Int {
    let total = 0
    for k in 0..limit {
        total = total + classify_${i}(Shape_${i}.Triangle) + k
    }
    return total
}

fun fib_${i}(n: Int) -> Int {
    if n < 2 {
        return n
    }
    return fib_${i}(n - 1) + fib_${i}(n - 2)
}

fun compute_${i}(seed: Int) -> Int {
    let v = Vec2_${i} { x: seed, y: seed + ${base} }
    let w = v.add(Vec2_${i} { x: 2, y: 3 }).scale(2)
    let node = Node_${i} { id: seed, weight: ${base}, label: "n" }
    let tagged = node.retag("tagged")
    let p = Pair_${i} { first: w.x, second: w.y }
    let q = Pair_${i} { first: tagged.label, second: "z" }
    let boxed = Wrap_${i} { inner: seed }
    let doubled = boxed.mapped(fun(x: Int) -> Int = x * 2)
    let big = boxed.mapped(fun(x: Int) -> Bool = x > ${base})
    let m = unwrap_or_${i}(maybe_${i}(seed), 0)
    let g = grade_points_${i}(grade_${i}(seed))
    let r = ranked_${i}(tagged) + ranked_${i}(v)
    let extra = if big { 1 } else { 0 }
    return p.left() + p.right() + w.magnitude_sq() + identity_${i}(doubled) +
        m + g + r + accumulate_${i}(${base}) + sweep_${i}(${base}) +
        fib_${i}(8) + classify_${i}(Shape_${i}.Hexagon) + extra +
        q.left().length()
}
EOF
}

ENTRY="$CORPUS_DIR/main.rv"
{
    for ((i = 0; i < COUNT; i++)); do
        echo "import \"./modules/mod${i}\" { compute_${i} }"
    done
    echo ""
    echo "fun main() {"
    echo "    let total = 0"
    for ((i = 0; i < COUNT; i++)); do
        echo "    total = total + compute_${i}(${i} + 1)"
    done
    echo "    print(total)"
    echo "}"
} > "$ENTRY"

for ((i = 0; i < COUNT; i++)); do
    emit_module "$i"
done

LINES=$(cat "$ENTRY" "$MODULES_DIR"/*.rv | wc -l)
echo "generated $COUNT modules plus entry, $LINES total lines under $CORPUS_DIR"
