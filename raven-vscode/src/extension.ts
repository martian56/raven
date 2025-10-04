import * as vscode from 'vscode';
import * as path from 'path';
import { exec } from 'child_process';

export function activate(context: vscode.ExtensionContext) {
    console.log('Raven Language Extension is now active!');

    // Register command to run Raven files
    let runFileCommand = vscode.commands.registerCommand('raven.runFile', (uri: vscode.Uri) => {
        if (uri) {
            runRavenFile(uri.fsPath);
        } else {
            // If no URI provided, try to run the currently active file
            const activeEditor = vscode.window.activeTextEditor;
            if (activeEditor && activeEditor.document.languageId === 'raven') {
                runRavenFile(activeEditor.document.fileName);
            } else {
                vscode.window.showErrorMessage('No Raven file is currently open.');
            }
        }
    });

    context.subscriptions.push(runFileCommand);

    // Register hover provider for built-in functions
    let hoverProvider = vscode.languages.registerHoverProvider('raven', {
        provideHover(document, position, token) {
            const range = document.getWordRangeAtPosition(position);
            const word = document.getText(range);
            
            const builtinFunctions: { [key: string]: string } = {
                'print': 'Prints output to console. Usage: `print(message)`',
                'input': 'Gets user input. Usage: `let name: String = input("Enter name: ")`',
                'format': 'Formats string with placeholders. Usage: `format("Hello {}", name)`',
                'len': 'Gets length of string or array. Usage: `len(text)` or `len(array)`',
                'type': 'Gets type information. Usage: `type(variable)`',
                'read_file': 'Reads file contents. Usage: `let content: String = read_file("file.txt")`',
                'write_file': 'Writes to file. Usage: `write_file("file.txt", content)`',
                'append_file': 'Appends to file. Usage: `append_file("file.txt", content)`',
                'file_exists': 'Checks if file exists. Usage: `file_exists("file.txt")`',
                'enum_from_string': 'Converts string to enum. Usage: `enum_from_string("EnumName", "VariantName")`'
            };

            if (builtinFunctions[word]) {
                return new vscode.Hover(builtinFunctions[word]);
            }

            return null;
        }
    });

    context.subscriptions.push(hoverProvider);

    // Register completion provider for built-in functions
    let completionProvider = vscode.languages.registerCompletionItemProvider('raven', {
        provideCompletionItems(document, position, token, context) {
            const builtinFunctions = [
                'print', 'input', 'format', 'len', 'type',
                'read_file', 'write_file', 'append_file', 'file_exists', 'enum_from_string'
            ];

            const keywords = [
                'let', 'fun', 'if', 'else', 'while', 'for', 'return',
                'import', 'export', 'struct', 'enum', 'true', 'false', 'void'
            ];

            const types = [
                'int', 'float', 'bool', 'String', 'int[]', 'float[]', 'bool[]', 'String[]'
            ];

            const completions: vscode.CompletionItem[] = [];

            // Add built-in functions
            builtinFunctions.forEach(func => {
                const item = new vscode.CompletionItem(func, vscode.CompletionItemKind.Function);
                item.detail = 'Built-in function';
                completions.push(item);
            });

            // Add keywords
            keywords.forEach(keyword => {
                const item = new vscode.CompletionItem(keyword, vscode.CompletionItemKind.Keyword);
                item.detail = 'Keyword';
                completions.push(item);
            });

            // Add types
            types.forEach(type => {
                const item = new vscode.CompletionItem(type, vscode.CompletionItemKind.TypeParameter);
                item.detail = 'Type';
                completions.push(item);
            });

            return completions;
        }
    });

    context.subscriptions.push(completionProvider);
}

function runRavenFile(filePath: string) {
    const terminal = vscode.window.createTerminal('Raven');
    terminal.sendText(`raven "${filePath}"`);
    terminal.show();
}

export function deactivate() {
    console.log('Raven Language Extension is now deactivated.');
}
