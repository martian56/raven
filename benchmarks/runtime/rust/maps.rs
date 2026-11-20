use std::collections::HashMap;

fn main() {
    let mut m: HashMap<i64, i64> = HashMap::new();
    for i in 0..1_000_000 {
        m.insert(i, i * 2);
    }
    let mut sum = 0;
    for j in 0..1_000_000 {
        sum += m.get(&j).copied().unwrap_or(0);
    }
    println!("{}", sum);
}
