//! Writes a large `.rv` file for parser/typechecker stress testing.

use clap::{Arg, Command};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Lines before `fun main() -> void {` + body + `}` + `main();`
static PREAMBLE: &[&str] = &[
    "// Generated long Raven program: structs, enums, impl, helpers, then main.",
    "struct GenPoint {",
    "    x: int,",
    "    y: int",
    "}",
    "struct GenItem {",
    "    id: int,",
    "    tag: string",
    "}",
    "enum GenKind {",
    "    Alpha,",
    "    Beta,",
    "    Gamma",
    "}",
    "enum GenFlag {",
    "    Off,",
    "    On",
    "}",
    "impl GenPoint {",
    "    fun sum(self) -> int {",
    "        return self.x + self.y;",
    "    }",
    "    fun with_offset(self, dx: int, dy: int) -> GenPoint {",
    "        return GenPoint { x: self.x + dx, y: self.y + dy };",
    "    }",
    "}",
    "fun add_pair(a: int, b: int) -> int {",
    "    return a + b;",
    "}",
    "fun triple(n: int) -> int {",
    "    return n * 3;",
    "}",
    "fun gen_if_chain(n: int) -> int {",
    "    if (n < 0) {",
    "        return -1;",
    "    } elseif (n == 0) {",
    "        return 0;",
    "    } else {",
    "        return 1;",
    "    }",
    "}",
    "fun gen_while_count(limit: int) -> int {",
    "    let i: int = 0;",
    "    while (i < limit) {",
    "        i = i + 1;",
    "    }",
    "    return i;",
    "}",
    "fun gen_for_sum(n: int) -> int {",
    "    let acc: int = 0;",
    "    for (let j: int = 0; j < n; j = j + 1) {",
    "        acc = acc + j;",
    "    }",
    "    return acc;",
    "}",
    "let global_seed: int = 7;",
    "let global_label: string = \"long-code\";",
    "/* exported binding (module syntax) */",
    "export let exported_flag: int = 42;",
];

const PATTERN: usize = 26;

fn write_preamble(w: &mut impl Write) -> std::io::Result<usize> {
    let mut n = 0;
    for line in PREAMBLE {
        writeln!(w, "{line}")?;
        n += 1;
    }
    Ok(n)
}

fn gen_kind_variant(i: usize) -> &'static str {
    match i % 3 {
        0 => "Alpha",
        1 => "Beta",
        _ => "Gamma",
    }
}

fn write_body_line(w: &mut impl Write, i: usize) -> std::io::Result<()> {
    match i % PATTERN {
        0 => writeln!(w, "    let m{i}: int = {i} + global_seed;"),
        1 => writeln!(w, "    let f{i}: float = {i}.0;"),
        2 => writeln!(w, "    let s{i}: string = \"x\";"),
        3 => writeln!(w, "    let a{i}: int[] = [{i}, {i} + 1, 2];"),
        4 => writeln!(w, "    let b{i}: bool = {i} % 2 == 0;"),
        5 => writeln!(w, "    let o{i}: bool = true or false;"),
        6 => writeln!(w, "    let n{i}: bool = true and true;"),
        7 => writeln!(w, "    print({i});"),
        8 => writeln!(w, "    print(format(\"{{}}\", {i}));"),
        9 => writeln!(
            w,
            "    let g{i}: GenKind = GenKind::{};",
            gen_kind_variant(i)
        ),
        10 => writeln!(w, "    let fl{i}: GenFlag = GenFlag::Off;"),
        11 => writeln!(
            w,
            "    let p{i}: GenPoint = GenPoint {{ x: {i}, y: {i} + 1 }};"
        ),
        12 => writeln!(
            w,
            "    let item{i}: GenItem = GenItem {{ id: {i}, tag: \"t\" }};"
        ),
        13 => writeln!(w, "    let u{i}: int = add_pair({i}, 1);"),
        14 => writeln!(w, "    let t{i}: int = triple({i});"),
        15 => writeln!(w, "    let c{i}: int = gen_if_chain({i} - {i});"),
        16 => writeln!(w, "    let w{i}: int = gen_while_count({i} % 3);"),
        17 => writeln!(w, "    let r{i}: int = gen_for_sum({i} % 4);"),
        18 => writeln!(w, "    let len{i}: int = len(\"ab\");"),
        19 => writeln!(
            w,
            "    if ({i} % 2 == 0) {{ print(1); }} else {{ print(0); }}"
        ),
        20 => writeln!(w, "    while (false) {{ print(1); }}"),
        21 => writeln!(
            w,
            "    for (let j{i}: int = 0; j{i} < 1; j{i} = j{i} + 1) {{ print(j{i}); }}"
        ),
        22 => writeln!(w, "    print(global_seed);"),
        23 => writeln!(w, "    print(global_label);"),
        24 => writeln!(
            w,
            "    let off{i}: GenPoint = GenPoint {{ x: {i} + 1, y: {i} + 1 }};"
        ),
        _ => writeln!(w, "    let sq{i}: int = {i} * {i};"),
    }
}

/// Two-line pattern: struct then `.sum()` on same `GenPoint` name.
fn write_point_sum_pair(w: &mut impl Write, i: usize) -> std::io::Result<()> {
    writeln!(
        w,
        "    let pt{i}: GenPoint = GenPoint {{ x: {i}, y: {i} }};"
    )?;
    writeln!(w, "    let ptsum{i}: int = pt{i}.sum();")?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = Command::new("gen-long-rv")
        .about("Generate a large Raven source file (stress / long-parse testing)")
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .default_value("testing/long-code/src/main.rv")
                .help("Output .rv path (parent directories are created)"),
        )
        .arg(
            Arg::new("lines")
                .short('n')
                .long("lines")
                .default_value("200000")
                .value_parser(clap::value_parser!(usize))
                .help("Total lines in the file (preamble + main + closing `main();`)"),
        )
        .get_matches();

    let path = matches.get_one::<String>("output").unwrap();
    let total_lines: usize = *matches.get_one("lines").unwrap();

    let out_path = Path::new(path);
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let preamble_len = PREAMBLE.len();
    let min_lines = preamble_len + 3;
    if total_lines < min_lines {
        return Err(format!(
            "need at least {min_lines} lines to fit preamble, `fun main()`, `}}`, and `main();` (got {total_lines})"
        )
        .into());
    }

    let body_lines = total_lines - preamble_len - 3;

    let f = fs::File::create(out_path)?;
    let mut w = BufWriter::with_capacity(1 << 20, f);

    write_preamble(&mut w)?;

    writeln!(w, "fun main() -> void {{")?;

    let mut i: usize = 0;
    while i < body_lines {
        if i % PATTERN == 11 && i + 1 < body_lines {
            write_point_sum_pair(&mut w, i)?;
            i += 2;
        } else {
            write_body_line(&mut w, i)?;
            i += 1;
        }
    }

    writeln!(w, "}}")?;
    writeln!(w, "main();")?;

    w.flush()?;
    eprintln!(
        "Wrote {total_lines} lines to {path} (preamble {preamble_len}, main body {body_lines})"
    );
    Ok(())
}
