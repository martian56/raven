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
                'print': 'Prints any `ToString` value followed by a newline. Usage: `print(value)`',
                'println': 'Prints a String with a trailing newline (from `std/io`). Usage: `import std/io { println }`',
                'panic': 'Aborts the program with a message (from `std/test`).',
                'type_name': 'Compile-time reflection: `type_name<T>()` returns the name of `T` as a `String`.',
                'field_names': 'Compile-time reflection: `field_names<T>()` returns a struct\'s field names in declaration order.',
                'to_any': 'Runtime reflection: `to_any<T>(value)` boxes a value into `Any`.',
                'type_name_of': 'Runtime reflection: `type_name_of(a: Any)` returns the runtime type name as a `String`.',
                'field_names_of': 'Runtime reflection: `field_names_of(a: Any)` returns the boxed value\'s field names.',
                'get_field': 'Runtime reflection: `get_field(a: Any, name)` reads a field by name, returning `Option<Any>`.',
                'cast': 'Runtime reflection: `cast<T>(a: Any)` downcasts to `T`, returning `Option<T>`.',
                'channel': 'Concurrency (`std/sync`): `channel()` creates an unbuffered `Channel`. Import with `import std/sync { channel }`.',
                'channel_buffered': 'Concurrency (`std/sync`): `channel_buffered(cap)` creates a buffered `Channel`.',
                'send': 'Concurrency (`std/sync`): `ch.send(value)` sends a value, blocking until accepted.',
                'recv': 'Concurrency (`std/sync`): `ch.recv()` receives a value, blocking until available.',
                'yield_now': 'Concurrency (`std/sync`): `yield_now()` yields to the scheduler so other goroutines run.',
                'to_cstr': 'FFI (`std/ffi`): `to_cstr(s: String)` converts a String to a `CStr`.',
                'from_cstr': 'FFI (`std/ffi`): `from_cstr(p: CStr)` reads a `CStr` back into a `String`.',
                'load': 'FFI (`std/ffi`): `load<T>(p)` reads a `T` through a raw `CPtr<T>`.',
                'store': 'FFI (`std/ffi`): `store<T>(p, value)` writes a `T` through a raw `CPtr<T>`.',
                'offset': 'FFI (`std/ffi`): `offset<T>(p, count)` advances a pointer by `count` elements.',
                'is_null': 'FFI (`std/ffi`): `is_null<T>(p)` reports whether a pointer is null.',
                'null_ptr': 'FFI (`std/ffi`): `null_ptr<T>()` returns a null `CPtr<T>`.',
                'alloc': 'FFI (`std/ffi`): `alloc<T>(count)` allocates an unmanaged buffer; pair with `free`.',
                'free': 'FFI (`std/ffi`): `free<T>(p)` frees a buffer obtained from `alloc`.'
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
            const builtinFunctions: { [name: string]: string } = {
                'print': 'Built-in function',
                'println': 'std/io',
                'panic': 'std/test',
                'type_name': 'Compile-time reflection: type_name<T>()',
                'field_names': 'Compile-time reflection: field_names<T>()',
                'to_any': 'Runtime reflection: to_any<T>(value) -> Any',
                'type_name_of': 'Runtime reflection: type_name_of(a: Any) -> String',
                'field_names_of': 'Runtime reflection: field_names_of(a: Any)',
                'get_field': 'Runtime reflection: get_field(a: Any, name) -> Option<Any>',
                'cast': 'Runtime reflection: cast<T>(a: Any) -> Option<T>',
                'channel': 'Concurrency (std/sync): channel() -> Channel',
                'channel_buffered': 'Concurrency (std/sync): channel_buffered(cap) -> Channel',
                'yield_now': 'Concurrency (std/sync): yield_now()',
                'to_cstr': 'FFI (std/ffi): to_cstr(s: String) -> CStr',
                'from_cstr': 'FFI (std/ffi): from_cstr(p: CStr) -> String',
                'load': 'FFI (std/ffi): load<T>(p) -> T',
                'store': 'FFI (std/ffi): store<T>(p, value)',
                'offset': 'FFI (std/ffi): offset<T>(p, count) -> CPtr<T>',
                'is_null': 'FFI (std/ffi): is_null<T>(p) -> Bool',
                'null_ptr': 'FFI (std/ffi): null_ptr<T>() -> CPtr<T>',
                'alloc': 'FFI (std/ffi): alloc<T>(count) -> CPtr<T>',
                'free': 'FFI (std/ffi): free<T>(p)'
            };

            const keywords = [
                'let', 'const', 'fun', 'return', 'if', 'else', 'while', 'for',
                'loop', 'in', 'break', 'continue', 'match', 'struct', 'enum',
                'trait', 'impl', 'import', 'as', 'extern', 'defer', 'dyn',
                'spawn', 'macro', 'true', 'false', 'self', 'Self'
            ];

            const types = [
                'Int', 'Float', 'Bool', 'String', 'Char', 'Unit', 'Any',
                'Option', 'Result', 'List', 'Map', 'Set', 'Channel',
                'CInt', 'CLong', 'CSize', 'CStr', 'CPtr', 'CFloat', 'CDouble', 'CFnPtr'
            ];

            const completions: vscode.CompletionItem[] = [];

            Object.keys(builtinFunctions).forEach(func => {
                const item = new vscode.CompletionItem(func, vscode.CompletionItemKind.Function);
                item.detail = builtinFunctions[func];
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
