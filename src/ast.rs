#[derive(Debug, Clone)]
pub enum ASTNode {
    VariableDecl(String, Box<Expression>),
    VariableDeclTyped(String, String, Box<Expression>),
    FunctionDecl(String, String, Vec<Parameter>, Box<ASTNode>),
    StructDecl(String, Vec<StructField>),
    ImplBlock(String, Vec<(String, String, Vec<Parameter>, Box<ASTNode>)>),
    EnumDecl(String, Vec<String>),
    ForLoop(Box<ASTNode>, Box<Expression>, Box<ASTNode>, Box<ASTNode>),
    WhileLoop(Box<Expression>, Box<ASTNode>),
    Assignment(Box<Expression>, Box<Expression>),
    IfStatement(
        Box<Expression>,
        Box<ASTNode>,
        Option<Box<ASTNode>>,
        Option<Box<ASTNode>>,
    ),
    Block(Vec<ASTNode>),
    Print(Box<Expression>),
    FunctionCall(String, Vec<Expression>),
    MethodCall(Box<Expression>, String, Vec<Expression>),
    ExpressionStatement(Expression),
    Return(Box<Expression>),
    Import(String, Option<String>),
    ImportSelective(String, Vec<String>),
    Export(Box<ASTNode>),
}

#[derive(Debug, Clone)]
pub enum Expression {
    Integer(i64),
    Float(f64),
    Boolean(bool),
    StringLiteral(String),
    Identifier(String),
    BinaryOp(Box<Expression>, Operator, Box<Expression>),
    UnaryOp(Operator, Box<Expression>),
    FunctionCall(String, Vec<Expression>),
    ArrayLiteral(Vec<Expression>),
    ArrayIndex(Box<Expression>, Box<Expression>),
    MethodCall(Box<Expression>, String, Vec<Expression>),
    StructInstantiation(String, Vec<(String, Expression)>),
    FieldAccess(Box<Expression>, String),
    EnumVariant(String, String),
}

#[derive(Debug, Clone)]
pub enum Operator {
    UnaryMinus,
    Not,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
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
pub struct StructField {
    pub name: String,
    pub field_type: String,
}

#[derive(Debug, Clone)]
pub enum Type {
    Int,
    Float,
    Bool,
    String,
    Void,
    Array(Box<Type>),
    Struct(String),
    Enum(String),
}
