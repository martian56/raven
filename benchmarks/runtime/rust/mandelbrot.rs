fn main() {
    let (size, max_iter) = (1600i64, 50i64);
    let mut inside = 0i64;
    for y in 0..size {
        let ci = 2.0 * y as f64 / size as f64 - 1.0;
        for x in 0..size {
            let cr = 2.0 * x as f64 / size as f64 - 1.5;
            let (mut zr, mut zi) = (0.0f64, 0.0f64);
            let mut i = 0;
            while i < max_iter && zr * zr + zi * zi <= 4.0 {
                let t = zr * zr - zi * zi + cr;
                zi = 2.0 * zr * zi + ci;
                zr = t;
                i += 1;
            }
            if i == max_iter {
                inside += 1;
            }
        }
    }
    println!("{}", inside);
}
