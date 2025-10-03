# 🧠 Memory & Execution Visualization

## What Happens in Memory During Execution

Let's trace this program step-by-step and show what's in memory:

```raven
let x: int = 5;
let y: int = x + 10;
print(y);
```

---

## 📊 Step-by-Step Memory States

### Initial State
```
┌─────────────────────────────────────────┐
│         Interpreter State               │
├─────────────────────────────────────────┤
│ variables: HashMap {}                   │
│ functions: HashMap {}                   │
│ return_value: None                      │
└─────────────────────────────────────────┘
```

---

### After: `let x: int = 5;`

**What happens:**
1. Parser gave us: `VariableDeclTyped("x", "int", Integer(5))`
2. Interpreter calls: `execute(VariableDeclTyped(...))`
3. Evaluates: `Integer(5)` → `Value::Int(5)`
4. Stores: `variables.insert("x", Value::Int(5))`

**Memory State:**
```
┌─────────────────────────────────────────┐
│         Interpreter State               │
├─────────────────────────────────────────┤
│ variables: HashMap {                    │
│   "x" → Value::Int(5)                   │
│ }                                       │
│ functions: HashMap {}                   │
│ return_value: None                      │
└─────────────────────────────────────────┘

   Heap Memory:
   ┌──────────┐
   │ String   │  (key "x")
   │  "x"     │
   └──────────┘
        ↓
   ┌──────────┐
   │ Value    │
   │ Int(5)   │  (8 bytes)
   └──────────┘
```

---

### After: `let y: int = x + 10;`

**What happens:**
1. AST node: `VariableDeclTyped("y", "int", BinaryOp(Identifier("x"), Add, Integer(10)))`
2. Evaluate expression:
   ```
   eval(BinaryOp(Identifier("x"), Add, Integer(10)))
     ├─ eval(Identifier("x"))
     │    └─ Look up in HashMap: variables["x"] → Value::Int(5)
     │    └─ Return: Value::Int(5)
     │
     ├─ eval(Integer(10))
     │    └─ Return: Value::Int(10)
     │
     └─ Apply operation: 5 + 10 → Value::Int(15)
   ```
3. Store: `variables.insert("y", Value::Int(15))`

**Memory State:**
```
┌─────────────────────────────────────────┐
│         Interpreter State               │
├─────────────────────────────────────────┤
│ variables: HashMap {                    │
│   "x" → Value::Int(5)                   │
│   "y" → Value::Int(15)     ← NEW!       │
│ }                                       │
│ functions: HashMap {}                   │
│ return_value: None                      │
└─────────────────────────────────────────┘

   HashMap Internal Structure:
   ┌─────────────────────────────────┐
   │  Key: "x"  →  Value: Int(5)     │
   │  Key: "y"  →  Value: Int(15)    │
   └─────────────────────────────────┘
```

---

### Executing: `print(y);`

**What happens:**
1. AST node: `Print(Identifier("y"))`
2. Evaluate: `Identifier("y")`
   - Look up: `variables["y"]` → `Value::Int(15)`
3. Print: `println!("{}", Value::Int(15))`
   - **OUTPUT**: `15`

**Memory State:** (unchanged)
```
┌─────────────────────────────────────────┐
│         Interpreter State               │
├─────────────────────────────────────────┤
│ variables: HashMap {                    │
│   "x" → Value::Int(5)                   │
│   "y" → Value::Int(15)                  │
│ }                                       │
│ functions: HashMap {}                   │
│ return_value: None                      │
└─────────────────────────────────────────┘

Console: 15
```

---

## 🔄 Loop Execution in Memory

```raven
let i: int = 0;
while (i < 3) {
    print(i);
    i = i + 1;
}
```

### Iteration 1

**Before loop:**
```
variables = { "i" → Value::Int(0) }
```

**Check condition:** `i < 3`
```
eval(BinaryOp(Identifier("i"), LessThan, Integer(3)))
  ├─ eval(Identifier("i")) → variables["i"] → Value::Int(0)
  └─ eval(Integer(3)) → Value::Int(3)
  └─ 0 < 3 → Value::Bool(true) ✓
```

**Execute body:**
1. `print(i)` → Output: `0`
2. `i = i + 1`
   ```
   eval(BinaryOp(Identifier("i"), Add, Integer(1)))
     ├─ eval(Identifier("i")) → Value::Int(0)
     └─ eval(Integer(1)) → Value::Int(1)
     └─ 0 + 1 → Value::Int(1)
   ```
   Store: `variables["i"] = Value::Int(1)`

**After iteration 1:**
```
variables = { "i" → Value::Int(1) }
```

### Iteration 2

**Check:** `1 < 3` → `true` ✓  
**Execute:** Print `1`, set `i = 2`  
**After:** `variables = { "i" → Value::Int(2) }`

### Iteration 3

