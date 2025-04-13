# Raven Language Specification (v0.1)

## 🔡 Variable Declarations

```raven
let name: String = "Fuad";
let age: int = 22;
let pi: float = 3.14;
let isReady: bool = true;
```

- Variables declared with `let` are **mutable by default**.
- Use `let const` to make them **immutable**:

```raven
let const maxUsers: int = 100;
```

---

## 🧮 Expressions

```raven
let sum = 5 + 10;
let greeting = "Hello, " + name;
```

- Arithmetic: `+`, `-`, `*`, `/`, `%`
- Logical: `&&`, `||`, `!`
- Comparison: `==`, `!=`, `<`, `>`, `<=`, `>=`

---

## 🧠 Functions

```raven
fun add(a: int, b: int) -> int {
    return a + b;
}

fun greet(name: String) -> void {
    print("Hello, " + name);
}
```

- `fun` declares a function
- Supports multiple parameters
- `void` return type means no return value

---

## 🔁 Control Flow

### `if`, `elseif`, `else`

```raven
if (age < 18) {
    print("Too young");
} elseif (age < 30) {
    print("Young adult");
} else {
    print("Mature");
}
```

### `while` loop

```raven
let i: int = 0;

while (i < 10) {
    print(i);
    i = i + 1;
}
```

### `for` loop (C-style)

```raven
for (let i = 0; i < 10; i = i + 1) {
    print(i);
}
```

> Optionally Python-style in the future:

```raven
for i in 0..10 {
    print(i);
}
```

---

## 📦 Data Structures (Preview)

```raven
struct Person {
    name: String;
    age: int;
}

let fuad = Person("Fuad", 22);
print(fuad.name);
```

---

## 🧰 Built-in Functions

```raven
print(value);       // Print to console
input(prompt);      // Read input
len(array);         // Length of array
```

---

## 📁 Modules (Future Plan)

```raven
import math;
let r = 3;
let area = math.pi * r * r;
```

---

## 🧪 Example Program

```raven
fun factorial(n: int) -> int {
    if (n <= 1) {
        return 1;
    }
    return n * factorial(n - 1);
}

let result = factorial(5);
print("5! = " + result);
```
