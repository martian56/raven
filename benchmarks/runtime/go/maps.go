package main

import "fmt"

func main() {
	m := map[int]int{}
	for i := 0; i < 1000000; i++ {
		m[i] = i * 2
	}
	sum := 0
	for j := 0; j < 1000000; j++ {
		sum += m[j]
	}
	fmt.Println(sum)
}
