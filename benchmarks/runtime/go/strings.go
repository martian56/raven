package main

import "fmt"

func main() {
	parts := []string{}
	for i := 0; i < 200000; i++ {
		parts = append(parts, fmt.Sprintf("item %d", i))
	}
	total := 0
	for _, p := range parts {
		total += len(p)
	}
	fmt.Println(total)
}
