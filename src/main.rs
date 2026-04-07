use clap::{Arg, Command};
use raven::code_gen::Interpreter;
use raven::lexer::Lexer;
use raven::parser::Parser;
use raven::repl::delimiter_depth;
use raven::type_checker::TypeChecker;
use std::fs;
use std::process;

fn main() {
    let matches = Command::new("Raven")
        .version(env!("CARGO_PKG_VERSION"))
        .author("martian56 <https://github.com/martian56>")
        .about("Raven compiler and interpreter")
        .arg(
            Arg::new("file")
                .help("The Raven source file to execute")
                .required(false)
                .num_args(0..=1),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .help("Enable verbose output (show tokens, AST, etc.)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("check")
                .short('c')
                .long("check")
                .help("Only check syntax and types, don't run")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("ast")
                .long("show-ast")
                .help("Display the Abstract Syntax Tree")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

    let verbose = matches.get_flag("verbose");
    let check_only = matches.get_flag("check");
    let show_ast = matches.get_flag("ast");

    if let Some(file_name) = matches.get_one::<String>("file") {
        execute_file(file_name, verbose, check_only, show_ast);
    } else {
        start_repl(verbose);
    }
}

fn execute_file(file_name: &str, verbose: bool, check_only: bool, show_ast: bool) {
    let source_code = fs::read_to_string(file_name).unwrap_or_else(|err| {
        eprintln!("❌ Failed to read file '{}': {}", file_name, err);
        process::exit(1);
    });

    if verbose {
        println!("📁 Reading file: {}", file_name);
        println!("─────────────────────────────────────────");
    }

    if verbose {
        println!("\n🔍 LEXING...");
    }

    let lexer = Lexer::new(source_code.clone());

    if verbose {
        let mut lex_clone = Lexer::new(source_code.clone());
        let mut tokens = Vec::new();
        loop {
            let token = lex_clone.next_token();
            if token == raven::lexer::TokenType::EOF {
                tokens.push(token);
                break;
            }
            tokens.push(token);
        }
        println!("   Tokens: {:?}", tokens);
    }

    if verbose {
        println!("\n🌳 PARSING...");
    }

    let mut parser = Parser::new(lexer, source_code.clone());
    let ast = parser.parse().unwrap_or_else(|e| {
        eprintln!(
            "\n❌ Parse error: {}",
            e.with_filename(file_name.to_string()).format()
        );
        process::exit(1);
    });

    if verbose {
        println!("   ✅ Parsing successful!");
        println!("\n📜 Abstract Syntax Tree:");
        println!("{:?}", ast);
    }

    if verbose {
        println!("\n🔎 TYPE CHECKING...");
    }

    let mut type_checker = TypeChecker::new();
    type_checker.check(&ast).unwrap_or_else(|e| {
        eprintln!("\n❌ Type error (in {}):\n{}", file_name, e);
        process::exit(1);
    });

    if verbose {
        println!("   ✅ Type checking passed!");
    }

    if check_only {
        if verbose {
            println!("\n─────────────────────────────────────────");
            println!("✅ Syntax and type checking completed successfully!");
        }
        return;
    }

    if show_ast {
        println!("\n📜 Abstract Syntax Tree:");
        println!("{:#?}", ast);
        return;
    }

    if verbose {
        println!("\n🚀 EXECUTING...");
        println!("─────────────────────────────────────────");
    }

    let mut interpreter = Interpreter::new();
    match interpreter.execute(&ast) {
        Ok(_) => {
            if verbose {
                println!("\n─────────────────────────────────────────");
                println!("✅ Program executed successfully!");
            }
        }
        Err(e) => {
            eprintln!("\n❌ Runtime error: {}", e);
            process::exit(1);
        }
    }
}

enum ReplInput {
    /// stdin closed before any code
    Eof,
    Quit,
    Help,
    Code(String),
}

/// Read one logical snippet: multiple lines while `(` / `{` depth is non-zero (outside strings
/// and comments). Single-line statements run as soon as delimiters balance.
fn read_repl_snippet() -> Result<ReplInput, std::io::Error> {
    use std::io::{self, Write};

    let mut buffer = String::new();
    loop {
        if buffer.is_empty() {
            print!("raven> ");
        } else {
            print!("...> ");
        }
        io::stdout().flush()?;

        let mut line = String::new();
        let n = io::stdin().read_line(&mut line)?;
        if n == 0 {
            if buffer.is_empty() {
                return Ok(ReplInput::Eof);
            }
            return Ok(ReplInput::Code(buffer));
        }

        let trimmed = line.trim();
        if buffer.is_empty() {
            if trimmed == "exit" || trimmed == "quit" {
                return Ok(ReplInput::Quit);
            }
            if trimmed == "help" {
                return Ok(ReplInput::Help);
            }
            if trimmed.is_empty() {
                continue;
            }
        }

        buffer.push_str(&line);
        let (p, b, bk) = delimiter_depth(&buffer);
        if p > 0 || b > 0 || bk > 0 {
            continue;
        }
        return Ok(ReplInput::Code(buffer));
    }
}

fn start_repl(verbose: bool) {
    println!("🐦 Welcome to Raven REPL!");
    println!("Type 'exit' or 'quit' to exit, 'help' for help");
    println!("Multi-line: keep typing until `()`, `{{}}`, and `[]` match (prompt changes to ...>)");
    println!("─────────────────────────────────────────");

    let mut interpreter = Interpreter::new();
    let mut type_checker = TypeChecker::new();

    loop {
        match read_repl_snippet() {
            Ok(ReplInput::Eof) => {
                println!("Goodbye!");
                break;
            }
            Ok(ReplInput::Quit) => {
                println!("Goodbye!");
                break;
            }
            Ok(ReplInput::Help) => {
                println!("Available commands:");
                println!("  exit, quit - Exit the REPL");
                println!("  help - Show this help message");
                println!("  Any Raven code - Execute the code (multi-line until delimiters match)");
            }
            Ok(ReplInput::Code(input)) => {
                match process_repl_input(&input, &mut interpreter, &mut type_checker, verbose) {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("❌ Error: {}", e);
                    }
                }
            }
            Err(error) => {
                eprintln!("❌ Error reading input: {}", error);
                break;
            }
        }
    }
}

fn process_repl_input(
    input: &str,
    interpreter: &mut Interpreter,
    type_checker: &mut TypeChecker,
    verbose: bool,
) -> Result<(), String> {
    let lexer = Lexer::new(input.to_string());

    if verbose {
        println!("🔍 Input: {}", input);
    }

    let mut parser = Parser::new(lexer, input.to_string());
    let ast = parser
        .parse()
        .map_err(|e| e.with_filename("<repl>".to_string()).format())?;

    if verbose {
        println!("🌳 AST: {:?}", ast);
    }

    type_checker.check(&ast)?;

    if verbose {
        println!("✅ Type check passed");
    }

    match interpreter.execute(&ast) {
        Ok(value) => match value {
            raven::code_gen::Value::Void => {}
            _ => println!("{}", value),
        },
        Err(e) => return Err(e),
    }

    Ok(())
}
