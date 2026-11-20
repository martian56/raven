fn main() {
    let n = 20_000_000usize;
    let mut flags = vec![true; n + 1];
    let mut count = 0;
    for p in 2..=n {
        if flags[p] {
            count += 1;
            let mut m = p * p;
            while m <= n {
                flags[m] = false;
                m += p;
            }
        }
    }
    println!("{}", count);
}
