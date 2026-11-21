def make(d):
    return None if d == 0 else (make(d - 1), make(d - 1))

def check(t):
    return 1 if t is None else 1 + check(t[0]) + check(t[1])

total = 0
for i in range(40):
    total += check(make(16))
print(total)
