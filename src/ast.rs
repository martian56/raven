#[derive(Debug, Clone)]
pub enum ASTNode {
    VariableDecl(String, Box<Expression>),
    FunctionDecl(String, Vec<Parameter>, Box<Expression>),
    ForLoop(Box<Expression>, Box<Expression>, Box<Expression>, Box<ASTNode>),
    WhileLoop(Box<Expression>, Box<ASTNode>),
    Assignment(String, Box<Expression>),
    IfStatement(Box<Expression>, Box<ASTNode>),
    Block(Vec<ASTNode>),
}

#[derive(Debug, Clone)]
pub enum Expression {
    Integer(i64),
    StringLiteral(String),
    Identifier(String),
    BinaryOp(Box<Expression>, Operator, Box<Expression>),
}

#[derive(Debug, Clone)]
pub enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Equal,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub param_type: String,
}
