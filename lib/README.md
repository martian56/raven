# Raven Standard Library

This directory contains the standard library modules for the Raven programming language.

## Available Modules

### 1. Math (`math.rv`)
Basic mathematical functions and operations.

**Functions:**
- `abs(n: int) -> int` - Returns the absolute value of a number
- `floor(n: float) -> int` - Returns the floor of a floating-point number
- `ceil(n: float) -> int` - Returns the ceiling of a floating-point number
- `round(n: float) -> int` - Rounds a floating-point number to the nearest integer
- `max(a: int, b: int) -> int` - Returns the maximum of two numbers
- `min(a: int, b: int) -> int` - Returns the minimum of two numbers

**Usage:**
```raven
import math;

let result: int = math.max(10, 20);
let absolute: int = math.abs(-5);
```

### 2. Collections (`collections.rv`)
Basic collection data structures.

**Classes:**
- `HashMap` - Key-value storage
- `Set` - Unique element storage

**HashMap Methods:**
- `new() -> HashMap` - Creates a new HashMap
- `set(key: string, value: string) -> void` - Sets a key-value pair
- `get(key: string) -> string` - Gets a value by key
- `contains_key(key: string) -> bool` - Checks if key exists
- `remove(key: string) -> void` - Removes a key-value pair
- `len() -> int` - Returns the number of key-value pairs

**Set Methods:**
- `new() -> Set` - Creates a new Set
- `add(element: string) -> void` - Adds an element
- `contains(element: string) -> bool` - Checks if element exists
- `remove(element: string) -> void` - Removes an element
- `len() -> int` - Returns the number of elements

**Usage:**
```raven
import collections;

let map: HashMap = collections.HashMap.new();
map.set("name", "John");
let value: string = map.get("name");

let set: Set = collections.Set.new();
set.add("apple");
set.add("banana");
```

### 3. string Utilities (`string.rv`)
Additional string manipulation functions.

**Functions:**
- `capitalize(s: string) -> string` - Capitalizes the first letter
- `starts_with(s: string, prefix: string) -> bool` - Checks if string starts with prefix
- `ends_with(s: string, suffix: string) -> bool` - Checks if string ends with suffix
- `trim(s: string) -> string` - Removes whitespace from both ends
- `to_lower(s: string) -> string` - Converts to lowercase (placeholder)
- `to_upper(s: string) -> string` - Converts to uppercase (placeholder)

**Usage:**
```raven
import string;

let result: string = string.capitalize("hello world");
let trimmed: string = string.trim("  hello  ");
let starts: bool = string.starts_with("hello", "he");
```

### 4. Time and Date (`time.rv`)
Basic time and date functionality.

**Classes:**
- `DateTime` - Represents a date and time

**DateTime Fields:**
- `year: int` - Year
- `month: int` - Month (1-12)
- `day: int` - Day (1-31)
- `hour: int` - Hour (0-23)
- `minute: int` - Minute (0-59)
- `second: int` - Second (0-59)

**Functions:**
- `now() -> DateTime` - Gets current date and time
- `format_datetime(dt: DateTime, format_str: string) -> string` - Formats DateTime as string
- `days_between(dt1: DateTime, dt2: DateTime) -> int` - Calculates days between dates

**Usage:**
```raven
import time;

let now: DateTime = time.now();
let formatted: string = time.format_datetime(now, "YYYY-MM-DD");
let days: int = time.days_between(now, now);
```

### 5. File System (`filesystem.rv`)
File and directory operations.

**Classes:**
- `FileInfo` - File information structure

**FileInfo Fields:**
- `name: string` - File name
- `size: int` - File size in bytes
- `is_directory: bool` - Whether it's a directory
- `is_file: bool` - Whether it's a file

