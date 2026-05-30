import { useEffect, useMemo, useRef } from "react";
import Editor, { type Monaco } from "@monaco-editor/react";
import type { IDisposable, Position, editor, languages } from "monaco-editor";
import type { StudioNamespace } from "../shared/types";
import {
  buildSqlCompletionData,
  resolveSqlContextualCompletions,
  type SqlCompletionEntry,
  type SqlCompletionData,
} from "./sqlCompletionCatalog";

type ExecuteMode = "all" | "selected";

const EMPTY_COMPLETION_DATA = buildSqlCompletionData([]);

interface StudioEditorPanelProps {
  schema: StudioNamespace[];
  sql: string;
  onSqlChange: (value: string) => void;
  onRun: (sql: string, mode: ExecuteMode) => void;
  onSelectedSqlChange?: (value: string) => void;
}

export function StudioEditorPanel({
  schema,
  sql,
  onSqlChange,
  onRun,
  onSelectedSqlChange,
}: StudioEditorPanelProps) {
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const editorListenerRefs = useRef<IDisposable[]>([]);
  const onRunRef = useRef(onRun);
  const sqlRef = useRef(sql);
  const completionProviderRef = useRef<IDisposable | null>(null);
  const completionDataRef = useRef<SqlCompletionData>(EMPTY_COMPLETION_DATA);

  const completionData = useMemo(() => {
    return buildSqlCompletionData(schema);
  }, [schema]);

  useEffect(() => {
    completionDataRef.current = completionData;
  }, [completionData]);

  useEffect(() => {
    onRunRef.current = onRun;
  }, [onRun]);

  useEffect(() => {
    sqlRef.current = sql;
  }, [sql]);

  useEffect(() => {
    return () => {
      completionProviderRef.current?.dispose();
      editorListenerRefs.current.forEach((listener) => listener.dispose());
    };
  }, []);

  const readSelectedSql = () => {
    const instance = editorRef.current;
    const model = instance?.getModel();
    const selection = instance?.getSelection();

    if (!instance || !model || !selection || selection.isEmpty()) {
      return "";
    }

    return model.getValueInRange(selection);
  };

  const syncSelectedSql = () => {
    const nextSelectedSql = readSelectedSql();
    onSelectedSqlChange?.(nextSelectedSql.trim().length > 0 ? nextSelectedSql : "");
  };

  const runSql = (mode: ExecuteMode | "auto" = "auto") => {
    const nextSelectedSql = readSelectedSql();
    const hasSelection = nextSelectedSql.trim().length > 0;
    const resolvedMode: ExecuteMode = mode === "auto"
      ? (hasSelection ? "selected" : "all")
      : mode;
    const nextSql = resolvedMode === "selected" ? nextSelectedSql : sqlRef.current;

    if (!nextSql.trim()) {
      return;
    }

    onRunRef.current(nextSql, resolvedMode);
  };

  const registerCompletionProvider = (monaco: Monaco) => {
    completionProviderRef.current?.dispose();
    completionProviderRef.current = monaco.languages.registerCompletionItemProvider("sql", {
      triggerCharacters: [".", " ", ","],
      provideCompletionItems: (model: editor.ITextModel, position: Position) => {
        const data = completionDataRef.current;
        const wordUntil = model.getWordUntilPosition(position);
        const range = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: wordUntil.startColumn,
          endColumn: wordUntil.endColumn,
        };

        const textUntilPosition = model.getValueInRange({
          startLineNumber: 1,
          startColumn: 1,
          endLineNumber: position.lineNumber,
          endColumn: position.column,
        });
        const prefix = wordUntil.word.toLowerCase();
        const suggestions: languages.CompletionItem[] = [];
        const seen = new Set<string>();
        const aliasToTable: Record<string, string> = {};

        const aliasRegex = /\b(?:from|join)\s+([a-zA-Z_][\w]*)\.([a-zA-Z_][\w]*)(?:\s+(?:as\s+)?([a-zA-Z_][\w]*))?/gi;
        let aliasMatch: RegExpExecArray | null = aliasRegex.exec(textUntilPosition);
        while (aliasMatch) {
          const namespaceName = aliasMatch[1]?.toLowerCase();
          const tableName = aliasMatch[2]?.toLowerCase();
          const alias = aliasMatch[3]?.toLowerCase();
          if (namespaceName && tableName && alias) {
            aliasToTable[alias] = `${namespaceName}.${tableName}`;
          }
          aliasMatch = aliasRegex.exec(textUntilPosition);
        }

        const pushSuggestion = (
          label: string,
          kind: languages.CompletionItemKind,
          detail: string,
          insertText = label,
          sortText?: string,
          insertTextRules?: languages.CompletionItemInsertTextRule,
          matchText = prefix,
        ) => {
          const key = `${kind}-${label}-${insertText}`;
          if (seen.has(key)) {
            return;
          }
          if (matchText && !label.toLowerCase().includes(matchText)) {
            return;
          }
          seen.add(key);
          suggestions.push({ label, kind, detail, insertText, insertTextRules, range, sortText });
        };

        const pushEntry = (entry: SqlCompletionEntry) => {
          const snippetRule = entry.isSnippet
            ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet
            : undefined;
          const kindByCategory: Record<SqlCompletionEntry["category"], languages.CompletionItemKind> = {
            function: monaco.languages.CompletionItemKind.Function,
            keyword: monaco.languages.CompletionItemKind.Keyword,
            operator: monaco.languages.CompletionItemKind.Operator,
            snippet: monaco.languages.CompletionItemKind.Snippet,
            type: monaco.languages.CompletionItemKind.TypeParameter,
          };

          pushSuggestion(
            entry.label,
            kindByCategory[entry.category],
            entry.detail,
            entry.insertText ?? entry.label,
            entry.sortText,
            snippetRule,
          );
        };

        const contextualCompletion = resolveSqlContextualCompletions(data, textUntilPosition, aliasToTable);
        if (contextualCompletion) {
          const kind = contextualCompletion.kind === "column"
            ? monaco.languages.CompletionItemKind.Field
            : monaco.languages.CompletionItemKind.Class;

          contextualCompletion.labels.forEach((label) =>
            pushSuggestion(label, kind, contextualCompletion.detail, label, undefined, undefined, contextualCompletion.partial),
          );
          return { suggestions };
        }

        data.keywords.forEach((keyword) =>
          pushSuggestion(keyword, monaco.languages.CompletionItemKind.Keyword, "SQL keyword", keyword, `2_${keyword}`),
        );
        data.snippets.forEach(pushEntry);
        data.functions.forEach(pushEntry);
        data.types.forEach(pushEntry);
        data.operators.forEach(pushEntry);
        data.namespaces.forEach((namespaceName) =>
          pushSuggestion(namespaceName, monaco.languages.CompletionItemKind.Module, "Namespace"),
        );
        Object.entries(data.tablesByNamespace).forEach(([namespaceName, tables]) => {
          tables.forEach((table) => {
            pushSuggestion(`${namespaceName}.${table}`, monaco.languages.CompletionItemKind.Class, "Qualified table name");
            pushSuggestion(table, monaco.languages.CompletionItemKind.Class, `Table in ${namespaceName}`);
          });
        });
        Object.entries(data.columnsByTable).forEach(([qualifiedTable, columns]) => {
          columns.forEach((column) => {
            pushSuggestion(column, monaco.languages.CompletionItemKind.Field, `Column (${qualifiedTable})`);
          });
        });

        return { suggestions };
      },
    });
  };

  const handleEditorMount = (instance: editor.IStandaloneCodeEditor, monaco: Monaco) => {
    editorRef.current = instance;
    instance.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => {
      runSql("auto");
    });
    editorListenerRefs.current.forEach((listener) => listener.dispose());
    editorListenerRefs.current = [
      instance.onDidChangeCursorSelection(() => {
        syncSelectedSql();
      }),
      instance.onDidChangeModelContent(() => {
        syncSelectedSql();
      }),
    ];
    syncSelectedSql();
    registerCompletionProvider(monaco);
  };

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden bg-background">
      <div className="min-h-0 flex-1 overflow-hidden">
        <Editor
          height="100%"
          defaultLanguage="sql"
          theme="vs-dark"
          value={sql}
          onChange={(value) => onSqlChange(value ?? "")}
          onMount={handleEditorMount}
          options={{
            minimap: { enabled: false },
            fontSize: 13,
            lineNumbers: "on",
            lineNumbersMinChars: 3,
            automaticLayout: true,
            wordWrap: "on",
            scrollBeyondLastLine: false,
            padding: { top: 12 },
            fontFamily: "JetBrains Mono, monospace",
          }}
        />
      </div>
    </div>
  );
}
