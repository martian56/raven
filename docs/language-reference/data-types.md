# Data Types

Raven is a statically-typed language with a rich type system that provides safety and expressiveness.

## Primitive Types

### Integer (`int`)
64-bit signed integers.

```raven
let age: int = 25;
let count: int = -10;
let maxValue: int = 9223372036854775807;
```

### Float (`float`)
64-bit floating-point numbers.

```raven
let pi: float = 3.14159;
let temperature: float = 98.6;
let precision: float = 0.001;
```

### Boolean (`bool`)
True or false values.

```raven
let isActive: bool = true;
let isComplete: bool = false;
let isValid: bool = (age > 18);
```

### String (`String`)
UTF-8 encoded strings.

```raven
let name: String = "Alice";
let message: String = "Hello, World!";
let empty: String = "";
```

## Array Types

Arrays are collections of elements of the same type.

### Integer Arrays
```raven
let numbers: int[] = [1, 2, 3, 4, 5];
let emptyNumbers: int[] = [];
let singleNumber: int[] = [42];
```

### String Arrays
```raven
let names: String[] = ["Alice", "Bob", "Charlie"];
let words: String[] = ["Hello", "World"];
```

### Boolean Arrays
```raven
let flags: bool[] = [true, false, true];
let results: bool[] = [];
```

## User-Defined Types

### Structs
Custom data structures with named fields.

```raven
struct Person {
    name: String,
    age: int,
    isActive: bool
}

struct Point {
    x: float,
    y: float
}

// Usage
let person: Person = Person { 
    name: "Alice", 
    age: 25, 
    isActive: true 
};

let point: Point = Point { x: 10.5, y: 20.0 };
```

### Enums
Custom types with a set of named variants.

```raven
enum HttpStatus {
    OK,
    NotFound,
    InternalError,
    BadRequest
}

enum Color {
    Red,
    Green,
    Blue,
    Yellow
}

// Usage
let status: HttpStatus = HttpStatus::OK;
let color: Color = Color::Red;
```

## Type Inference

Raven can infer types in some cases, but explicit typing is recommended for clarity.

```raven
// Explicit typing (recommended)
let name: String = "Alice";
let age: int = 25;

// Type inference (works but less clear)
let name = "Alice";  // Inferred as String
let age = 25;        // Inferred as int
```

## Type Checking

Raven performs static type checking at compile time.

```raven
let name: String = "Alice";
let age: int = 25;

// This would cause a type error:
// let result: int = name + age;  // Error: can't add String and int

// Correct way:
let result: String = format("{} is {} years old", name, age);
```

## Type Conversion

Raven provides built-in functions for type conversion.

```raven
let number: int = 42;
let text: String = format("{}", number);  // Convert int to String

let input: String = "123";
// Note: Raven doesn't have built-in string-to-number conversion yet
// This would be added in future versions
```

## Void Type

The `void` type represents the absence of a value, used for functions that don't return anything.

```raven
fun greet(name: String) -> void {
    print(format("Hello, {}!", name));
    // No return statement needed
}
```

## Type Safety

Raven's type system prevents many common errors:

- **No null pointers** - All variables must be initialized
- **No type confusion** - Can't accidentally mix types
- **Array bounds checking** - Prevents buffer overflows
- **Immutable by default** - Variables can be reassigned but not mutated in place

## Best Practices

1. **Always use explicit types** for function parameters and return values
2. **Use descriptive names** for struct fields and enum variants
3. **Initialize variables** when declaring them
4. **Use appropriate types** for your data (int for counts, float for measurements)
5. **Leverage type checking** to catch errors early

---

**Next**: [Control Flow](../syntax.md#control-flow) - Conditional statements and loops
