fn a(i: usize, j: usize) -> f64 {
    1.0 / ((i + j) * (i + j + 1) / 2 + i + 1) as f64
}

fn mul_av(v: &[f64], out: &mut [f64]) {
    for i in 0..v.len() {
        let mut s = 0.0;
        for j in 0..v.len() {
            s += a(i, j) * v[j];
        }
        out[i] = s;
    }
}

fn mul_atv(v: &[f64], out: &mut [f64]) {
    for i in 0..v.len() {
        let mut s = 0.0;
        for j in 0..v.len() {
            s += a(j, i) * v[j];
        }
        out[i] = s;
    }
}

fn main() {
    let n = 2000;
    let mut u = vec![1.0; n];
    let mut v = vec![0.0; n];
    let mut tmp = vec![0.0; n];
    for _ in 0..10 {
        mul_av(&u, &mut tmp);
        mul_atv(&tmp, &mut v);
        mul_av(&v, &mut tmp);
        mul_atv(&tmp, &mut u);
    }
    let (mut vbv, mut vv) = (0.0, 0.0);
    for i in 0..n {
        vbv += u[i] * v[i];
        vv += v[i] * v[i];
    }
    println!("{:.9}", (vbv / vv).sqrt());
}
