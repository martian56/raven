# 📚 Raven Standard Library Specification

## Architecture Overview

```
raven/
├── stdlib/
│   ├── core/           # Core built-ins (always available)
│   │   ├── print.rv
│   │   ├── len.rv
│   │   └── type_conversion.rv
│   ├── io/             # File and console I/O
│   │   ├── file.rv
│   │   ├── console.rv
│   │   └── mod.rv
│   ├── math/           # Mathematics
│   │   ├── basic.rv
│   │   ├── trig.rv
│   │   └── mod.rv
│   ├── string/         # string manipulation
│   │   ├── transform.rv
│   │   ├── search.rv
│   │   └── mod.rv
│   ├── collections/    # Data structures
│   │   ├── array.rv
│   │   ├── map.rv
│   │   └── mod.rv
│   ├── time/           # Date and time
│   │   └── mod.rv
│   └── json/           # JSON parsing and formatting
│       └── mod.rv
```

---

## Core Built-ins (No Import Required)

### Type Conversion
```raven
// Integer conversions
fun int(value: any) -> int;
fun float(value: any) -> float;
fun bool(value: any) -> bool;
fun string(value: any) -> string;

// Examples:
let x: int = int("42");        // string to int
let y: float = float(42);       // Int to float
let s: string = string(3.14);   // Float to string
let b: bool = bool(1);          // Int to bool (0=false, else=true)
```

### I/O Functions
```raven
// Print without newline
fun print(value: any) -> void;

// Print with newline
fun println(value: any) -> void;

// Read input from user
fun input(prompt: string) -> string;

// Examples:
print("Hello");      // Hello (no newline)
println("World");    // World\n
let name: string = input("Enter name: ");
```

### Collection Functions
```raven
// Get length of string or array
fun len(collection: any) -> int;

// Examples:
let size: int = len("hello");     // 5
let count: int = len([1, 2, 3]);  // 3
```

### Utility Functions
```raven
// Type checking
fun type_of(value: any) -> string;

// Examples:
let t: string = type_of(42);  // "int"
```

---

## `io` Module

### File Operations

```raven
import io;

// Read entire file to string
fun io.read_file(path: string) -> string;

// Write string to file
fun io.write_file(path: string, content: string) -> void;

// Append to file
fun io.append_file(path: string, content: string) -> void;

// Check if file exists
fun io.file_exists(path: string) -> bool;

// Delete file
fun io.delete_file(path: string) -> void;

// Read file line by line
fun io.read_lines(path: string) -> [string];

// Examples:
let content: string = io.read_file("data.txt");
io.write_file("output.txt", "Hello, World!");

if (io.file_exists("config.txt")) {
    let lines: [string] = io.read_lines("config.txt");
}
```

### Console Operations

```raven
import io;

// Read single line from console
fun io.read_line() -> string;

// Read single character
fun io.read_char() -> string;

// Clear console
fun io.clear() -> void;

// Examples:
println("Enter your name:");
let name: string = io.read_line();
```

---

## `math` Module

### Constants

```raven
import math;

let pi: float = math.PI;       // 3.14159265359
let e: float = math.E;          // 2.71828182846
let tau: float = math.TAU;      // 6.28318530718
```

### Basic Math

```raven
import math;

// Absolute value
fun math.abs(n: int) -> int;
fun math.abs_f(n: float) -> float;

// Power
fun math.pow(base: int, exp: int) -> int;
fun math.pow_f(base: float, exp: float) -> float;

// Square root
fun math.sqrt(n: float) -> float;

// Min and max
fun math.min(a: int, b: int) -> int;
fun math.max(a: int, b: int) -> int;

// Rounding
fun math.floor(n: float) -> int;
fun math.ceil(n: float) -> int;
fun math.round(n: float) -> int;

// Examples:
let abs_val: int = math.abs(-5);           // 5
let power: int = math.pow(2, 10);          // 1024
let root: float = math.sqrt(16.0);         // 4.0
let minimum: int = math.min(10, 20);       // 10
let rounded: int = math.round(3.7);        // 4
```

### Trigonometry

```raven
import math;

fun math.sin(radians: float) -> float;
fun math.cos(radians: float) -> float;
fun math.tan(radians: float) -> float;

fun math.asin(value: float) -> float;
fun math.acos(value: float) -> float;
fun math.atan(value: float) -> float;

// Degrees/Radians conversion
fun math.to_radians(degrees: float) -> float;
fun math.to_degrees(radians: float) -> float;

// Examples:
let sine: float = math.sin(math.PI / 2.0);  // 1.0
let angle: float = math.to_radians(90.0);   // 1.57...
```

### Random Numbers (Future)

```raven
import math;

fun math.random() -> float;  // Random float [0.0, 1.0)
fun math.random_int(min: int, max: int) -> int;
```

---

## `string` Module

### Transformation

```raven
import string;

// Case conversion
fun string.uppercase(s: string) -> string;
fun string.lowercase(s: string) -> string;

// Trim whitespace
fun string.trim(s: string) -> string;
fun string.trim_left(s: string) -> string;
fun string.trim_right(s: string) -> string;

// Reverse
fun string.reverse(s: string) -> string;

// Examples:
let upper: string = string.uppercase("hello");  // "HELLO"
let lower: string = string.lowercase("WORLD");  // "world"
let clean: string = string.trim("  hello  ");  // "hello"
let rev: string = string.reverse("abc");        // "cba"
```

### Searching & Testing

