fn main() {
    let parts: Vec<String> = (0..200_000).map(|i| format!("item {}", i)).collect();
    let total: usize = parts.iter().map(|s| s.len()).sum();
    println!("{}", total);
}
