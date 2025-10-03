# 🔍 Execution Trace - How The Interpreter Works

## The Interpreter's State

The interpreter maintains runtime state in three main components:

```rust
pub struct Interpreter {
    variables: HashMap<String, Value>,    // Variable storage at runtime
    functions: HashMap<String, Function>, // Function definitions
    return_value: Option<Value>,          // For handling return statements
}
```

### Example: Tracing Variable Storage

Let's trace this program:

```raven
let x: int = 5;
let y: int = 10;
if (x < y) {
    let z: int = x + y;
    print(z);
}
```

## Step-by-Step Execution

### STEP 1: Initialize Interpreter

```rust
let mut interpreter = Interpreter::new();
// State: variables = {}, functions = {}, return_value = None
```

### STEP 2: Execute Block

The AST is:
```
Block([
    VariableDeclTyped("x", "int", Integer(5)),
    VariableDeclTyped("y", "int", Integer(10)),
    IfStatement(...)
])
```

The interpreter calls `execute()` on the Block:

```rust
fn execute(&mut self, node: &ASTNode) -> Result<Value, String> {
    match node {
        ASTNode::Block(statements) => {
            // Loop through each statement and execute it
            for stmt in statements {
                self.execute(stmt)?;
            }
            Ok(Value::Void)
        }
    }
}
```

### STEP 3: Execute `let x: int = 5;`

```rust
ASTNode::VariableDeclTyped(name, _type, expr) => {
    // 1. Evaluate the expression (the value)
    let value = self.eval_expression(expr)?;
    
    // 2. Store it in the variables HashMap
    self.variables.insert(name.clone(), value);
    
    Ok(Value::Void)
}
```

**Detailed Flow:**

1. **Call**: `self.eval_expression(Integer(5))`
   ```rust
   fn eval_expression(&mut self, expr: &Expression) -> Result<Value, String> {
       match expr {
           Expression::Integer(i) => Ok(Value::Int(*i))  // Returns Value::Int(5)
       }
   }
   ```
   **Returns**: `Value::Int(5)`

2. **Store**: `self.variables.insert("x", Value::Int(5))`
   
   **State After**:
   ```
   variables = {
       "x": Value::Int(5)
   }
   ```

### STEP 4: Execute `let y: int = 10;`

Same process as above.

**State After**:
```
variables = {
    "x": Value::Int(5),
    "y": Value::Int(10)
}
```

### STEP 5: Execute the If Statement

```rust
ASTNode::IfStatement(condition, then_block, else_if, else_block) => {
    // 1. Evaluate the condition
    let cond_value = self.eval_expression(condition)?;
    
    // 2. Check if it's true
    if let Value::Bool(true) = cond_value {
        self.execute(then_block)  // Execute the then block
    } else if let Some(else_if_node) = else_if {
        self.execute(else_if_node)
    } else if let Some(else_node) = else_block {
        self.execute(else_node)
    } else {
        Ok(Value::Void)
    }
}
```

**Detailed Flow:**

#### 5a. Evaluate Condition: `x < y`

The condition is: `BinaryOp(Identifier("x"), LessThan, Identifier("y"))`

```rust
fn eval_expression(&mut self, expr: &Expression) -> Result<Value, String> {
    match expr {
        Expression::BinaryOp(left, op, right) => {
            // 1. Evaluate left side
            let left_val = self.eval_expression(left)?;
            
            // 2. Evaluate right side
            let right_val = self.eval_expression(right)?;
            
            // 3. Apply the operation
            match (left_val, op, right_val) {
                (Value::Int(l), Operator::LessThan, Value::Int(r)) => {
                    Ok(Value::Bool(l < r))
                }
            }
        }
    }
}
```

**Breaking it down:**

1. **Evaluate left**: `Identifier("x")`
   ```rust
   Expression::Identifier(name) => {
       self.variables.get(name)           // Look up "x" in HashMap
           .cloned()                       // Clone the value
           .ok_or_else(|| format!("..."))  // Error if not found
   }
   ```
   **Lookup**: `variables["x"]` → `Value::Int(5)`
   **Returns**: `Value::Int(5)`

2. **Evaluate right**: `Identifier("y")`
   **Lookup**: `variables["y"]` → `Value::Int(10)`
   **Returns**: `Value::Int(10)`

3. **Apply operation**: `5 < 10`
   ```rust
   match (Value::Int(5), Operator::LessThan, Value::Int(10)) {
       (Value::Int(l), Operator::LessThan, Value::Int(r)) => {
           Ok(Value::Bool(l < r))  // 5 < 10 = true
       }
   }
   ```
   **Returns**: `Value::Bool(true)`

