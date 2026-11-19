m = {}
for i in range(1000000):
    m[i] = i * 2
s = 0
for j in range(1000000):
    s += m.get(j, 0)
print(s)
