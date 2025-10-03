use clap::{Arg, Command};
use std::fs;
use std::process;
use raven::lexer::{Lexer, TokenType};
use raven::parser::Parser;

fn main() {
    // Define the CLI using clap
    let matches = Command::new("Raven Compiler")
        .version("1.0")
        .author("Your Name <your_email@example.com>")
        .about("Lexes and parses Raven code from a file")
        .arg(
            Arg::new("file")
                .short('f')
                .long("file")
                .value_name("FILE")
                .help("The Raven source file to lex and parse")
                .required(true)
                .num_args(1),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .help("Print more detailed information during lexing and parsing")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

    // Get the file path from arguments
    let file_name = matches.get_one::<String>("file").expect("File argument is required");

    // Read file contents
    let source_code = fs::read_to_string(file_name)
        .unwrap_or_else(|err| {
            eprintln!("Failed to read file '{}': {}", file_name, err);
            process::exit(1);
        });

    if matches.contains_id("verbose") {
        println!("🔍 Lexing and Parsing file: {}", file_name);
    }

    // Initialize the lexer
    let mut lexer = Lexer::new(source_code.clone());
    let mut parser = Parser::new(lexer.clone(), source_code);
    
    let mut token_count = 0;

    // Lex the source code, but only for parsing
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token();
        token_count += 1;

        // Print the token with verbose output if needed
        if matches.contains_id("verbose") {
            println!("Token {}: {:?}", token_count, token);
        }

        tokens.push(token.clone());

        if token == TokenType::EOF {
            println!("\n✅ Finished lexing {} tokens.", token_count);
            break;
        }
    }

    // Initialize the parser with the lexer, not the tokens

    // Parse the tokens and build the AST
    match parser.parse() {
        Ok(ast) => {
            // Print the parsed AST
            if matches.contains_id("verbose") {
                println!("\n📜 Parsed AST:");
                println!("{:?}", ast);
            } else {
                println!("\n✅ AST parsing completed successfully!");
            }
        }
        Err(e) => {
            eprintln!("Error during parsing: {}", e);
            process::exit(1);
        }
    }
}
