package main

import (
	"fmt"
	"math"
)

type Body struct{ x, y, z, vx, vy, vz, m float64 }

func advance(b []*Body, dt float64) {
	n := len(b)
	for i := 0; i < n; i++ {
		a := b[i]
		for j := i + 1; j < n; j++ {
			c := b[j]
			dx, dy, dz := a.x-c.x, a.y-c.y, a.z-c.z
			d2 := dx*dx + dy*dy + dz*dz
			mag := dt / (d2 * math.Sqrt(d2))
			a.vx -= dx * c.m * mag
			a.vy -= dy * c.m * mag
			a.vz -= dz * c.m * mag
			c.vx += dx * a.m * mag
			c.vy += dy * a.m * mag
			c.vz += dz * a.m * mag
		}
	}
	for _, c := range b {
		c.x += dt * c.vx
		c.y += dt * c.vy
		c.z += dt * c.vz
	}
}

func energy(b []*Body) float64 {
	e := 0.0
	for i, a := range b {
		e += 0.5 * a.m * (a.vx*a.vx + a.vy*a.vy + a.vz*a.vz)
		for j := i + 1; j < len(b); j++ {
			c := b[j]
			dx, dy, dz := a.x-c.x, a.y-c.y, a.z-c.z
			e -= a.m * c.m / math.Sqrt(dx*dx+dy*dy+dz*dz)
		}
	}
	return e
}

func main() {
	b := []*Body{{0, 0, 0, 0, 0, 0, 39.47}, {4.84, -1.16, -0.10, 0.60, 2.81, -0.02, 0.037}, {8.34, 4.12, -0.40, -1.01, 1.82, 0.008, 0.011}, {12.89, -15.11, -0.22, 1.08, 0.86, -0.01, 0.0017}, {15.37, -25.91, 0.17, 0.97, 0.59, -0.03, 0.002}}
	fmt.Printf("%.9f\n", energy(b))
	for k := 0; k < 5000000; k++ {
		advance(b, 0.01)
	}
	fmt.Printf("%.9f\n", energy(b))
}
