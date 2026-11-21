package main

import "fmt"

func main() {
	size, maxIter, inside := 1600, 50, 0
	for y := 0; y < size; y++ {
		ci := 2.0*float64(y)/float64(size) - 1.0
		for x := 0; x < size; x++ {
			cr := 2.0*float64(x)/float64(size) - 1.5
			zr, zi := 0.0, 0.0
			i := 0
			for i < maxIter && zr*zr+zi*zi <= 4.0 {
				t := zr*zr - zi*zi + cr
				zi = 2.0*zr*zi + ci
				zr = t
				i++
			}
			if i == maxIter {
				inside++
			}
		}
	}
	fmt.Println(inside)
}
