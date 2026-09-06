import { Plus, Trash2, Undo2 } from "lucide-react";
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
import {
  fieldLabelClassName,
  sectionTitleClassName,
} from "@/components/layout/typography";
import {
  POLICY_COMMANDS,
  type DraftPolicyCommand,
  type DraftPolicyTarget,
  type DraftTablePolicy,
  newDraftPolicy,
} from "./types";
import { policyUsesUsing, policyUsesWithCheck } from "./ddl-generator";

const COMMAND_HELP: Record<DraftPolicyCommand, string> = {
  all: "SELECT, INSERT, UPDATE, and DELETE",
  select: "read rows that match USING",
  insert: "write new rows that match WITH CHECK",
  update: "old row must match USING; new row must match WITH CHECK",
  delete: "delete rows that match USING",
};

function titleCase(value: string): string {
  return `${value.slice(0, 1).toUpperCase()}${value.slice(1)}`;
}

function toggleTarget(
  targets: DraftPolicyTarget[],
  target: DraftPolicyTarget,
  enabled: boolean,
): DraftPolicyTarget[] {
  if (target === "public") {
    return enabled ? ["public"] : ["user", "service"];
  }
  const withoutPublic = targets.filter((item) => item !== "public");
  if (enabled) {
    return withoutPublic.includes(target)
      ? withoutPublic
      : [...withoutPublic, target];
  }
  return withoutPublic.filter((item) => item !== target);
}