**Functions:**
- `list_directory(path: string) -> string[]` - Lists directory contents
- `list_files(path: string) -> string[]` - Lists only files
- `list_directories(path: string) -> string[]` - Lists only directories
- `join_path(parts: string[]) -> string` - Joins path components
- `split_path(path: string) -> string[]` - Splits path into components
- `dirname(path: string) -> string` - Gets directory name
- `basename(path: string) -> string` - Gets file name
- `extension(path: string) -> string` - Gets file extension
- `copy_file(source: string, destination: string) -> bool` - Copies a file
- `move_file(source: string, destination: string) -> bool` - Moves a file
- `create_directory(path: string) -> bool` - Creates a directory
- `remove_directory(path: string) -> bool` - Removes a directory
- `remove_file(path: string) -> bool` - Removes a file
- `get_file_info(path: string) -> FileInfo` - Gets file information
- `read_lines(path: string) -> string[]` - Reads file as lines
- `write_lines(path: string, lines: string[]) -> bool` - Writes lines to file
- `find_files(directory: string, pattern: string) -> string[]` - Finds files by pattern
- `find_files_by_extension(directory: string, ext: string) -> string[]` - Finds files by extension
- `is_valid_filename(filename: string) -> bool` - Validates filename
- `sanitize_filename(filename: string) -> string` - Sanitizes filename
- `files_are_equal(path1: string, path2: string) -> bool` - Compares files
- `get_file_hash(path: string) -> string` - Gets file hash
- `create_temp_file(content: string) -> string` - Creates temporary file
- `cleanup_temp_file(path: string) -> bool` - Cleans up temporary file

**Usage:**
```raven
import filesystem;

let files: string[] = filesystem.list_files(".");
let path: string = filesystem.join_path(["dir", "subdir", "file.txt"]);
let info: FileInfo = filesystem.get_file_info("test.txt");
let lines: string[] = filesystem.read_lines("data.txt");
```

### 6. Network (`network.rv`)
HTTP client and network operations.

**Classes:**
- `HttpRequest` - HTTP request structure
- `HttpResponse` - HTTP response structure
- `Url` - URL parsing structure
- `HttpServer` - HTTP server structure
- `WebSocket` - WebSocket connection structure

**HttpRequest Fields:**
- `method: string` - HTTP method (GET, POST, etc.)
- `url: string` - Request URL
- `headers: string[]` - HTTP headers
- `body: string` - Request body

**HttpResponse Fields:**
- `status_code: int` - HTTP status code
- `status_text: string` - HTTP status text
- `headers: string[]` - Response headers
- `body: string` - Response body

**Functions:**
- `GET(url: string) -> HttpResponse` - Makes GET request
- `POST(url: string, body: string) -> HttpResponse` - Makes POST request
- `PUT(url: string, body: string) -> HttpResponse` - Makes PUT request
- `DELETE(url: string) -> HttpResponse` - Makes DELETE request
- `parse_url(url_string: string) -> Url` - Parses URL
- `build_url(protocol: string, host: string, port: int, path: string) -> string` - Builds URL
- `parse_query_string(query: string) -> string[]` - Parses query string
- `get_query_param(query: string, param_name: string) -> string` - Gets query parameter
- `set_header(headers: string[], name: string, value: string) -> string[]` - Sets HTTP header
- `get_header(headers: string[], name: string) -> string` - Gets HTTP header
- `is_success(response: HttpResponse) -> bool` - Checks if response is successful
- `is_redirect(response: HttpResponse) -> bool` - Checks if response is redirect
- `is_client_error(response: HttpResponse) -> bool` - Checks if response is client error
- `is_server_error(response: HttpResponse) -> bool` - Checks if response is server error
- `json_encode(obj: string) -> string` - Encodes to JSON
- `json_decode(json: string) -> string` - Decodes from JSON
- `download_file(url: string, filename: string) -> bool` - Downloads file
- `create_server(port: int) -> HttpServer` - Creates HTTP server
- `add_route(server: HttpServer, method: string, path: string, handler: string) -> HttpServer` - Adds route
- `start_server(server: HttpServer) -> bool` - Starts server
- `connect_websocket(url: string) -> WebSocket` - Connects WebSocket
- `send_message(ws: WebSocket, message: string) -> bool` - Sends WebSocket message
- `close_websocket(ws: WebSocket) -> bool` - Closes WebSocket
- `ping(host: string) -> bool` - Pings host
- `resolve_dns(hostname: string) -> string` - Resolves DNS
- `is_valid_url(url: string) -> bool` - Validates URL
- `is_valid_email(email: string) -> bool` - Validates email

**Usage:**
```raven
import network;

let response: HttpResponse = network.GET("https://api.example.com/data");
if (network.is_success(response)) {
    print(response.body);
}

let server: HttpServer = network.create_server(8080);
server = network.add_route(server, "GET", "/", "handle_root");
network.start_server(server);
```

### 7. Testing (`testing.rv`)
Testing framework and utilities.

**Classes:**
- `TestResult` - Test execution result
- `TestSuite` - Test suite structure
- `MockObject` - Mock object for testing
- `TestConfig` - Test configuration

