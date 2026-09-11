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
import type { ProcedureMetadata } from "@/features/functions/types";
import {
  completionsForCallContext,
  hoverForIdentifier,
  parseCallCompletionContext,
  readQualifiedIdentifier,
  signatureHelpForCall,
} from "@/features/functions/sqlCompletions";

type ExecuteMode = "all" | "selected";

const EMPTY_COMPLETION_DATA = buildSqlCompletionData([]);

interface StudioEditorPanelProps {
  schema: StudioNamespace[];
  procedures?: ProcedureMetadata[];
  sql: string;
  onSqlChange: (value: string) => void;
  onRun: (sql: string, mode: ExecuteMode) => void;
  onSelectedSqlChange?: (value: string) => void;
}

export function StudioEditorPanel({
  schema,
  procedures = [],
  sql,
  onSqlChange,
  onRun,
  onSelectedSqlChange,
}: StudioEditorPanelProps) {
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const editorListenerRefs = useRef<IDisposable[]>([]);
  const onRunRef = useRef(onRun);
  const sqlRef = useRef(sql);
  const languageProviderRefs = useRef<IDisposable[]>([]);
  const completionDataRef = useRef<SqlCompletionData>(EMPTY_COMPLETION_DATA);

  const completionData = useMemo(() => {
    return buildSqlCompletionData(schema, procedures);
  }, [schema, procedures]);

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
      languageProviderRefs.current.forEach((listener) => listener.dispose());
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

  const registerLanguageProviders = (monaco: Monaco) => {
    languageProviderRefs.current.forEach((provider) => provider.dispose());
    languageProviderRefs.current = [
      monaco.languages.registerCompletionItemProvider("sql", {
      triggerCharacters: [".", " ", ",", "("],
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
        const callContext = parseCallCompletionContext(textUntilPosition);

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
          suggestionRange = range,
        ) => {
          const key = `${kind}-${label}-${insertText}`;
          if (seen.has(key)) {
            return;
          }
          if (matchText && !label.toLowerCase().includes(matchText)) {
            return;
          }
          seen.add(key);
          suggestions.push({
            label,
            kind,
            detail,
            insertText,
            insertTextRules,
            range: suggestionRange,
            sortText,
          });
        };

        const pushEntry = (
          entry: SqlCompletionEntry,
          matchText = prefix,
          suggestionRange = range,
        ) => {
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
            matchText,
            suggestionRange,
          );
        };

        if (callContext) {
          const callRange = callContext.kind === "name"
            ? {
                startLineNumber: position.lineNumber,
                endLineNumber: position.lineNumber,
                startColumn: Math.max(1, position.column - (textUntilPosition.length - callContext.replaceFrom)),
                endColumn: position.column,
              }
            : range;
          completionsForCallContext(callContext, data.procedures).forEach((entry) =>
            pushEntry(entry, callContext.kind === "name" ? callContext.partial : "", callRange),
          );
          if (callContext.kind === "arguments" || callContext.partial.length > 0) {
            return { suggestions };
          }
        }

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
        data.snippets.forEach((entry) => pushEntry(entry));
        data.functions.forEach((entry) => pushEntry(entry));
        data.procedureEntries.forEach((entry) => pushEntry(entry));
        data.types.forEach((entry) => pushEntry(entry));
        data.operators.forEach((entry) => pushEntry(entry));
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
    }),
      monaco.languages.registerSignatureHelpProvider("sql", {
        signatureHelpTriggerCharacters: ["(", ",", " "],
        signatureHelpRetriggerCharacters: [",", " "],
        provideSignatureHelp: (model: editor.ITextModel, position: Position) => {
          const textUntilPosition = model.getValueInRange({
            startLineNumber: 1,
            startColumn: 1,
            endLineNumber: position.lineNumber,
            endColumn: position.column,
          });
          const context = parseCallCompletionContext(textUntilPosition);
          if (!context || context.kind !== "arguments") {
            return { value: { signatures: [], activeSignature: 0, activeParameter: 0 }, dispose: () => {} };
          }
          const help = signatureHelpForCall(context, completionDataRef.current.procedures);
          if (!help) {
            return { value: { signatures: [], activeSignature: 0, activeParameter: 0 }, dispose: () => {} };
          }
          return {
            value: {
              signatures: [
                {
                  label: help.label,
                  documentation: help.documentation,
                  parameters: help.parameters.map((parameter) => ({
                    label: parameter.label,
                    documentation: parameter.documentation,
                  })),
                },
              ],
              activeSignature: 0,
              activeParameter: help.activeParameter,
            },
            dispose: () => {},
          };
        },
      }),
      monaco.languages.registerHoverProvider("sql", {
        provideHover: (model: editor.ITextModel, position: Position) => {
          const offset = model.getOffsetAt(position);
          const identifier = readQualifiedIdentifier(model.getValue(), offset);
          const documentation = hoverForIdentifier(identifier, completionDataRef.current.procedures);
          if (!documentation) {
            return null;
          }
          return {
            contents: [{ value: documentation }],
          };
        },
      }),
    ];
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
    try {
      registerLanguageProviders(monaco);
    } catch (error) {
      console.warn("Failed to register SQL language providers", error);
    }
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
