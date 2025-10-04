# Basic Examples

These examples demonstrate the fundamental features of Raven programming language.

## Hello World

The classic first program:

```raven
fun main() -> void {
    print("Hello, World!");
}

main();
```

## Variables and Types

```raven
fun main() -> void {
    let name: String = "Alice";
    let age: int = 25;
    let height: float = 5.9;
    let isActive: bool = true;
    
    print(format("Name: {}, Age: {}, Height: {}, Active: {}", 
                 name, age, height, isActive));
}

main();
```

## Arrays

```raven
fun main() -> void {
    let numbers: int[] = [1, 2, 3, 4, 5];
    
    // Print array
    print(numbers);
    
    // Add element
    numbers.push(6);
    print(numbers);
    
    // Remove last element
    let last: int = numbers.pop();
    print(format("Removed: {}", last));
    print(numbers);
    
    // Get slice
    let slice: int[] = numbers.slice(1, 3);
    print(slice);
}

main();
```

## Control Flow

### If Statements

```raven
fun main() -> void {
    let age: int = 18;
    
    if (age < 18) {
        print("Too young");
    } else if (age < 65) {
        print("Working age");
    } else {
        print("Retirement age");
    }
}

main();
```

### While Loops

```raven
fun main() -> void {
    let i: int = 0;
    
    while (i < 5) {
        print(format("Count: {}", i));
        i = i + 1;
    }
}

main();
```

### For Loops

```raven
fun main() -> void {
    let numbers: int[] = [10, 20, 30, 40, 50];
    
    for (let i: int = 0; i < len(numbers); i = i + 1) {
        print(format("Index {}: {}", i, numbers[i]));
    }
}

main();
```

## Functions

### Simple Functions

```raven
fun greet(name: String) -> void {
    print(format("Hello, {}!", name));
}

fun add(a: int, b: int) -> int {
    return a + b;
}

fun main() -> void {
    greet("Raven");
    let result: int = add(5, 3);
    print(format("5 + 3 = {}", result));
}

main();
```

### Recursive Functions

```raven
fun factorial(n: int) -> int {
    if (n <= 1) {
        return 1;
    }
    return n * factorial(n - 1);
}

fun main() -> void {
    let result: int = factorial(5);
    print(format("5! = {}", result));
}

main();
```

## Structs

```raven
struct Person {
    name: String,
    age: int,
    isActive: bool
}

fun main() -> void {
    let person: Person = Person { 
        name: "Alice", 
        age: 25, 
        isActive: true 
    };
    
    print(format("Name: {}", person.name));
    print(format("Age: {}", person.age));
    print(format("Active: {}", person.isActive));
    
    // Modify fields
    person.age = 26;
    print(format("New age: {}", person.age));
}

main();
```

## Enums

```raven
enum HttpStatus {
    OK,
    NotFound,
    InternalError,
    BadRequest
}

fun main() -> void {
    let status: HttpStatus = HttpStatus::OK;
    
    print(format("Status: {}", status));
    
    // String to enum conversion
    let jsonStatus: String = "NotFound";
    let parsedStatus: HttpStatus = enum_from_string("HttpStatus", jsonStatus);
    print(format("Parsed: {}", parsedStatus));
}

main();
```

## File Operations

```raven
fun main() -> void {
    let filename: String = "test.txt";
    let content: String = "Hello from Raven!";
    
    // Write file
    write_file(filename, content);
    print("File written");
    
    // Check if file exists
    if (file_exists(filename)) {
        print("File exists");
        
        // Read file
        let data: String = read_file(filename);
        print(format("Content: {}", data));
    } else {
        print("File not found");
    }
}

main();
```

## User Input

```raven
fun main() -> void {
    let name: String = input("Enter your name: ");
    let ageStr: String = input("Enter your age: ");
    
    print(format("Hello, {}!", name));
    print(format("You are {} years old", ageStr));
}

main();
```

## Error Handling

```raven
fun divide(a: float, b: float) -> float {
    if (b == 0.0) {
        print("Error: Division by zero");
        return 0.0;
    }
    return a / b;
}

fun main() -> void {
    let result1: float = divide(10.0, 2.0);
    print(format("10 / 2 = {}", result1));
    
    let result2: float = divide(10.0, 0.0);
    print(format("10 / 0 = {}", result2));
}

main();
```

## Complete Example: Calculator

```raven
fun add(a: float, b: float) -> float {
    return a + b;
}

fun subtract(a: float, b: float) -> float {
    return a - b;
}

fun multiply(a: float, b: float) -> float {
    return a * b;
}

fun divide(a: float, b: float) -> float {
    if (b == 0.0) {
        print("Error: Division by zero");
        return 0.0;
    }
    return a / b;
}

fun main() -> void {
    print("Simple Calculator");
    print("1. Add");
    print("2. Subtract");
    print("3. Multiply");
    print("4. Divide");
    
    let choice: String = input("Enter choice (1-4): ");
    let aStr: String = input("Enter first number: ");
    let bStr: String = input("Enter second number: ");
    
    // Note: In a real implementation, you'd convert strings to numbers
    print(format("Choice: {}, A: {}, B: {}", choice, aStr, bStr));
}

main();
```

---

**Next**: Check out more examples in our [GitHub repository](https://github.com/martian56/raven/tree/main/examples)

