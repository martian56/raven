n = 20000000
flags = [True] * (n + 1)
count = 0
for p in range(2, n + 1):
    if flags[p]:
        count += 1
        for m in range(p * p, n + 1, p):
            flags[m] = False
print(count)
