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
- `set(key: String, value: String) -> void` - Sets a key-value pair
- `get(key: String) -> String` - Gets a value by key
- `contains_key(key: String) -> bool` - Checks if key exists
- `remove(key: String) -> void` - Removes a key-value pair
- `len() -> int` - Returns the number of key-value pairs

**Set Methods:**
- `new() -> Set` - Creates a new Set
- `add(element: String) -> void` - Adds an element
- `contains(element: String) -> bool` - Checks if element exists
- `remove(element: String) -> void` - Removes an element
- `len() -> int` - Returns the number of elements

**Usage:**
```raven
import collections;

let map: HashMap = collections.HashMap.new();
map.set("name", "John");
let value: String = map.get("name");

let set: Set = collections.Set.new();
set.add("apple");
set.add("banana");
```

### 3. String Utilities (`string.rv`)
Additional string manipulation functions.

**Functions:**
- `capitalize(s: String) -> String` - Capitalizes the first letter
- `starts_with(s: String, prefix: String) -> bool` - Checks if string starts with prefix
- `ends_with(s: String, suffix: String) -> bool` - Checks if string ends with suffix
- `trim(s: String) -> String` - Removes whitespace from both ends
- `to_lower(s: String) -> String` - Converts to lowercase (placeholder)
- `to_upper(s: String) -> String` - Converts to uppercase (placeholder)

**Usage:**
```raven
import string;

let result: String = string.capitalize("hello world");
let trimmed: String = string.trim("  hello  ");
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
- `format_datetime(dt: DateTime, format_str: String) -> String` - Formats DateTime as string
- `days_between(dt1: DateTime, dt2: DateTime) -> int` - Calculates days between dates

**Usage:**
```raven
import time;

let now: DateTime = time.now();
let formatted: String = time.format_datetime(now, "YYYY-MM-DD");
let days: int = time.days_between(now, now);
```

### 5. File System (`filesystem.rv`)
File and directory operations.

**Classes:**
- `FileInfo` - File information structure

**FileInfo Fields:**
- `name: String` - File name
- `size: int` - File size in bytes
- `is_directory: bool` - Whether it's a directory
- `is_file: bool` - Whether it's a file

**Functions:**
- `list_directory(path: String) -> String[]` - Lists directory contents
- `list_files(path: String) -> String[]` - Lists only files
- `list_directories(path: String) -> String[]` - Lists only directories
- `join_path(parts: String[]) -> String` - Joins path components
- `split_path(path: String) -> String[]` - Splits path into components
- `dirname(path: String) -> String` - Gets directory name
- `basename(path: String) -> String` - Gets file name
- `extension(path: String) -> String` - Gets file extension
- `copy_file(source: String, destination: String) -> bool` - Copies a file
- `move_file(source: String, destination: String) -> bool` - Moves a file
- `create_directory(path: String) -> bool` - Creates a directory
- `remove_directory(path: String) -> bool` - Removes a directory
- `remove_file(path: String) -> bool` - Removes a file
- `get_file_info(path: String) -> FileInfo` - Gets file information
- `read_lines(path: String) -> String[]` - Reads file as lines
- `write_lines(path: String, lines: String[]) -> bool` - Writes lines to file
- `find_files(directory: String, pattern: String) -> String[]` - Finds files by pattern
- `find_files_by_extension(directory: String, ext: String) -> String[]` - Finds files by extension
- `is_valid_filename(filename: String) -> bool` - Validates filename
- `sanitize_filename(filename: String) -> String` - Sanitizes filename
- `files_are_equal(path1: String, path2: String) -> bool` - Compares files
- `get_file_hash(path: String) -> String` - Gets file hash
- `create_temp_file(content: String) -> String` - Creates temporary file
- `cleanup_temp_file(path: String) -> bool` - Cleans up temporary file

**Usage:**
```raven
import filesystem;

let files: String[] = filesystem.list_files(".");
let path: String = filesystem.join_path(["dir", "subdir", "file.txt"]);
let info: FileInfo = filesystem.get_file_info("test.txt");
let lines: String[] = filesystem.read_lines("data.txt");
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
- `method: String` - HTTP method (GET, POST, etc.)
- `url: String` - Request URL
- `headers: String[]` - HTTP headers
- `body: String` - Request body

**HttpResponse Fields:**
- `status_code: int` - HTTP status code
- `status_text: String` - HTTP status text
- `headers: String[]` - Response headers
- `body: String` - Response body

