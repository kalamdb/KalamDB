import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
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

  if (type.kind === "array") {
    const items = Array.isArray(value) ? value : [];
    return (
      <fieldset className="grid gap-2">
        <legend className="text-xs font-medium">
          {name}
          <span className="ml-2 font-normal text-muted-foreground">{formatDeclaredType(type)}</span>
        </legend>
        <div className="grid gap-3">
          {items.map((item, index) => (
            <div key={`${path}-${index}`} className="rounded-md border p-3">
              <div className="mb-2 flex items-center justify-between gap-2">
                <span className="text-xs text-muted-foreground">
                  {name}[{index}]
                </span>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xxs"
                  aria-label={`Remove ${name} item ${index + 1}`}
                  onClick={() => onChange(path, items.filter((_, itemIndex) => itemIndex !== index))}
                >
                  <Trash2 className="size-3.5" />
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
        </div>
        <div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() =>
              onChange(path, [...items, defaultValueForType({ ...type.element, notNull: true })])
            }
          >
            <Plus className="size-3.5" />
            Add item
          </Button>
        </div>
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </fieldset>
    );
  }

  if (type.kind === "composite") {
    const record = value && typeof value === "object" && !Array.isArray(value)
      ? (value as Record<string, TestValue>)
      : {};
    return (
      <fieldset className="grid gap-3 rounded-md border p-3">
        <legend className="px-1 text-xs font-medium">
          {name}
          <span className="ml-2 font-normal text-muted-foreground">{formatDeclaredType(type)}</span>
        </legend>
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
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </fieldset>
    );
  }

  if (type.kind === "enum") {
    const selected = typeof value === "string" && value !== "" ? value : NULL_SELECT_VALUE;
    return (
      <label className="grid gap-1.5">
        <span className="text-xs font-medium">
          {name}
          <span className="ml-2 font-normal text-muted-foreground">{formatDeclaredType(type)}</span>
        </span>
        <Select
          value={selected}
          onValueChange={(next) => onChange(path, next === NULL_SELECT_VALUE ? null : next)}
        >
          <SelectTrigger aria-invalid={Boolean(error)}>
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
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </label>
    );
  }

  const builtin = type.kind === "builtin" ? classifyFieldKind(type.sqlName) : "text";

  if (builtin === "boolean") {
    if (!type.notNull) {
      const selected =
        value === true ? "true" : value === false ? "false" : NULL_SELECT_VALUE;
      return (
        <label className="grid gap-1.5">
          <span className="text-xs font-medium">
            {name}
            <span className="ml-2 font-normal text-muted-foreground">{formatDeclaredType(type)}</span>
          </span>
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
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={NULL_SELECT_VALUE}>NULL</SelectItem>
              <SelectItem value="true">true</SelectItem>
              <SelectItem value="false">false</SelectItem>
            </SelectContent>
          </Select>
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
        </label>
      );
    }

    return (
      <div className="flex items-center justify-between gap-3 rounded-md border px-3 py-2">
        <div className="grid gap-0.5">
          <span className="text-xs font-medium">{name}</span>
          <span className="text-xs text-muted-foreground">{formatDeclaredType(type)}</span>
        </div>
        <Switch
          checked={value === true}
          onCheckedChange={(checked) => onChange(path, checked)}
          aria-label={name}
        />
      </div>
    );
  }

  if (builtin === "json") {
    return (
      <label className="grid gap-1.5">
        <span className="text-xs font-medium">
          {name}
          <span className="ml-2 font-normal text-muted-foreground">{formatDeclaredType(type)}</span>
        </span>
        <Textarea
          className="min-h-28 font-mono text-xs"
          value={jsonText(value)}
          aria-invalid={Boolean(error)}
          onChange={(event) => onChange(path, event.target.value)}
          placeholder="{}"
        />
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </label>
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
    <label className="grid gap-1.5">
      <span className="text-xs font-medium">
        {name}
        <span className="ml-2 font-normal text-muted-foreground">{formatDeclaredType(type)}</span>
      </span>
      <Input
        type={inputType}
        step={builtin === "float" || builtin === "decimal" ? "any" : undefined}
        value={displayValue}
        aria-invalid={Boolean(error)}
        className={cn(builtin === "uuid" && "font-mono")}
        placeholder={builtin === "uuid" ? "00000000-0000-0000-0000-000000000000" : undefined}
        onChange={(event) => onChange(path, event.target.value === "" ? (type.notNull ? "" : null) : event.target.value)}
      />
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </label>
  );
}
