use clap::{Arg, Command};
use raven::code_gen::Interpreter;
use raven::lexer::Lexer;
use raven::parser::Parser;
use raven::type_checker::TypeChecker;
use std::fs;
use std::process;

fn main() {
    let matches = Command::new("Raven Programming Language")
        .version("0.1.0")
        .author("martian58 <https://github.com/martian58>")
        .about("Raven compiler and interpreter - fast, safe, and expressive")
        .arg(
            Arg::new("file")
                .short('f')
                .long("file")
                .value_name("FILE")
                .help("The Raven source file to compile/run")
                .required(true)
                .num_args(1),
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

    // Get file path
    let file_name = matches
        .get_one::<String>("file")
        .expect("File argument is required");

    // Read source code
    let source_code = fs::read_to_string(file_name).unwrap_or_else(|err| {
        eprintln!("❌ Failed to read file '{}': {}", file_name, err);
        process::exit(1);
    });

    let verbose = matches.get_flag("verbose");
    let check_only = matches.get_flag("check");
    let show_ast = matches.get_flag("ast");

    if verbose {
        println!("📁 Reading file: {}", file_name);
        println!("─────────────────────────────────────────");
    }

    // === LEXING ===
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

    // === PARSING ===
    if verbose {
        println!("\n🌳 PARSING...");
    }

    let mut parser = Parser::new(lexer);
    let ast = match parser.parse() {
        Ok(ast) => {
            if verbose {
                println!("   ✅ Parsing successful!");
            }
            ast
        }
        Err(e) => {
            eprintln!("❌ Parse error: {}", e);
            process::exit(1);
        }
    };

    // Show AST if requested
    if show_ast || verbose {
        println!("\n📜 Abstract Syntax Tree:");
        println!("{:#?}", ast);
    }

    // === TYPE CHECKING ===
    if verbose {
        println!("\n🔎 TYPE CHECKING...");
    }

    let mut type_checker = TypeChecker::new();
    match type_checker.check(&ast) {
        Ok(_) => {
            if verbose {
                println!("   ✅ Type checking passed!");
            }
        }
        Err(e) => {
            eprintln!("❌ Type error: {}", e);
            process::exit(1);
        }
    }

    // If only checking, exit here
    if check_only {
        println!("✅ All checks passed!");
        return;
    }

    // === EXECUTION ===
    if verbose {
        println!("\n🚀 EXECUTING...");
        println!("─────────────────────────────────────────");
    } else {
        println!("🚀 Running Raven program...\n");
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
