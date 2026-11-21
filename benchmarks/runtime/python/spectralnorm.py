import math


def a(i, j):
    return 1.0 / ((i + j) * (i + j + 1) // 2 + i + 1)


def mul_av(v, out):
    n = len(v)
    for i in range(n):
        out[i] = sum(a(i, j) * v[j] for j in range(n))


def mul_atv(v, out):
    n = len(v)
    for i in range(n):
        out[i] = sum(a(j, i) * v[j] for j in range(n))


n = 2000
u, v, tmp = [1.0] * n, [0.0] * n, [0.0] * n
for _ in range(10):
    mul_av(u, tmp)
    mul_atv(tmp, v)
    mul_av(v, tmp)
    mul_atv(tmp, u)
vbv = sum(u[i] * v[i] for i in range(n))
vv = sum(x * x for x in v)
print("%.9f" % math.sqrt(vbv / vv))