**Functions:**
- `GET(url: String) -> HttpResponse` - Makes GET request
- `POST(url: String, body: String) -> HttpResponse` - Makes POST request
- `PUT(url: String, body: String) -> HttpResponse` - Makes PUT request
- `DELETE(url: String) -> HttpResponse` - Makes DELETE request
- `parse_url(url_string: String) -> Url` - Parses URL
- `build_url(protocol: String, host: String, port: int, path: String) -> String` - Builds URL
- `parse_query_string(query: String) -> String[]` - Parses query string
- `get_query_param(query: String, param_name: String) -> String` - Gets query parameter
- `set_header(headers: String[], name: String, value: String) -> String[]` - Sets HTTP header
- `get_header(headers: String[], name: String) -> String` - Gets HTTP header
- `is_success(response: HttpResponse) -> bool` - Checks if response is successful
- `is_redirect(response: HttpResponse) -> bool` - Checks if response is redirect
- `is_client_error(response: HttpResponse) -> bool` - Checks if response is client error
- `is_server_error(response: HttpResponse) -> bool` - Checks if response is server error
- `json_encode(obj: String) -> String` - Encodes to JSON
- `json_decode(json: String) -> String` - Decodes from JSON
- `download_file(url: String, filename: String) -> bool` - Downloads file
- `create_server(port: int) -> HttpServer` - Creates HTTP server
- `add_route(server: HttpServer, method: String, path: String, handler: String) -> HttpServer` - Adds route
- `start_server(server: HttpServer) -> bool` - Starts server
- `connect_websocket(url: String) -> WebSocket` - Connects WebSocket
- `send_message(ws: WebSocket, message: String) -> bool` - Sends WebSocket message
- `close_websocket(ws: WebSocket) -> bool` - Closes WebSocket
- `ping(host: String) -> bool` - Pings host
- `resolve_dns(hostname: String) -> String` - Resolves DNS
- `is_valid_url(url: String) -> bool` - Validates URL
- `is_valid_email(email: String) -> bool` - Validates email

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
- `name: String` - Test name
- `passed: bool` - Whether test passed
- `error_message: String` - Error message if failed
- `execution_time: float` - Execution time in seconds

**TestSuite Fields:**
- `name: String` - Suite name
- `tests: String[]` - Test names
- `results: TestResult[]` - Test results
- `total_tests: int` - Total number of tests
- `passed_tests: int` - Number of passed tests
- `failed_tests: int` - Number of failed tests

**Assertion Functions:**
- `assert_true(condition: bool, message: String) -> bool` - Asserts condition is true
- `assert_false(condition: bool, message: String) -> bool` - Asserts condition is false
- `assert_equals(actual: String, expected: String, message: String) -> bool` - Asserts equality
- `assert_not_equals(actual: String, expected: String, message: String) -> bool` - Asserts inequality
- `assert_greater_than(actual: int, expected: int, message: String) -> bool` - Asserts greater than
- `assert_less_than(actual: int, expected: int, message: String) -> bool` - Asserts less than
- `assert_greater_than_or_equal(actual: int, expected: int, message: String) -> bool` - Asserts greater than or equal
- `assert_less_than_or_equal(actual: int, expected: int, message: String) -> bool` - Asserts less than or equal
- `assert_contains(haystack: String, needle: String, message: String) -> bool` - Asserts contains
- `assert_not_contains(haystack: String, needle: String, message: String) -> bool` - Asserts not contains
- `assert_starts_with(str: String, prefix: String, message: String) -> bool` - Asserts starts with
- `assert_ends_with(str: String, suffix: String, message: String) -> bool` - Asserts ends with
- `assert_is_empty(str: String, message: String) -> bool` - Asserts empty
- `assert_is_not_empty(str: String, message: String) -> bool` - Asserts not empty
- `assert_is_null(value: String, message: String) -> bool` - Asserts null
- `assert_is_not_null(value: String, message: String) -> bool` - Asserts not null

**Test Execution Functions:**
- `run_test(test_name: String, test_function: String) -> TestResult` - Runs a single test
- `run_test_suite(suite_name: String, tests: String[]) -> TestSuite` - Runs a test suite
- `print_test_results(suite: TestSuite) -> void` - Prints test results
- `generate_test_report(suite: TestSuite) -> String` - Generates test report
- `skip_test(test_name: String, reason: String) -> TestResult` - Skips a test
- `todo_test(test_name: String, description: String) -> TestResult` - Marks test as TODO
- `benchmark_function(function_name: String, iterations: int) -> float` - Benchmarks function
- `performance_test(test_name: String, max_execution_time: float) -> TestResult` - Performance test

**Mock Functions:**
- `create_mock(name: String) -> MockObject` - Creates mock object
- `mock_call(mock: MockObject, method_name: String, args: String[]) -> String` - Mocks method call
- `verify_mock_calls(mock: MockObject, expected_calls: String[]) -> bool` - Verifies mock calls
- `reset_mock(mock: MockObject) -> MockObject` - Resets mock

**Test Data Generators:**
- `generate_random_string(length: int) -> String` - Generates random string
- `generate_random_int(min: int, max: int) -> int` - Generates random integer
- `generate_test_data(count: int) -> String[]` - Generates test data

**Usage:**
```raven
import testing;

// Assertions
testing.assert_equals("hello", "hello", "Strings should be equal");
testing.assert_true(5 > 3, "5 should be greater than 3");

// Test execution
let tests: String[] = ["test_math", "test_string", "test_collections"];
let suite: TestSuite = testing.run_test_suite("Math Tests", tests);
testing.print_test_results(suite);

// Mock objects
let mock: MockObject = testing.create_mock("Calculator");
let result: String = testing.mock_call(mock, "add", ["2", "3"]);
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