#### 5b. Condition is True - Execute Then Block

The then block is:
```
Block([
    VariableDeclTyped("z", "int", BinaryOp(Identifier("x"), Add, Identifier("y"))),
    Print(Identifier("z"))
])
```

### STEP 6: Execute `let z: int = x + y;`

1. **Evaluate**: `BinaryOp(Identifier("x"), Add, Identifier("y"))`

   - Evaluate left: `Identifier("x")` → Lookup → `Value::Int(5)`
   - Evaluate right: `Identifier("y")` → Lookup → `Value::Int(10)`
   - Apply operation: `5 + 10`
     ```rust
     match (Value::Int(5), Operator::Add, Value::Int(10)) {
         (Value::Int(l), Operator::Add, Value::Int(r)) => {
             Ok(Value::Int(l + r))  // 5 + 10 = 15
         }
     }
     ```
   - **Returns**: `Value::Int(15)`

2. **Store**: `self.variables.insert("z", Value::Int(15))`

**State After**:
```
variables = {
    "x": Value::Int(5),
    "y": Value::Int(10),
    "z": Value::Int(15)
}
```

### STEP 7: Execute `print(z);`

```rust
ASTNode::Print(expr) => {
    // 1. Evaluate the expression
    let value = self.eval_expression(expr)?;
    
    // 2. Print it!
    println!("{}", value);
    
    Ok(Value::Void)
}
```

1. **Evaluate**: `Identifier("z")`
   - Lookup: `variables["z"]` → `Value::Int(15)`
   - **Returns**: `Value::Int(15)`

2. **Print**: `println!("{}", Value::Int(15))`
   - **Output**: `15`

### Final State

```
variables = {
    "x": Value::Int(5),
    "y": Value::Int(10),
    "z": Value::Int(15)
}

Console output: 15
```

## 🔄 Loop Execution Example

Let's trace a while loop:

```raven
let i: int = 0;
while (i < 3) {
    print(i);
    i = i + 1;
}
```

### While Loop Implementation

```rust
ASTNode::WhileLoop(condition, body) => {
    loop {  // Infinite Rust loop
        // 1. Evaluate condition
        let cond_value = self.eval_expression(condition)?;
        
        // 2. Check if true
        if let Value::Bool(true) = cond_value {
            // 3. Execute body
            self.execute(body)?;
            
            // 4. Check for return (breaks loop if function returns)
            if self.return_value.is_some() {
                break;
            }
        } else {
            break;  // Condition is false - exit loop
        }
    }
    Ok(Value::Void)
}
```

### Execution Trace

**Initial State**: `variables = { "i": Value::Int(0) }`

#### Iteration 1:
1. **Check**: `i < 3` → `0 < 3` → `true` ✓
2. **Execute body**:
   - `print(i)` → Print `0`
   - `i = i + 1` → Evaluate `0 + 1` = `1`, Store `variables["i"] = Value::Int(1)`
3. **Continue loop**

**State**: `variables = { "i": Value::Int(1) }`

#### Iteration 2:
1. **Check**: `i < 3` → `1 < 3` → `true` ✓
2. **Execute body**:
   - `print(i)` → Print `1`
   - `i = i + 1` → Store `variables["i"] = Value::Int(2)`
3. **Continue loop**

**State**: `variables = { "i": Value::Int(2) }`

#### Iteration 3:
1. **Check**: `i < 3` → `2 < 3` → `true` ✓
2. **Execute body**:
   - `print(i)` → Print `2`
   - `i = i + 1` → Store `variables["i"] = Value::Int(3)`
3. **Continue loop**

**State**: `variables = { "i": Value::Int(3) }`

#### Iteration 4:
1. **Check**: `i < 3` → `3 < 3` → `false` ✗
2. **Break** - Exit loop

**Output**:
```
0
1
2
```

## 🎯 Function Calls (Advanced)

Let's trace a function call:

```raven
fun add(a: int, b: int) -> int {
    return a + b;
}

let result: int = 15;  // Simulated call
```

### Function Declaration

```rust
ASTNode::FunctionDecl(name, _return_type, params, body) => {
    // Store the function definition
    self.functions.insert(
        name.clone(),
        Function {
            params: params.clone(),
            body: (**body).clone(),
        },
    );
    Ok(Value::Void)
}
```

**State After Declaration**:
```
functions = {
    "add": Function {
        params: [Parameter { name: "a", type: "int" }, 
                 Parameter { name: "b", type: "int" }],
        body: Block([Return(...)])
    }
}
```

### Function Call (Hypothetical)

