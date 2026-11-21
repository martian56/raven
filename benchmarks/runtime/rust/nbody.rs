#[derive(Clone, Copy)]
struct Body {
    x: f64,
    y: f64,
    z: f64,
    vx: f64,
    vy: f64,
    vz: f64,
    m: f64,
}

fn advance(b: &mut [Body], dt: f64) {
    let n = b.len();
    for i in 0..n {
        for j in i + 1..n {
            let (l, r) = b.split_at_mut(j);
            let a = &mut l[i];
            let c = &mut r[0];
            let (dx, dy, dz) = (a.x - c.x, a.y - c.y, a.z - c.z);
            let d2 = dx * dx + dy * dy + dz * dz;
            let mag = dt / (d2 * d2.sqrt());
            a.vx -= dx * c.m * mag;
            a.vy -= dy * c.m * mag;
            a.vz -= dz * c.m * mag;
            c.vx += dx * a.m * mag;
            c.vy += dy * a.m * mag;
            c.vz += dz * a.m * mag;
        }
    }
    for c in b.iter_mut() {
        c.x += dt * c.vx;
        c.y += dt * c.vy;
        c.z += dt * c.vz;
    }
}

fn energy(b: &[Body]) -> f64 {
    let mut e = 0.0;
    for i in 0..b.len() {
        let a = &b[i];
        e += 0.5 * a.m * (a.vx * a.vx + a.vy * a.vy + a.vz * a.vz);
        for c in &b[i + 1..] {
            let (dx, dy, dz) = (a.x - c.x, a.y - c.y, a.z - c.z);
            e -= a.m * c.m / (dx * dx + dy * dy + dz * dz).sqrt();
        }
    }
    e
}

fn body(x: f64, y: f64, z: f64, vx: f64, vy: f64, vz: f64, m: f64) -> Body {
    Body { x, y, z, vx, vy, vz, m }
}

fn main() {
    let mut b = [
        body(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 39.47),
        body(4.84, -1.16, -0.10, 0.60, 2.81, -0.02, 0.037),
        body(8.34, 4.12, -0.40, -1.01, 1.82, 0.008, 0.011),
        body(12.89, -15.11, -0.22, 1.08, 0.86, -0.01, 0.0017),
        body(15.37, -25.91, 0.17, 0.97, 0.59, -0.03, 0.002),
    ];
    println!("{:.9}", energy(&b));
    for _ in 0..5_000_000 {
        advance(&mut b, 0.01);
    }
    println!("{:.9}", energy(&b));
}
