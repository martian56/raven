import * as vscode from 'vscode';
import * as path from 'path';
import { exec } from 'child_process';

export function activate(context: vscode.ExtensionContext) {
    console.log('Raven Language Extension is now active!');

    let runFileCommand = vscode.commands.registerCommand('raven.runFile', (uri: vscode.Uri) => {
        if (uri) {
            runRavenFile(uri.fsPath);
        } else {
            const activeEditor = vscode.window.activeTextEditor;
            if (activeEditor && activeEditor.document.languageId === 'raven') {
                runRavenFile(activeEditor.document.fileName);
            } else {
                vscode.window.showErrorMessage('No Raven file is currently open.');
            }
        }
    });

    context.subscriptions.push(runFileCommand);

    let hoverProvider = vscode.languages.registerHoverProvider('raven', {
        provideHover(document, position, token) {
            const range = document.getWordRangeAtPosition(position);
            const word = document.getText(range);
            
            const builtinFunctions: { [key: string]: string } = {
                'print': 'Prints a String followed by a newline. Usage: `print(message)`',
                'print_int': 'Prints an Int followed by a newline. Usage: `print_int(n)`',
                'println': 'Prints a String with a trailing newline (from `std/io`). Usage: `import std/io { println }`',
                'panic': 'Aborts the program with a message (from `std/test`).'
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
                'print', 'print_int', 'println', 'panic'
            ];

            const keywords = [
                'let', 'const', 'fun', 'return', 'if', 'else', 'while', 'for',
                'loop', 'in', 'break', 'continue', 'match', 'struct', 'enum',
                'trait', 'impl', 'import', 'as', 'extern', 'defer', 'dyn',
                'true', 'false', 'self', 'Self'
            ];

            const types = [
                'Int', 'Float', 'Bool', 'String', 'Char', 'Unit',
                'Option', 'Result', 'List', 'Map', 'Set',
                'CInt', 'CLong', 'CSize', 'CStr', 'CPtr', 'CDouble'
            ];

            const completions: vscode.CompletionItem[] = [];

            builtinFunctions.forEach(func => {
                const item = new vscode.CompletionItem(func, vscode.CompletionItemKind.Function);
                item.detail = 'Built-in function';
                completions.push(item);
            });

            keywords.forEach(keyword => {
                const item = new vscode.CompletionItem(keyword, vscode.CompletionItemKind.Keyword);
                item.detail = 'Keyword';
                completions.push(item);
            });

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
