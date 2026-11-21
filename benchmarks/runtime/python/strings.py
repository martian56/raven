parts = [f"item {i}" for i in range(200000)]
print(sum(len(p) for p in parts))
