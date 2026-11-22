n = 10
perm1 = list(range(n))
count = [0] * n
max_flips = checksum = perm_count = 0
r = n
while True:
    while r != 1:
        count[r - 1] = r
        r -= 1
    perm = perm1[:]
    flips = 0
    first = perm[0]
    while first != 0:
        perm[: first + 1] = perm[first::-1]
        flips += 1
        first = perm[0]
    max_flips = max(max_flips, flips)
    checksum += flips if perm_count % 2 == 0 else -flips
    done = False
    while True:
        if r == n:
            print(checksum)
            print(max_flips)
            done = True
            break
        p0 = perm1[0]
        for j in range(r):
            perm1[j] = perm1[j + 1]
        perm1[r] = p0
        count[r] -= 1
        if count[r] > 0:
            break
        r += 1
    if done:
        break
    perm_count += 1
