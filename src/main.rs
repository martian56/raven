use clap::{Arg, Command};
use std::fs;
use std::process;
use raven::lexer::{Lexer, TokenType};

fn main() {
    // Define the CLI using clap
    let matches = Command::new("Raven Lexer")
        .version("1.0")
        .author("Your Name <your_email@example.com>")
        .about("Lexes Raven code from a file")
        .arg(
            Arg::new("file")
                .short('f')
                .long("file")
                .value_name("FILE")
                .help("The Raven source file to lex")
                .required(true)
                .num_args(1),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .help("Print more detailed information during lexing")
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
        println!("🔍 Lexing file: {}", file_name);
    }

    let mut lexer = Lexer::new(source_code);

    let mut token_count = 0;

    loop {
        let token = lexer.next_token();
        token_count += 1;

        // Print the token with verbose output if needed
        if matches.contains_id("verbose") {
            println!("Token {}: {:?}", token_count, token);
        } else {
            print!("{:?} ", token);
        }

        if token == TokenType::EOF {
            println!("\n✅ Finished lexing {} tokens.", token_count);
            break;
        }
    }
}
