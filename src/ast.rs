#[derive(Debug, Clone)]
pub enum ASTNode {
    VariableDecl(String, Box<Expression>),                    // old style: let x = 5;
    VariableDeclTyped(String, String, Box<Expression>),       // new style: let x: int = 5;
    FunctionDecl(String, String, Vec<Parameter>, Box<ASTNode>), // name, return_type, params, body
    ForLoop(Box<ASTNode>, Box<Expression>, Box<ASTNode>, Box<ASTNode>), // init, condition, increment, body
    WhileLoop(Box<Expression>, Box<ASTNode>),
    Assignment(String, Box<Expression>),
    IfStatement(
        Box<Expression>,       // The condition expression of the 'if'
        Box<ASTNode>,          // The body of the 'if' block
        Option<Box<ASTNode>>,  // The 'else if' (optional)
        Option<Box<ASTNode>>,  // The 'else' (optional)
    ),
    Block(Vec<ASTNode>),
    Print(Box<Expression>),
    FunctionCall(String, Vec<Expression>), // function_name, arguments (as statement)
    MethodCall(Box<Expression>, String, Vec<Expression>), // object.method(args) (as statement)
    Return(Box<Expression>),
    Import(String, Option<String>), // module_name, optional alias
    ImportSelective(String, Vec<String>), // module_name, selected_items
    Export(Box<ASTNode>), // export any AST node
}


#[derive(Debug, Clone)]
pub enum Expression {
    Integer(i64),
    Float(f64),
    Boolean(bool),
    StringLiteral(String),
    Identifier(String),
    BinaryOp(Box<Expression>, Operator, Box<Expression>),
    FunctionCall(String, Vec<Expression>), // function_name, arguments
    ArrayLiteral(Vec<Expression>), // [1, 2, 3]
    ArrayIndex(Box<Expression>, Box<Expression>), // array[index]
    MethodCall(Box<Expression>, String, Vec<Expression>), // object.method(args)
}


#[derive(Debug, Clone)]
pub enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Equal,
    NotEqual,
    LessThan,
    GreaterThan,
    LessEqual,
    GreaterEqual,
    And, 
    Or,
}



#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub param_type: String,
}

#[derive(Debug, Clone)]
pub enum Type {
    Int,
    Float,
    Bool,
    String,
    Void,
    Array(Box<Type>), // int[] -> Array(Box::new(Type::Int))
}
