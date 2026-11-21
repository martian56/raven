package main

import "fmt"

type T struct{ l, r *T }

func mk(d int) *T {
	if d == 0 {
		return &T{}
	}
	return &T{mk(d - 1), mk(d - 1)}
}

func check(t *T) int {
	if t.l == nil {
		return 1
	}
	return 1 + check(t.l) + check(t.r)
}

func main() {
	total := 0
	for i := 0; i < 40; i++ {
		total += check(mk(16))
	}
	fmt.Println(total)
}
