import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  Trace,
  TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;
let outputChannel: vscode.OutputChannel | undefined;

function resolveServerCommand(context: vscode.ExtensionContext): string {
  const configured = vscode.workspace
    .getConfiguration('huzi')
    .get<string>('server.path', '');
  if (configured && configured.trim().length > 0) {
    return configured.trim();
  }
  const binName = process.platform === 'win32' ? 'huzi-lsp.exe' : 'huzi-lsp';
  const bundled = path.join(context.extensionPath, 'server', binName);
  if (fs.existsSync(bundled)) {
    return bundled;
  }
  // No bundled binary and no user override: rely on PATH resolution.
  // Users can point `huzi.server.path` at a cargo-built binary instead.
  return binName;
}

function resolveTrace(): Trace {
  const configured = vscode.workspace
    .getConfiguration('huzi')
    .get<string>('trace.server', 'off');
  switch ((configured ?? 'off').toLowerCase()) {
    case 'messages':
      return Trace.Messages;
    case 'verbose':
      return Trace.Verbose;
    case 'off':
    default:
      return Trace.Off;
  }
}

async function startClient(context: vscode.ExtensionContext): Promise<void> {
  const serverCommand = resolveServerCommand(context);
  const maxRestartCount = vscode.workspace
    .getConfiguration('huzi')
    .get<number>('maxRestartCount', 5);

  const serverOptions: ServerOptions = {
    run: { command: serverCommand, transport: TransportKind.stdio },
    debug: { command: serverCommand, transport: TransportKind.stdio },
  };

  if (!outputChannel) {
    outputChannel = vscode.window.createOutputChannel('Huzi LSP');
  }

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'huzi' }],
    outputChannel,
    traceOutputChannel: outputChannel,
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher('**/*.hz'),
    },
  };

  client = new LanguageClient(
    'huzi',
    'Huzi Language Server',
    serverOptions,
    clientOptions,
  );
  client.setTrace(resolveTrace());
  outputChannel.appendLine(
    `[huzi] starting server: ${serverCommand} (maxRestartCount=${maxRestartCount})`,
  );
  await client.start();
}

async function restartServer(context: vscode.ExtensionContext): Promise<void> {
  try {
    await client?.stop();
  } catch (err) {
    outputChannel?.appendLine(`[huzi] stop failed: ${String(err)}`);
  } finally {
    client = undefined;
  }
  await startClient(context);
}

function resolveHuzcCommand(): string {
  const configured = vscode.workspace
    .getConfiguration('huzi')
    .get<string>('huzc.path', 'huzc');
  if (configured && configured.trim().length > 0) {
    return configured.trim();
  }
  return 'huzc';
}

function runHuziFile(): void {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.languageId !== 'huzi') {
    vscode.window.showWarningMessage('Huzi: open a .hz file first.');
    return;
  }
  const filePath = editor.document.fileName;
  const huzc = resolveHuzcCommand();
  const quotedHuzc = `"${huzc}"`;
  const quotedFile = `"${filePath}"`;
  // 输出与源文件同目录、同名（跟 huzc 缺省规则一致，但显式传 -o，
  // 不依赖编译器默认值，且用绝对路径避免终端 cwd 影响）。
  const parsed = path.parse(filePath);
  const outBase = path.join(parsed.dir, parsed.name);
  const quotedOut = `"${outBase}"`;
  const exePath =
    process.platform === 'win32' ? `${outBase}.exe` : outBase;
  const quotedExe = `"${exePath}"`;
  const cmd = `${quotedHuzc} --input ${quotedFile} -o ${quotedOut} && ${quotedExe}`;
  let terminal =
    vscode.window.terminals.find((t) => t.name === 'Huzi Run') ??
    vscode.window.createTerminal('Huzi Run');
  terminal.show(true);
  terminal.sendText(cmd, true);
}

export async function activate(
  context: vscode.ExtensionContext,
): Promise<void> {
  await startClient(context);
  context.subscriptions.push(
    vscode.commands.registerCommand('huzi.restartServer', () =>
      restartServer(context),
    ),
    vscode.commands.registerCommand('huzi.runFile', () => runHuziFile()),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('huzi.trace.server') && client) {
        void client.setTrace(resolveTrace());
      }
    }),
  );
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
