struct Tree {
    l: Option<Box<Tree>>,
    r: Option<Box<Tree>>,
}

fn make(d: i32) -> Box<Tree> {
    if d == 0 {
        return Box::new(Tree { l: None, r: None });
    }
    Box::new(Tree { l: Some(make(d - 1)), r: Some(make(d - 1)) })
}

fn check(t: &Tree) -> i64 {
    match (&t.l, &t.r) {
        (Some(l), Some(r)) => 1 + check(l) + check(r),
        _ => 1,
    }
}

fn main() {
    let mut total = 0;
    for _ in 0..40 {
        let t = make(16);
        total += check(&t);
    }
    println!("{}", total);
}