**Check:** `2 < 3` → `true` ✓  
**Execute:** Print `2`, set `i = 3`  
**After:** `variables = { "i" → Value::Int(3) }`

### Iteration 4

**Check:** `3 < 3` → `false` ✗  
**Exit loop**

---

## 🎯 Function Call Memory

```raven
fun add(a: int, b: int) -> int {
    return a + b;
}
```

### After Function Declaration

**Memory State:**
```
┌──────────────────────────────────────────────┐
│         Interpreter State                    │
├──────────────────────────────────────────────┤
│ variables: HashMap {}                        │
│                                              │
│ functions: HashMap {                         │
│   "add" → Function {                         │
│     params: [                                │
│       Parameter { name: "a", type: "int" },  │
│       Parameter { name: "b", type: "int" }   │
│     ],                                       │
│     body: Block([                            │
│       Return(BinaryOp(...))                  │
│     ])                                       │
│   }                                          │
│ }                                            │
│                                              │
│ return_value: None                           │
└──────────────────────────────────────────────┘
```

### During Function Call: `add(5, 10)`

**Step 1: Save state**
```
saved_vars = current variables
```

**Step 2: Bind parameters**
```
variables = {
  "a" → Value::Int(5)
  "b" → Value::Int(10)
}
```

**Step 3: Execute body**
```
execute(Return(BinaryOp(Identifier("a"), Add, Identifier("b"))))
  ├─ eval(BinaryOp(...))
  │    ├─ eval(Identifier("a")) → Value::Int(5)
  │    └─ eval(Identifier("b")) → Value::Int(10)
  │    └─ 5 + 10 → Value::Int(15)
  └─ Set return_value = Some(Value::Int(15))
```

**Memory during execution:**
```
┌──────────────────────────────────────────────┐
│         Interpreter State                    │
├──────────────────────────────────────────────┤
│ variables: HashMap {                         │
│   "a" → Value::Int(5)                        │
│   "b" → Value::Int(10)                       │
│ }                                            │
│                                              │
│ return_value: Some(Value::Int(15))  ← SET!  │
└──────────────────────────────────────────────┘
```

**Step 4: Restore state**
```
result = return_value → Value::Int(15)
variables = saved_vars
return_value = None
```

**Return:** `Value::Int(15)`

---

## 🧮 Expression Evaluation Stack

When evaluating complex expressions, the Rust call stack handles the recursion:

**Expression:** `(5 + 10) * 2`
**AST:** `BinaryOp(BinaryOp(5, Add, 10), Multiply, 2)`

```
Rust Call Stack During Evaluation:

┌────────────────────────────────────────┐
│ eval_expression(BinaryOp(...))         │ ← Start here
│   left_val = ?                         │
└────────────────────────────────────────┘
            ↓ Calls
┌────────────────────────────────────────┐
│ eval_expression(BinaryOp(5, Add, 10))  │
│   left_val = ?                         │
└────────────────────────────────────────┘
            ↓ Calls
┌────────────────────────────────────────┐
│ eval_expression(Integer(5))            │
│   Returns: Value::Int(5)               │ ← Returns
└────────────────────────────────────────┘
            ↓ Then calls
┌────────────────────────────────────────┐
│ eval_expression(Integer(10))           │
│   Returns: Value::Int(10)              │ ← Returns
└────────────────────────────────────────┘
            ↓ Computes
┌────────────────────────────────────────┐
│ eval_expression(BinaryOp(5, Add, 10))  │
│   5 + 10 = 15                          │
│   Returns: Value::Int(15)              │ ← Returns
└────────────────────────────────────────┘
            ↓ Then calls
┌────────────────────────────────────────┐
│ eval_expression(Integer(2))            │
│   Returns: Value::Int(2)               │ ← Returns
└────────────────────────────────────────┘
            ↓ Finally computes
┌────────────────────────────────────────┐
│ eval_expression(BinaryOp(...))         │
│   15 * 2 = 30                          │
│   Returns: Value::Int(30)              │ ← Final result
└────────────────────────────────────────┘
```

---

## 🎭 The Magic: Pattern Matching

Every execution decision uses Rust's pattern matching:

```rust
match node {
    ASTNode::VariableDecl(...) => { /* handle variable */ }
    ASTNode::Print(...) => { /* handle print */ }
    ASTNode::IfStatement(...) => { /* handle if */ }
    // etc...
}
```

This is like a **giant switch statement** that routes each AST node to its handler!

---

## 💡 Key Takeaways

1. **HashMap = Variable Storage**: Every variable lookup is a HashMap lookup
2. **Recursive Evaluation**: Expressions are evaluated by recursively calling `eval_expression`
3. **Direct Execution**: No compilation - the interpreter directly performs actions
4. **Pattern Matching**: Rust's `match` dispatches to the right handler for each AST node
5. **Call Stack**: Rust's call stack handles expression evaluation recursion

This is a **Tree-Walking Interpreter** - we literally walk the AST tree and execute as we go! 🌲

