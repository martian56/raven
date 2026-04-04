# Quick Start

Let's get you up and running with Raven in just a few minutes!

## Your First Program

Create a file called `hello.rv`:

```raven
fun main() -> void {
    print("Hello, Raven!");
}

main();
```

Run it:

```bash
raven hello.rv
```

You should see: `Hello, Raven!`

## Interactive REPL

Try the interactive mode:

```bash
raven
```

You'll see the Raven prompt:

```
raven> let name: string = "World";
raven> print(format("Hello, {}!", name));
Hello, World!
raven> exit
```

## Basic Syntax

### Variables

```raven
let name: string = "Alice";
let age: int = 25;
let height: float = 5.9;
let isActive: bool = true;
```

### Arrays

```raven
let numbers: int[] = [1, 2, 3, 4, 5];
numbers.push(6);
print(numbers);  // [1, 2, 3, 4, 5, 6]
```

### Control Flow

```raven
let age: int = 18;

if (age < 18) {
    print("Too young");
} else {
    print("Welcome!");
}

// While loop
let i: int = 0;
while (i < 5) {
    print(i);
    i = i + 1;
}
```

### Functions

```raven
fun greet(name: string) -> void {
    print(format("Hello, {}!", name));
}

fun add(a: int, b: int) -> int {
    return a + b;
}

// Usage
greet("Raven");
let result: int = add(5, 3);
print(result);  // 8
```

## Data Structures

### Structs

```raven
struct Person {
    name: string,
    age: int,
    isActive: bool
}

let person: Person = Person { 
    name: "Alice", 
    age: 25, 
    isActive: true 
};

print(person.name);  // Alice
```

### Enums

```raven
enum HttpStatus {
    OK,
    NotFound,
    InternalError
}

let status: HttpStatus = HttpStatus::OK;
print(status);  // HttpStatus::OK
```

## File Operations

```raven
// Write to file
let content: string = "Hello from Raven!";
write_file("output.txt", content);

// Read from file
if (file_exists("output.txt")) {
    let data: string = read_file("output.txt");
    print(data);
}
```

## Next Steps

Now that you've got the basics:

1. **[Language Reference](../syntax.md)** - Complete syntax guide
2. **[Standard Library](../standard-library/overview.md)** - Built-in functions
3. **[rvpm and formatting](rvpm-and-format.md)** - `rv.toml`, formatter, `rvpm fmt`
4. **[Examples](../examples/basic.md)** - More sample programs
5. **[VS Code Extension](../resources/vscode-extension.md)** - Development setup

## Common Commands

```bash
# Run a program
raven program.rv

# Start REPL
raven

# Type-check only (parse + types; does not run)
raven program.rv -c

# Verbose output (tokens, AST)
raven program.rv -v

# Show help
raven --help
```

### Project commands (`rvpm`)

From a directory that contains (or is under) an `rv.toml` project:

```bash
rvpm init my_app    # create rv.toml, src/main.rv, rv_env/
cd my_app
rvpm run            # runs raven on src/main.rv
rvpm fmt            # format .rv files (optional [fmt] in rv.toml)
rvpm fmt --check    # fail if formatting would change files (CI)
```

---

**Ready for more?** Check out our [Language Reference](../syntax.md) for complete syntax details!
