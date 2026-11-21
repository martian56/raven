size, max_iter, inside = 1600, 50, 0
for y in range(size):
    ci = 2.0 * y / size - 1.0
    for x in range(size):
        cr = 2.0 * x / size - 1.5
        zr = zi = 0.0
        i = 0
        while i < max_iter and zr * zr + zi * zi <= 4.0:
            zr, zi = zr * zr - zi * zi + cr, 2.0 * zr * zi + ci
            i += 1
        if i == max_iter:
            inside += 1
print(inside)
