import * as vscode from "vscode";
import * as os from "os";

import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  Executable,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

// Activates the Codetypo VSCode extension
export async function activate(
  context: vscode.ExtensionContext,
): Promise<void> {
  let name = "Codetypo";

  const outputChannel = vscode.window.createOutputChannel(name);

  // context.subscriptions holds the disposables we want called
  // when the extension is deactivated
  context.subscriptions.push(outputChannel);

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration(
      async (e: vscode.ConfigurationChangeEvent) => {
        const restartTriggeredBy = [
          "codetypo.config",
          "codetypo.diagnosticSeverity",
          "codetypo.logLevel",
          "codetypo.path",
        ].find((s) => e.affectsConfiguration(s));

        if (restartTriggeredBy) {
          await vscode.commands.executeCommand("codetypo.restart");
        }
      },
    ),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("codetypo.restart", async () => {
      // can't stop if the client has previously failed to start
      if (client && client.needsStop()) {
        await client.stop();
      }

      try {
        client = await createClient(context, name, outputChannel);
      } catch (err) {
        vscode.window.showErrorMessage(
          `${err instanceof Error ? err.message : err}`,
        );
        return;
      }

      // Start the client. This will also launch the server
      await client.start();
    }),
  );

  // use the command as our single entry point for (re)starting
  // the client and server. This ensures at activation time we
  // start and handle errors in a way that's consistent with the
  // other triggers
  await vscode.commands.executeCommand("codetypo.restart");
}

// Creates and configures the language client
async function createClient(
  context: vscode.ExtensionContext,
  name: string,
  outputChannel: vscode.OutputChannel,
): Promise<LanguageClient> {
  const env = { ...process.env };

  let config = vscode.workspace.getConfiguration("codetypo");
  let path = await getServerPath(context, config);

  outputChannel.appendLine("Using codetypo server " + path);

  env.RUST_LOG = config.get("logLevel");

  const run: Executable = {
    command: path,
    options: { env: env },
  };

  const serverOptions: ServerOptions = {
    run: run,
    // used when launched in debug mode
    debug: run,
  };

  const clientOptions: LanguageClientOptions = {
    // Register the server for all documents
    documentSelector: [
      { scheme: "untitled" },
      { scheme: "file", pattern: "**" },
      // source control commit message
      { scheme: "vscode-scm" },
    ],
    outputChannel: outputChannel,
    traceOutputChannel: outputChannel,
    initializationOptions: {
      config: config.get("config") ? config.get("config") : null,
      diagnosticSeverity: config.get("diagnosticSeverity"),
    },
  };

  return new LanguageClient(
    name.toLowerCase(),
    name,
    serverOptions,
    clientOptions,
  );
}

// Resolves the server path for codetypo-lsp (env, config, or bundled)
async function getServerPath(
  context: vscode.ExtensionContext,
  config: vscode.WorkspaceConfiguration,
): Promise<string> {
  let path = process.env.CODETYPO_LSP_PATH ?? config.get<null | string>("path");

  if (path) {
    if (path.startsWith("~/")) {
      path = os.homedir() + path.slice("~".length);
    }
    const pathUri = vscode.Uri.file(path);

    return await vscode.workspace.fs.stat(pathUri).then(
      () => pathUri.fsPath,
      () => {
        throw new Error(
          `${path} does not exist. Please check codetypo.path in Settings.`,
        );
      },
    );
  }

  //if (config.package.releaseTag === null) return "codetypo-lsp";

  const ext = process.platform === "win32" ? ".exe" : "";
  const bundled = vscode.Uri.joinPath(
    context.extensionUri,
    "bundled",
    `codetypo-lsp${ext}`,
  );

  return await vscode.workspace.fs.stat(bundled).then(
    () => bundled.fsPath,
    () => {
      throw new Error(
        "Unfortunately we don't ship binaries for your platform yet. " +
          "Try specifying codetypo.path in Settings. " +
          "Or raise an issue [here](https://github.com/khulnasoft/codetypo-lsp/issues) " +
          "to request a binary for your platform.",
      );
    },
  );
}

// Deactivates the extension and stops the client
export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