```rust
pub fn call_function(&mut self, name: &str, args: Vec<Value>) -> Result<Value, String> {
    let func = self.functions.get(name).cloned()?;
    
    // 1. SAVE current variables (for scoping)
    let saved_vars = self.variables.clone();
    
    // 2. BIND parameters to argument values
    for (i, param) in func.params.iter().enumerate() {
        self.variables.insert(param.name.clone(), args[i].clone());
    }
    // Now variables = { "a": Value::Int(5), "b": Value::Int(10) }
    
    // 3. EXECUTE function body
    self.return_value = None;
    self.execute(&func.body)?;
    
    // 4. GET return value
    let result = self.return_value.clone().unwrap_or(Value::Void);
    self.return_value = None;
    
    // 5. RESTORE variables (exit scope)
    self.variables = saved_vars;
    
    Ok(result)
}
```

**Call**: `add(5, 10)`

1. **Save**: `saved_vars = { current variables }`
2. **Bind**: `variables = { "a": Value::Int(5), "b": Value::Int(10) }`
3. **Execute body**: 
   - `return a + b`
   - Evaluate `a + b` → `5 + 10` → `Value::Int(15)`
   - Set `self.return_value = Some(Value::Int(15))`
4. **Get result**: `Value::Int(15)`
5. **Restore**: `variables = saved_vars`
6. **Return**: `Value::Int(15)`

## 📊 Memory Model

### Variable Storage

Variables are stored in a **HashMap** (hash table):

```
Memory Layout:
┌─────────────────────────────────┐
│     HashMap<String, Value>      │
├─────────────┬───────────────────┤
│    Key      │      Value        │
├─────────────┼───────────────────┤
│    "x"      │  Value::Int(5)    │
│    "y"      │  Value::Int(10)   │
│    "z"      │  Value::Int(15)   │
│  "message"  │ Value::String(...) │
└─────────────┴───────────────────┘
```

### Value Representation

```rust
pub enum Value {
    Int(i64),        // 8 bytes on stack
    Float(f64),      // 8 bytes on stack  
    Bool(bool),      // 1 byte on stack
    String(String),  // Pointer to heap-allocated string
    Void,            // Zero-sized
}
```

## 🎬 Complete Execution Flow

```
User runs: raven -f program.rv
          ↓
    main.rs starts
          ↓
1. Read file → String
          ↓
2. Lexer::new(source) → Creates lexer
          ↓
3. Parser::new(lexer) → Creates parser
          ↓
4. parser.parse() → Returns AST
          ↓
5. TypeChecker::check(&ast) → Validates types
          ↓
6. Interpreter::new() → Creates interpreter
          ↓
7. interpreter.execute(&ast) → EXECUTION STARTS HERE
          ↓
    ┌─────────────────────────┐
    │  execute() matches on   │
    │  ASTNode type           │
    └─────────────────────────┘
          ↓
    For each statement:
          ↓
    ┌────────────────────────────────┐
    │  VariableDecl?                 │
    │    → eval_expression()         │
    │    → store in HashMap          │
    ├────────────────────────────────┤
    │  Assignment?                   │
    │    → eval_expression()         │
    │    → update HashMap            │
    ├────────────────────────────────┤
    │  IfStatement?                  │
    │    → eval_expression(condition)│
    │    → execute(then/else block)  │
    ├────────────────────────────────┤
    │  WhileLoop?                    │
    │    → loop {                    │
    │         check condition        │
    │         execute body           │
    │       }                        │
    ├────────────────────────────────┤
    │  Print?                        │
    │    → eval_expression()         │
    │    → println!()                │
    └────────────────────────────────┘
          ↓
    Program finishes
```

## 🔑 Key Insights

### 1. **Direct AST Walking**
The interpreter doesn't compile to anything - it directly walks the AST tree and performs actions.

### 2. **HashMap for Variables**
All variables are stored in a HashMap. Looking up `x` is just:
```rust
self.variables.get("x")
```

### 3. **Recursive Evaluation**
Expressions are evaluated recursively:
```
eval(a + b * c)
  → eval(a) + eval(b * c)
  → eval(a) + (eval(b) * eval(c))
```

### 4. **State Mutation**
The interpreter mutates its state (`self.variables`) as it executes.

### 5. **No Compilation**
There's no bytecode, no machine code - just Rust code walking the tree!

## 🚀 Performance Characteristics

**Speed**: ~10-100x slower than compiled code
- Each operation requires pattern matching
- No optimizations
- HashMap lookups for every variable

**Memory**: AST + Variables
- The entire AST stays in memory
- Variables stored as Values in HashMap

**But**: Perfect for learning and prototyping! 🎓