**TestResult Fields:**
- `name: string` - Test name
- `passed: bool` - Whether test passed
- `error_message: string` - Error message if failed
- `execution_time: float` - Execution time in seconds

**TestSuite Fields:**
- `name: string` - Suite name
- `tests: string[]` - Test names
- `results: TestResult[]` - Test results
- `total_tests: int` - Total number of tests
- `passed_tests: int` - Number of passed tests
- `failed_tests: int` - Number of failed tests

**Assertion Functions:**
- `assert_true(condition: bool, message: string) -> bool` - Asserts condition is true
- `assert_false(condition: bool, message: string) -> bool` - Asserts condition is false
- `assert_equals(actual: string, expected: string, message: string) -> bool` - Asserts equality
- `assert_not_equals(actual: string, expected: string, message: string) -> bool` - Asserts inequality
- `assert_greater_than(actual: int, expected: int, message: string) -> bool` - Asserts greater than
- `assert_less_than(actual: int, expected: int, message: string) -> bool` - Asserts less than
- `assert_greater_than_or_equal(actual: int, expected: int, message: string) -> bool` - Asserts greater than or equal
- `assert_less_than_or_equal(actual: int, expected: int, message: string) -> bool` - Asserts less than or equal
- `assert_contains(haystack: string, needle: string, message: string) -> bool` - Asserts contains
- `assert_not_contains(haystack: string, needle: string, message: string) -> bool` - Asserts not contains
- `assert_starts_with(str: string, prefix: string, message: string) -> bool` - Asserts starts with
- `assert_ends_with(str: string, suffix: string, message: string) -> bool` - Asserts ends with
- `assert_is_empty(str: string, message: string) -> bool` - Asserts empty
- `assert_is_not_empty(str: string, message: string) -> bool` - Asserts not empty
- `assert_is_null(value: string, message: string) -> bool` - Asserts null
- `assert_is_not_null(value: string, message: string) -> bool` - Asserts not null

**Test Execution Functions:**
- `run_test(test_name: string, test_function: string) -> TestResult` - Runs a single test
- `run_test_suite(suite_name: string, tests: string[]) -> TestSuite` - Runs a test suite
- `print_test_results(suite: TestSuite) -> void` - Prints test results
- `generate_test_report(suite: TestSuite) -> string` - Generates test report
- `skip_test(test_name: string, reason: string) -> TestResult` - Skips a test
- `todo_test(test_name: string, description: string) -> TestResult` - Marks test as TODO
- `benchmark_function(function_name: string, iterations: int) -> float` - Benchmarks function
- `performance_test(test_name: string, max_execution_time: float) -> TestResult` - Performance test

**Mock Functions:**
- `create_mock(name: string) -> MockObject` - Creates mock object
- `mock_call(mock: MockObject, method_name: string, args: string[]) -> string` - Mocks method call
- `verify_mock_calls(mock: MockObject, expected_calls: string[]) -> bool` - Verifies mock calls
- `reset_mock(mock: MockObject) -> MockObject` - Resets mock

**Test Data Generators:**
- `generate_random_string(length: int) -> string` - Generates random string
- `generate_random_int(min: int, max: int) -> int` - Generates random integer
- `generate_test_data(count: int) -> string[]` - Generates test data

**Usage:**
```raven
import testing;

// Assertions
testing.assert_equals("hello", "hello", "Strings should be equal");
testing.assert_true(5 > 3, "5 should be greater than 3");

// Test execution
let tests: string[] = ["test_math", "test_string", "test_collections"];
let suite: TestSuite = testing.run_test_suite("Math Tests", tests);
testing.print_test_results(suite);

// Mock objects
let mock: MockObject = testing.create_mock("Calculator");
let result: string = testing.mock_call(mock, "add", ["2", "3"]);
```

## Installation

These modules are automatically available when you import them in your Raven programs. No additional installation is required.

## Contributing

When adding new modules to the standard library:

1. Create a new `.rv` file in this directory
2. Use the `export` keyword for functions and classes you want to make available
3. Add comprehensive documentation to this README
4. Include usage examples
5. Test your module thoroughly

## Future Modules

Planned modules include:
- `crypto.rv` - Cryptographic functions
- `regex.rv` - Regular expression support
- `xml.rv` - XML parsing and generation
- `csv.rv` - CSV file handling
- `config.rv` - Configuration file management
