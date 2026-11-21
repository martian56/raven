import math


class Body:
    def __init__(self, x, y, z, vx, vy, vz, m):
        self.x, self.y, self.z = x, y, z
        self.vx, self.vy, self.vz = vx, vy, vz
        self.m = m


def advance(bodies, dt):
    n = len(bodies)
    for i in range(n):
        a = bodies[i]
        for j in range(i + 1, n):
            b = bodies[j]
            dx, dy, dz = a.x - b.x, a.y - b.y, a.z - b.z
            d2 = dx * dx + dy * dy + dz * dz
            mag = dt / (d2 * math.sqrt(d2))
            a.vx -= dx * b.m * mag
            a.vy -= dy * b.m * mag
            a.vz -= dz * b.m * mag
            b.vx += dx * a.m * mag
            b.vy += dy * a.m * mag
            b.vz += dz * a.m * mag
    for b in bodies:
        b.x += dt * b.vx
        b.y += dt * b.vy
        b.z += dt * b.vz


def energy(bodies):
    e = 0.0
    for i, a in enumerate(bodies):
        e += 0.5 * a.m * (a.vx * a.vx + a.vy * a.vy + a.vz * a.vz)
        for b in bodies[i + 1:]:
            dx, dy, dz = a.x - b.x, a.y - b.y, a.z - b.z
            e -= a.m * b.m / math.sqrt(dx * dx + dy * dy + dz * dz)
    return e


bodies = [
    Body(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 39.47),
    Body(4.84, -1.16, -0.10, 0.60, 2.81, -0.02, 0.037),
    Body(8.34, 4.12, -0.40, -1.01, 1.82, 0.008, 0.011),
    Body(12.89, -15.11, -0.22, 1.08, 0.86, -0.01, 0.0017),
    Body(15.37, -25.91, 0.17, 0.97, 0.59, -0.03, 0.002),
]
print(f"{energy(bodies):.9f}")
for _ in range(5_000_000):
    advance(bodies, 0.01)
print(f"{energy(bodies):.9f}")
