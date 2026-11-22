package main

import "fmt"

func main() {
	n := 10
	perm, perm1, count := make([]int, n), make([]int, n), make([]int, n)
	for i := range perm1 {
		perm1[i] = i
	}
	maxFlips, checksum, permCount, r := 0, 0, 0, n
	for {
		for r != 1 {
			count[r-1] = r
			r--
		}
		copy(perm, perm1)
		flips, first := 0, perm[0]
		for first != 0 {
			for lo, hi := 0, first; lo < hi; lo, hi = lo+1, hi-1 {
				perm[lo], perm[hi] = perm[hi], perm[lo]
			}
			flips++
			first = perm[0]
		}
		if flips > maxFlips {
			maxFlips = flips
		}
		if permCount%2 == 0 {
			checksum += flips
		} else {
			checksum -= flips
		}
		for {
			if r == n {
				fmt.Println(checksum)
				fmt.Println(maxFlips)
				return
			}
			p0 := perm1[0]
			for j := 0; j < r; j++ {
				perm1[j] = perm1[j+1]
			}
			perm1[r] = p0
			count[r]--
			if count[r] > 0 {
				break
			}
			r++
		}
		permCount++
	}
}
