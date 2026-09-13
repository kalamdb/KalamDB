import type { ReactNode } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Field, FieldError, FieldGroup, FieldLabel, FieldLegend, FieldSet } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { classifyFieldKind } from "@/components/sql-studio-v2/shared/value-validation";
import { formatDeclaredType, formatSqlType } from "@/features/functions/format";
import { defaultValueForType, type TestFieldError, type TestValue } from "@/features/functions/testValues";
import type { ResolvedKalamType } from "@/features/functions/types";
import { cn } from "@/lib/utils";

const NULL_SELECT_VALUE = "__kalam_null__";

function errorForPath(errors: TestFieldError[], path: string): string | undefined {
  return errors.find((error) => error.path === path)?.message;
}

function fieldId(path: string): string {
  return `fn-param-${path.replace(/[^A-Za-z0-9_-]+/g, "-")}`;
}

function TypeHint({ type }: { type: ResolvedKalamType }) {
  return <span className="font-normal text-muted-foreground">{formatDeclaredType(type)}</span>;
}

function jsonText(value: TestValue): string {
  if (typeof value === "string") {
    return value;
  }
  if (value === null || value === undefined) {
    return "";
  }
  return JSON.stringify(value, null, 2);
}

function datetimeLocalValue(value: TestValue): string {
  if (typeof value !== "string" || !value) {
    return "";
  }
  const iso = value.replace(" ", "T").replace(/Z$/, "");
  return iso.slice(0, 16);
}

function ParameterField({
  name,
  type,
  htmlFor,
  error,
  children,
}: {
  name: string;
  type: ResolvedKalamType;
  htmlFor?: string;
  error?: string;
  children: ReactNode;
}) {
  return (
    <Field data-invalid={error ? true : undefined}>
      <FieldLabel htmlFor={htmlFor}>
        {name}
        <TypeHint type={type} />
      </FieldLabel>
      {children}
      <FieldError>{error}</FieldError>
    </Field>
  );
}