function PolicyCard({
  policy,
  disabled,
  error,
  onChange,
  onDelete,
}: {
  policy: DraftTablePolicy;
  disabled?: boolean;
  error?: string | null;
  onChange: (next: DraftTablePolicy) => void;
  onDelete: () => void;
}) {
  const showUsing = policyUsesUsing(policy.command);
  const showCheck = policyUsesWithCheck(policy.command);
  const isPublic = policy.targets.includes("public");
  const commandLocked = !policy.isNew;

  return (
    <div
      className={`space-y-3 p-3 ${policy.isDeleted ? "opacity-60" : ""}`}
      data-testid="table-policy-card"
    >
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(0,1fr)_8rem_auto]">
        <label className="flex flex-col gap-1.5">
          <span className={fieldLabelClassName}>Name</span>
          <Input
            value={policy.name}
            onChange={(event) =>
              onChange({ ...policy, name: event.target.value })
            }
            disabled={disabled || policy.isDeleted}
            placeholder="owner_read"
            className="h-8 font-mono text-xs"
            data-testid="table-policy-name"
          />
        </label>
        <label className="flex flex-col gap-1.5">
          <span className={fieldLabelClassName}>For</span>
          <Select
            value={policy.command}
            disabled={disabled || policy.isDeleted || commandLocked}
            onValueChange={(value) => {
              const command = value as DraftPolicyCommand;
              onChange({
                ...policy,
                command,
                usingExpr: policyUsesUsing(command) ? policy.usingExpr : "",
                withCheckExpr: policyUsesWithCheck(command)
                  ? policy.withCheckExpr
                  : "",
              });
            }}
          >
            <SelectTrigger
              size="sm"
              className="h-8 text-xs"
              data-testid="table-policy-command"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {POLICY_COMMANDS.map((command) => (
                <SelectItem key={command} value={command}>
                  {titleCase(command)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </label>
        {!disabled ? (
          <div className="flex items-end justify-end">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={onDelete}
              className="h-8 w-8 p-0 text-muted-foreground hover:text-destructive"
              aria-label={
                policy.isDeleted ? `Restore ${policy.name || "policy"}` : `Remove ${policy.name || "policy"}`
              }
            >
              {policy.isDeleted ? (
                <Undo2 className="h-3.5 w-3.5" />
              ) : (
                <Trash2 className="h-3.5 w-3.5" />
              )}
            </Button>
          </div>
        ) : null}
      </div>

      <p className="text-[11px] leading-snug text-muted-foreground">
        {commandLocked
          ? "Command is fixed after create; drop and add a new policy to change it. "
          : null}
        {COMMAND_HELP[policy.command]}.
      </p>

      <div className="flex flex-wrap gap-4">
        {(["public", "user", "service"] as const).map((target) => (
          <label key={target} className="flex items-center gap-2">
            <Switch
              size="sm"
              checked={
                target === "public"
                  ? isPublic
                  : !isPublic && policy.targets.includes(target)
              }
              disabled={disabled || policy.isDeleted}
              onCheckedChange={(checked) =>
                onChange({
                  ...policy,
                  targets: toggleTarget(policy.targets, target, checked),
                })
              }
              aria-label={`Policy applies to ${target}`}
              data-testid={`table-policy-target-${target}`}
            />
            <span className={fieldLabelClassName}>
              {target === "public" ? "Public" : titleCase(target)}
            </span>
          </label>
        ))}
      </div>

      {showUsing ? (
        <label className="flex flex-col gap-1.5">
          <span className={fieldLabelClassName}>USING</span>
          <Textarea
            value={policy.usingExpr}
            onChange={(event) =>
              onChange({ ...policy, usingExpr: event.target.value })
            }
            disabled={disabled || policy.isDeleted}
            placeholder="owner_id = CURRENT_USER()"
            className="min-h-16 font-mono text-xs"
            data-testid="table-policy-using"
          />
        </label>
      ) : null}

      {showCheck ? (
        <label className="flex flex-col gap-1.5">
          <span className={fieldLabelClassName}>WITH CHECK</span>
          <Textarea
            value={policy.withCheckExpr}
            onChange={(event) =>
              onChange({ ...policy, withCheckExpr: event.target.value })
            }
            disabled={disabled || policy.isDeleted}
            placeholder="owner_id = CURRENT_USER()"
            className="min-h-16 font-mono text-xs"
            data-testid="table-policy-check"
          />
        </label>
      ) : null}

      {error ? (
        <p className="text-[11px] text-destructive">{error}</p>
      ) : null}
    </div>
  );
}

export function TablePoliciesEditor({
  policies,
  disabled,
  errors,
  onChange,
}: {
  policies: DraftTablePolicy[];
  disabled?: boolean;
  errors?: Record<string, string>;
  onChange: (policies: DraftTablePolicy[]) => void;
}) {
  const addPolicy = () => {
    onChange([...policies, newDraftPolicy()]);
  };

  const updatePolicy = (next: DraftTablePolicy) => {
    onChange(policies.map((policy) => (policy.id === next.id ? next : policy)));
  };

  const deletePolicy = (policy: DraftTablePolicy) => {
    if (policy.isNew) {
      onChange(policies.filter((item) => item.id !== policy.id));
      return;
    }
    updatePolicy({ ...policy, isDeleted: !policy.isDeleted });
  };

  return (
    <section className="space-y-2" data-testid="table-policies-section">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h3 className={sectionTitleClassName}>Row-level policies</h3>
          <p className="mt-0.5 text-[11px] leading-snug text-muted-foreground">
            Shared tables deny User and Service rows until a policy allows them.
            System and DBA bypass policies.
          </p>
        </div>
        {!disabled ? (
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={addPolicy}
            className="gap-1.5 text-xs"
            data-testid="table-policy-add"
          >
            <Plus data-icon="inline-start" />
            Add policy
          </Button>
        ) : null}
      </div>
      <div className="overflow-hidden rounded-md border border-border divide-y divide-border">
        {policies.length === 0 ? (
          <p className="px-3 py-6 text-center text-xs text-muted-foreground">
            No policies yet. Without one, User and Service queries return no
            rows.
          </p>
        ) : (
          policies.map((policy) => (
            <PolicyCard
              key={policy.id}
              policy={policy}
              disabled={disabled}
              error={errors?.[policy.id] ?? null}
              onChange={updatePolicy}
              onDelete={() => deletePolicy(policy)}
            />
          ))
        )}
      </div>
    </section>
  );
}
