# Standard Library Overview

Raven comes with a comprehensive standard library that provides essential functionality for common programming tasks.

## Available Modules

### Core Modules
- **[Math](https://github.com/martian56/raven/blob/main/lib/math.rv)** - Mathematical operations and functions
- **[Collections](https://github.com/martian56/raven/blob/main/lib/collections.rv)** - Data structures (maps, sets, lists)
- **[String](https://github.com/martian56/raven/blob/main/lib/string.rv)** - String manipulation and utilities
- **[Filesystem](https://github.com/martian56/raven/blob/main/lib/filesystem.rv)** - File and directory operations
- **[Time](https://github.com/martian56/raven/blob/main/lib/time.rv)** - Date and time handling
- **[Network](https://github.com/martian56/raven/blob/main/lib/network.rv)** - HTTP and networking utilities
- **[Testing](https://github.com/martian56/raven/blob/main/lib/testing.rv)** - Unit testing framework

## Built-in Functions

Raven provides several built-in functions that are always available:

### Output Functions
```raven
// Print to console
print("Hello, World!");

// Formatted printing
print(format("User: {}, Age: {}", name, age));
```

### Input Functions
```raven
// Get user input
let name: String = input("Enter your name: ");
```

### Type Functions
```raven
// Get type information
let value: int = 42;
print(type(value));  // "int"

// Convert string to enum
let status: HttpStatus = enum_from_string("HttpStatus", "OK");
```

### File Functions
```raven
// File operations
write_file("data.txt", "Hello, World!");
let content: String = read_file("data.txt");
let exists: bool = file_exists("data.txt");
```

### Array Functions
```raven
// Array utilities
let numbers: int[] = [1, 2, 3, 4, 5];
print(len(numbers));  // 5

// Array methods
numbers.push(6);
let last: int = numbers.pop();
let slice: int[] = numbers.slice(1, 3);
```

## Using Modules

Import modules using the `import` statement:

```raven
import "math";
import "collections";
import "string";

// Use functions from modules
let result: float = math_pow(2.0, 3.0);
let map: Map = new_map();
let formatted: String = string_format("Value: {}", 42);
```

## Module Structure

Each module follows a consistent structure:

```raven
// Module: math.rv
export fun math_add(a: float, b: float) -> float {
    return a + b;
}

export fun math_subtract(a: float, b: float) -> float {
    return a - b;
}

// Structs can also be exported
export struct Vector {
    x: float,
    y: float,
    z: float
}
```

## Best Practices

1. **Import only what you need** - Don't import unused modules
2. **Use descriptive names** - Module functions are prefixed with module name
3. **Check documentation** - Each module has detailed documentation
4. **Handle errors** - Some functions may fail, check return values
5. **Use appropriate types** - Match function parameter types exactly

## Error Handling

Standard library functions use consistent error handling:

```raven
// File operations may fail
if (file_exists("config.txt")) {
    let config: String = read_file("config.txt");
    print(config);
} else {
    print("Config file not found");
}

// Array operations check bounds
let numbers: int[] = [1, 2, 3];
if (len(numbers) > 0) {
    let first: int = numbers[0];
    print(first);
}
```

## Performance Notes

- **Built-in functions** are optimized and fast
- **Module functions** may have slight overhead
- **Array operations** are efficient for most use cases
- **String operations** create new strings (immutable)

## Future Modules

Planned standard library modules:
- **JSON** - JSON parsing and generation
- **Regex** - Regular expressions
- **Crypto** - Cryptographic functions
- **Database** - Database connectivity
- **Web** - Web framework utilities

---

**Next**: Explore the [GitHub repository](https://github.com/martian56/raven/tree/main/lib) for standard library source code