```raven
import string;

// Check if string contains substring
fun string.contains(s: string, substr: string) -> bool;

// Check if starts/ends with
fun string.starts_with(s: string, prefix: string) -> bool;
fun string.ends_with(s: string, suffix: string) -> bool;

// Find index of substring
fun string.index_of(s: string, substr: string) -> int;  // -1 if not found

// Examples:
let has_hello: bool = string.contains("hello world", "hello");  // true
let starts: bool = string.starts_with("raven", "rav");          // true
let idx: int = string.index_of("hello", "ll");                  // 2
```

### Splitting & Joining

```raven
import string;

// Split string by delimiter
fun string.split(s: string, delim: string) -> [string];

// Join array of strings
fun string.join(parts: [string], delim: string) -> string;

// Replace occurrences
fun string.replace(s: string, old: string, new: string) -> string;

// Examples:
let parts: [string] = string.split("a,b,c", ",");  // ["a", "b", "c"]
let joined: string = string.join(parts, "-");      // "a-b-c"
let replaced: string = string.replace("hello", "l", "r");  // "herro"
```

### Character Operations

```raven
import string;

// Get character at index
fun string.char_at(s: string, index: int) -> string;

// Get ASCII code
fun string.char_code(c: string) -> int;

// From ASCII code
fun string.from_char_code(code: int) -> string;

// Examples:
let ch: string = string.char_at("hello", 0);  // "h"
let code: int = string.char_code("A");        // 65
let letter: string = string.from_char_code(65);  // "A"
```

---

## `array` Module (Extended Array Functions)

```raven
import array;

// Add element to end
fun array.push(arr: [T], item: T) -> void;

// Remove and return last element
fun array.pop(arr: [T]) -> T;

// Insert at index
fun array.insert(arr: [T], index: int, item: T) -> void;

// Remove at index
fun array.remove(arr: [T], index: int) -> T;

// Check if contains
fun array.contains(arr: [T], item: T) -> bool;

// Find index of item
fun array.index_of(arr: [T], item: T) -> int;

// Reverse array
fun array.reverse(arr: [T]) -> void;

// Sort array
fun array.sort(arr: [T]) -> void;

// Examples:
let nums: [int] = [1, 2, 3];
array.push(nums, 4);           // [1, 2, 3, 4]
let last: int = array.pop(nums);  // 4, nums = [1, 2, 3]
array.reverse(nums);           // [3, 2, 1]
```

---

## `time` Module (Future)

```raven
import time;

// Get current timestamp
fun time.now() -> int;

// Sleep for milliseconds
fun time.sleep(ms: int) -> void;

// Format time
fun time.format(timestamp: int, format: string) -> string;

// Examples:
let now: int = time.now();
time.sleep(1000);  // Sleep 1 second
let formatted: string = time.format(now, "YYYY-MM-DD");
```

---

## `json` Module (Future)

```raven
import json;

// Parse JSON string
fun json.parse(s: string) -> Map;

// Convert to JSON string
fun json.stringify(obj: Map) -> string;

// Examples:
let data: Map = json.parse('{"name": "Alice", "age": 25}');
let json_str: string = json.stringify(data);
```

---

## `http` Module (Future)

```raven
import http;

// Make HTTP GET request
fun http.get(url: string) -> string;

// Make HTTP POST request
fun http.post(url: string, body: string) -> string;

// Examples:
let response: string = http.get("https://api.example.com/data");
let result: string = http.post("https://api.example.com/submit", '{"data": "value"}');
```

---

## Implementation Strategy

### Phase 1: Core Built-ins (Week 1-2)
1. Implement `print`, `println`, `input`
2. Implement `len`, `type_of`
3. Implement type conversion functions
4. Add to `src/builtins.rs`

### Phase 2: `math` Module (Week 3)
1. Create `stdlib/math/mod.rv`
2. Implement basic math functions
3. Add constants (PI, E)
4. Implement trigonometry functions

### Phase 3: `string` Module (Week 4)
1. Create `stdlib/string/mod.rv`
2. Implement transformation functions
3. Implement search/test functions
4. Implement split/join

### Phase 4: `io` Module (Week 5)
1. Create `stdlib/io/mod.rv`
2. Implement file operations using Rust std::fs
3. Implement console operations

### Phase 5: `array` Module (Week 6)
1. Create `stdlib/array/mod.rv`
2. Implement array manipulation functions

---

## Directory Structure

```
raven/
├── src/
│   ├── builtins.rs         # ← NEW: Built-in function registry
│   ├── stdlib_loader.rs    # ← NEW: Standard library loader
│   └── ...
├── stdlib/
│   ├── prelude.rv          # ← Auto-imported in every file
│   ├── math/
│   │   └── mod.rv
│   ├── string/
│   │   └── mod.rv
│   ├── io/
│   │   └── mod.rv
│   └── array/
│       └── mod.rv
```

---

## Example Usage

```raven
// Math operations
import math;

let radius: float = 5.0;
let area: float = math.PI * math.pow_f(radius, 2.0);
println(area);

// string manipulation
import string;

let text: string = "Hello, Raven!";
let upper: string = string.uppercase(text);
let parts: [string] = string.split(upper, ",");

for (let i: int = 0; i < len(parts); i = i + 1) {
    let trimmed: string = string.trim(parts[i]);
    println(trimmed);
}

// File I/O
import io;

let data: string = io.read_file("input.txt");
let lines: [string] = string.split(data, "\n");

let processed: string = "";
for (let i: int = 0; i < len(lines); i = i + 1) {
    processed = processed + string.uppercase(lines[i]) + "\n";
}

io.write_file("output.txt", processed);
println("Processing complete!");
```

This creates a solid foundation for a usable standard library! 🚀

