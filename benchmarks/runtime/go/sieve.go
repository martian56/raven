package main

import "fmt"

func main() {
	n := 20000000
	flags := make([]bool, 0)
	for i := 0; i <= n; i++ {
		flags = append(flags, true)
	}
	count := 0
	for p := 2; p <= n; p++ {
		if flags[p] {
			count++
			for m := p * p; m <= n; m += p {
				flags[m] = false
			}
		}
	}
	fmt.Println(count)
}
