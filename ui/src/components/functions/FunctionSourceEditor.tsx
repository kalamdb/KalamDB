import Editor, { type Monaco } from "@monaco-editor/react";
import { cn } from "@/lib/utils";

const MAX_HEIGHT_PX = 384;
const MIN_HEIGHT_PX = 128;
const LINE_HEIGHT_PX = 20;
const EDITOR_CHROME_PX = 24;

function editorHeightForSource(source: string): number {
  const lineCount = Math.max(1, source.split("\n").length);
  return Math.min(MAX_HEIGHT_PX, Math.max(MIN_HEIGHT_PX, lineCount * LINE_HEIGHT_PX + EDITOR_CHROME_PX));
}

function disableLanguageDiagnostics(monaco: Monaco) {
  monaco.languages.typescript.typescriptDefaults.setDiagnosticsOptions({
    noSemanticValidation: true,
    noSyntaxValidation: true,
    noSuggestionDiagnostics: true,
  });
  monaco.languages.typescript.javascriptDefaults.setDiagnosticsOptions({
    noSemanticValidation: true,
    noSyntaxValidation: true,
    noSuggestionDiagnostics: true,
  });
}

function applySourceTheme(monaco: Monaco) {
  monaco.editor.defineTheme("kalam-inline-source", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#0b0a09",
      "scrollbarSlider.background": "#ffffff4d",
      "scrollbarSlider.hoverBackground": "#ffffff73",
      "scrollbarSlider.activeBackground": "#ffffff99",
    },
  });
}

function prepareSourceEditor(monaco: Monaco) {
  disableLanguageDiagnostics(monaco);
  applySourceTheme(monaco);
}

export function FunctionSourceEditor({
  source,
  className,
}: {
  source: string;
  className?: string;
}) {
  const height = editorHeightForSource(source);

  return (
    <div
      data-testid="function-inline-source-editor"
      className={cn("overflow-hidden rounded-md border border-slate-700 bg-black", className)}
      style={{ height }}
    >
      <Editor
        height={height}
        language="typescript"
        theme="kalam-inline-source"
        value={source}
        loading={(
          <pre className="h-full overflow-auto p-3 font-mono text-xs leading-5 text-slate-200">
            {source}
          </pre>
        )}
        beforeMount={prepareSourceEditor}
        options={{
          readOnly: true,
          domReadOnly: true,
          minimap: { enabled: false },
          fontSize: 13,
          lineNumbers: "on",
          lineNumbersMinChars: 3,
          glyphMargin: false,
          folding: true,
          scrollBeyondLastLine: false,
          automaticLayout: true,
          wordWrap: "off",
          padding: { top: 12, bottom: 20 },
          fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
          overviewRulerBorder: false,
          renderLineHighlight: "none",
          selectionHighlight: false,
          occurrencesHighlight: "off",
          contextmenu: false,
          links: false,
          scrollbar: {
            vertical: "visible",
            horizontal: "visible",
            verticalScrollbarSize: 12,
            horizontalScrollbarSize: 12,
            alwaysConsumeMouseWheel: false,
            useShadows: true,
          },
        }}
      />
    </div>
  );
}
