fn main() {
    let n = 10usize;
    let mut perm = vec![0usize; n];
    let mut perm1: Vec<usize> = (0..n).collect();
    let mut count = vec![0usize; n];
    let (mut max_flips, mut checksum, mut perm_count, mut r) = (0i64, 0i64, 0i64, n);
    loop {
        while r != 1 {
            count[r - 1] = r;
            r -= 1;
        }
        perm.copy_from_slice(&perm1);
        let mut flips = 0i64;
        let mut first = perm[0];
        while first != 0 {
            let (mut lo, mut hi) = (0usize, first);
            while lo < hi {
                perm.swap(lo, hi);
                lo += 1;
                hi -= 1;
            }
            flips += 1;
            first = perm[0];
        }
        if flips > max_flips {
            max_flips = flips;
        }
        checksum += if perm_count % 2 == 0 { flips } else { -flips };
        loop {
            if r == n {
                println!("{}\n{}", checksum, max_flips);
                return;
            }
            let p0 = perm1[0];
            for j in 0..r {
                perm1[j] = perm1[j + 1];
            }
            perm1[r] = p0;
            count[r] -= 1;
            if count[r] > 0 {
                break;
            }
            r += 1;
        }
        perm_count += 1;
    }
}
