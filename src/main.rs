use clap::{Arg, Command};
use raven::code_gen::Interpreter;
use raven::lexer::Lexer;
use raven::parser::Parser;
use raven::type_checker::TypeChecker;
use raven::error::RavenError;
use raven::span::Span;
use std::fs;
use std::process;

fn main() {
    let matches = Command::new("Raven Programming Language")
        .version("0.1.0")
        .author("martian56 <https://github.com/martian56>")
        .about("Raven compiler and interpreter - fast, safe, and expressive")
        .arg(
            Arg::new("file")
                .short('f')
                .long("file")
                .value_name("FILE")
                .help("The Raven source file to compile/run")
                .required(false)
                .num_args(1),
        )
        .arg(
            Arg::new("repl")
                .long("repl")
                .help("Start interactive REPL mode")
                .action(clap::ArgAction::SetTrue),
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
    let repl_mode = matches.get_flag("repl");

    // Check if we should start REPL mode
    if repl_mode {
        start_repl(verbose);
        return;
    }

    // Get file path (required if not in REPL mode)
    let file_name = matches.get_one::<String>("file").unwrap_or_else(|| {
        eprintln!("❌ Error: Either --file or --repl must be specified");
        eprintln!("Use --help for more information");
        process::exit(1);
    });

    // Read source code
    let source_code = fs::read_to_string(file_name).unwrap_or_else(|err| {
        eprintln!("❌ Failed to read file '{}': {}", file_name, err);
        process::exit(1);
    });

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

    let mut parser = Parser::new(lexer, source_code.clone());
    let ast = match parser.parse() {
        Ok(ast) => {
            if verbose {
                println!("   ✅ Parsing successful!");
            }
            ast
        }
        Err(e) => {
            // Use our beautiful error formatting!
            let error_with_file = e.with_filename(file_name.clone());
            eprint!("{}", error_with_file.format());
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

fn start_repl(verbose: bool) {
    use std::io::{self, Write};
    
    println!("🐦 Welcome to Raven REPL!");
    println!("Type 'exit' or 'quit' to exit, 'help' for help");
    println!("─────────────────────────────────────────");
    
    let mut interpreter = Interpreter::new();
    let mut type_checker = TypeChecker::new();
    
    loop {
        print!("raven> ");
        io::stdout().flush().unwrap();
        
        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(_) => {
                let input = input.trim();
                
                // Handle special commands
                match input {
                    "exit" | "quit" => {
                        println!("👋 Goodbye!");
                        break;
                    }
                    "help" => {
                        println!("Available commands:");
                        println!("  exit/quit - Exit the REPL");
                        println!("  help      - Show this help");
                        println!("  clear     - Clear the interpreter state");
                        println!("  Any valid Raven expression or statement");
                        continue;
                    }
                    "clear" => {
                        interpreter = Interpreter::new();
                        type_checker = TypeChecker::new();
                        println!("✅ Interpreter state cleared");
                        continue;
                    }
                    "" => continue, // Empty input
                    _ => {} // Process as Raven code
                }
                
                // Process Raven code
                match process_repl_input(input, &mut interpreter, &mut type_checker, verbose) {
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

fn process_repl_input(input: &str, interpreter: &mut Interpreter, type_checker: &mut TypeChecker, verbose: bool) -> Result<(), String> {
    // Create lexer
    let lexer = Lexer::new(input.to_string());
    
    if verbose {
        println!("🔍 Input: {}", input);
    }
    
    // Create parser
    let mut parser = Parser::new(lexer, input.to_string());
    let ast = parser.parse().map_err(|e| e.format())?;
    
    if verbose {
        println!("🌳 AST: {:?}", ast);
    }
    
    // Type check with persistent type checker
    type_checker.check(&ast).map_err(|e| e)?;
    
    if verbose {
        println!("✅ Type check passed");
    }
    
    // Execute
    match interpreter.execute(&ast) {
        Ok(value) => {
            // Only print if there's a meaningful result
            match value {
                raven::code_gen::Value::Void => {} // Don't print void
                _ => println!("{}", value),
            }
        }
        Err(e) => return Err(e),
    }
    
    Ok(())
}