export function FunctionParameterInput({
  name,
  path,
  type,
  value,
  errors,
  onChange,
}: {
  name: string;
  path: string;
  type: ResolvedKalamType;
  value: TestValue;
  errors: TestFieldError[];
  onChange: (path: string, next: TestValue) => void;
}) {
  const error = errorForPath(errors, path);
  const inputId = fieldId(path);

  if (type.kind === "array") {
    const items = Array.isArray(value) ? value : [];
    return (
      <FieldSet>
        <FieldLegend className="text-xs">
          {name}
          <TypeHint type={type} />
        </FieldLegend>
        <FieldGroup className="gap-3">
          {items.map((item, index) => (
            <div key={`${path}-${index}`} className="rounded-md border p-3">
              <div className="mb-2 flex items-center justify-between gap-2">
                <span className="text-xs text-muted-foreground">
                  {name}[{index}]
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={`Remove ${name} item ${index + 1}`}
                  onClick={() => onChange(path, items.filter((_, itemIndex) => itemIndex !== index))}
                >
                  <Trash2 />
                </Button>
              </div>
              <FunctionParameterInput
                name={formatSqlType(type.element)}
                path={`${path}[${index}]`}
                type={type.element}
                value={item}
                errors={errors}
                onChange={onChange}
              />
            </div>
          ))}
          <div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() =>
                onChange(path, [...items, defaultValueForType({ ...type.element, notNull: true })])
              }
            >
              <Plus data-icon="inline-start" />
              Add item
            </Button>
          </div>
          <FieldError>{error}</FieldError>
        </FieldGroup>
      </FieldSet>
    );
  }

  if (type.kind === "composite") {
    const record = value && typeof value === "object" && !Array.isArray(value)
      ? (value as Record<string, TestValue>)
      : {};
    return (
      <FieldSet className="rounded-md border p-3">
        <FieldLegend className="px-1 text-xs">
          {name}
          <TypeHint type={type} />
        </FieldLegend>
        <FieldGroup className="gap-3">
          {type.fields.map((field) => (
            <FunctionParameterInput
              key={field.name}
              name={field.name}
              path={`${path}.${field.name}`}
              type={field.type}
              value={record[field.name]}
              errors={errors}
              onChange={onChange}
            />
          ))}
          <FieldError>{error}</FieldError>
        </FieldGroup>
      </FieldSet>
    );
  }

  if (type.kind === "enum") {
    const selected = typeof value === "string" && value !== "" ? value : NULL_SELECT_VALUE;
    return (
      <ParameterField name={name} type={type} error={error}>
        <Select
          value={selected}
          onValueChange={(next) => onChange(path, next === NULL_SELECT_VALUE ? null : next)}
        >
          <SelectTrigger id={inputId} aria-invalid={Boolean(error)} aria-label={name}>
            <SelectValue placeholder={`Select ${name}`} />
          </SelectTrigger>
          <SelectContent>
            {!type.notNull ? <SelectItem value={NULL_SELECT_VALUE}>NULL</SelectItem> : null}
            {type.values.map((entry) => (
              <SelectItem key={entry} value={entry}>
                {entry}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {type.values.length === 0 ? (
          <p className="text-xs text-muted-foreground">No enum labels found for {type.sqlName}.</p>
        ) : null}
      </ParameterField>
    );
  }

  const builtin = type.kind === "builtin" ? classifyFieldKind(type.sqlName) : "text";

  if (builtin === "boolean") {
    if (!type.notNull) {
      const selected =
        value === true ? "true" : value === false ? "false" : NULL_SELECT_VALUE;
      return (
        <ParameterField name={name} type={type} error={error}>
          <Select
            value={selected}
            onValueChange={(next) => {
              if (next === NULL_SELECT_VALUE) {
                onChange(path, null);
                return;
              }
              onChange(path, next === "true");
            }}
          >
            <SelectTrigger id={inputId} aria-label={name}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={NULL_SELECT_VALUE}>NULL</SelectItem>
              <SelectItem value="true">true</SelectItem>
              <SelectItem value="false">false</SelectItem>
            </SelectContent>
          </Select>
        </ParameterField>
      );
    }

    return (
      <Field orientation="horizontal" className="rounded-md border px-3 py-2">
        <div className="grid gap-0.5">
          <span className="text-xs font-medium">{name}</span>
          <span className="text-xs text-muted-foreground">{formatDeclaredType(type)}</span>
        </div>
        <Switch
          id={inputId}
          checked={value === true}
          onCheckedChange={(checked) => onChange(path, checked)}
          aria-label={name}
        />
      </Field>
    );
  }

  if (builtin === "json") {
    return (
      <ParameterField name={name} type={type} htmlFor={inputId} error={error}>
        <Textarea
          id={inputId}
          className="min-h-28 font-mono text-xs"
          value={jsonText(value)}
          aria-invalid={Boolean(error)}
          onChange={(event) => onChange(path, event.target.value)}
          placeholder="{}"
        />
      </ParameterField>
    );
  }

  const inputType =
    builtin === "datetime" ? "datetime-local" : builtin === "date" ? "date" : builtin === "time" ? "time" : builtin === "int" || builtin === "smallint" || builtin === "bigint" || builtin === "float" || builtin === "decimal" ? "number" : "text";

  const displayValue =
    builtin === "datetime"
      ? datetimeLocalValue(value)
      : value === null || value === undefined
        ? ""
        : String(value);

  return (
    <ParameterField name={name} type={type} htmlFor={inputId} error={error}>
      <Input
        id={inputId}
        type={inputType}
        step={builtin === "float" || builtin === "decimal" ? "any" : undefined}
        value={displayValue}
        aria-invalid={Boolean(error)}
        className={cn(builtin === "uuid" && "font-mono")}
        placeholder={builtin === "uuid" ? "00000000-0000-0000-0000-000000000000" : undefined}
        onChange={(event) => onChange(path, event.target.value === "" ? (type.notNull ? "" : null) : event.target.value)}
      />
    </ParameterField>
  );
}
