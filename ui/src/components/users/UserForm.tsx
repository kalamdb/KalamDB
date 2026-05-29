import { useEffect, useMemo, useState } from "react";
import { AlertCircle, Loader2 } from "lucide-react";
import { useCreateUserMutation, useGetStoragesQuery, useUpdateUserMutation } from "@/store/apiSlice";
import type { User } from "@/services/userService";
import { formatTimestamp } from "@/lib/formatters";
import { getErrorMessage } from "@/lib/errors";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

interface UserFormProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  user?: User;
  onSuccess: () => void;
}

const ROLES = ["user", "service", "dba", "system"] as const;
const STORAGE_MODES = ["table", "region"] as const;
const NONE_STORAGE_ID = "__none__";
type CreateMode = "create" | "invite";

interface FormData {
  username: string;
  password: string;
  role: string;
  email: string;
  inviteExpiresDays: string;
  storageMode: "table" | "region";
  storageId: string;
}

function valueToText(value: unknown): string {
  if (value === null || value === undefined) {
    return "";
  }
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function formatSystemValue(value: unknown): string {
  if (value === null || value === undefined) {
    return "N/A";
  }
  if (typeof value === "object") {
    return valueToText(value);
  }
  return String(value);
}

function formatTimestampValue(value: unknown): string {
  if (!value) {
    return "N/A";
  }

  if (typeof value !== "string" && typeof value !== "number") {
    return "N/A";
  }

  return formatTimestamp(value, undefined, "iso8601-datetime", "utc");
}

function FieldHelp({ text }: { text: string }) {
  return <p className="text-xs text-muted-foreground">{text}</p>;
}

interface SystemFieldProps {
  label: string;
  value: unknown;
  description: string;
  monospace?: boolean;
  breakAll?: boolean;
}

function SystemField({ label, value, description, monospace, breakAll }: SystemFieldProps) {
  return (
    <div className="space-y-1">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className={["text-sm", monospace ? "font-mono" : "", breakAll ? "break-all" : ""].join(" ").trim()}>
        {formatSystemValue(value)}
      </p>
      <p className="text-xs text-muted-foreground">{description}</p>
    </div>
  );
}

export function UserForm({ open, onOpenChange, user, onSuccess }: UserFormProps) {
  const [createUserMutation] = useCreateUserMutation();
  const [updateUserMutation] = useUpdateUserMutation();
  const { data: storages = [] } = useGetStoragesQuery();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [createMode, setCreateMode] = useState<CreateMode>("create");
  const [formData, setFormData] = useState<FormData>({
    username: "",
    password: "",
    role: "user",
    email: "",
    inviteExpiresDays: "7",
    storageMode: "table",
    storageId: "",
  });

  const isEditing = Boolean(user);

  useEffect(() => {
    if (!open) {
      return;
    }

    setFormData({
      username: user?.user_id ?? "",
      password: "",
      role: user?.role ?? "user",
      email: user?.email ?? "",
      inviteExpiresDays: "7",
      storageMode: user?.storage_mode === "region" ? "region" : "table",
      storageId: user?.storage_id ?? "",
    });
    if (!user) {
      setCreateMode("create");
    }
    setError(null);
  }, [open, user]);

  const isInviteMode = !isEditing && createMode === "invite";
  const showPasswordField = !isEditing ? !isInviteMode : true;
  const showInviteExpiryField = isInviteMode;

  const canSubmit = useMemo(() => {
    if (isEditing) {
      return true;
    }

    if (!isInviteMode && !formData.username.trim()) {
      return false;
    }
    if (isInviteMode && !formData.email.trim()) {
      return false;
    }
    if (!isInviteMode && !formData.password.trim()) {
      return false;
    }
    if (isInviteMode && Number(formData.inviteExpiresDays) <= 0) {
      return false;
    }
    return true;
  }, [
    isEditing,
    isInviteMode,
    formData.username,
    formData.password,
    formData.email,
    formData.inviteExpiresDays,
  ]);

  const usernameHelpText = useMemo(() => {
    if (isEditing) {
      return "Username cannot be changed after creation.";
    }
    if (isInviteMode) {
      return "Invite rows get a system-generated ID and become real users after OIDC login.";
    }
    return "Unique user login name.";
  }, [isEditing, isInviteMode]);

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    setIsSubmitting(true);
    setError(null);

    try {
      if (isEditing && user) {
        const updateInput: {
          role?: string;
          password?: string;
          email?: string;
          storage_mode?: "table" | "region" | null;
          storage_id?: string | null;
        } = {};
        if (formData.role !== user.role) {
          updateInput.role = formData.role;
        }
        if (formData.password.trim()) {
          updateInput.password = formData.password.trim();
        }
        if ((formData.email || "") !== (user.email || "")) {
          updateInput.email = formData.email.trim();
        }
        if (formData.storageMode !== (user.storage_mode ?? "table")) {
          updateInput.storage_mode = formData.storageMode;
        }
        const normalizedStorageId = formData.storageId.trim() || null;
        if (normalizedStorageId !== (user.storage_id ?? null)) {
          updateInput.storage_id = normalizedStorageId;
        }
        await updateUserMutation({ username: user.user_id, input: updateInput }).unwrap();
      } else {
        const inviteExpiresDays = Number(formData.inviteExpiresDays);
        const inviteExpiresAt = Date.now() + inviteExpiresDays * 24 * 60 * 60 * 1000;
        await createUserMutation({
          username: isInviteMode ? undefined : formData.username.trim(),
          password: isInviteMode ? undefined : formData.password.trim(),
          auth_type: isInviteMode ? "oidc_invite" : "password",
          role: formData.role,
          email: formData.email.trim() || undefined,
          invite_expires_at: isInviteMode ? Math.round(inviteExpiresAt) : undefined,
          storage_mode: formData.storageMode,
          storage_id: formData.storageId.trim() || null,
        }).unwrap();
      }

      onSuccess();
      onOpenChange(false);
    } catch (submitError) {
      setError(getErrorMessage(submitError, "Failed to save user"));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="flex flex-col sm:max-w-[620px]">
        <SheetHeader>
          <SheetTitle>{isEditing ? "Edit User" : "Create User"}</SheetTitle>
          <SheetDescription>
            {isEditing
              ? "Update account details, role, and storage preferences. Some authentication metadata remains system-managed."
              : "Create a password user account or send an OIDC invite."}
          </SheetDescription>
        </SheetHeader>

        <form id="user-form" onSubmit={handleSubmit} className="flex min-h-0 flex-1 flex-col">
          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-5">
            {!isEditing && (
              <Tabs
                value={createMode}
                onValueChange={(value) => setCreateMode(value as CreateMode)}
                className="gap-2"
              >
                <TabsList className="grid w-full grid-cols-2">
                  <TabsTrigger value="create">Create User</TabsTrigger>
                  <TabsTrigger value="invite">Invite User</TabsTrigger>
                </TabsList>
              </Tabs>
            )}

            {isEditing && user && (
              <div className="space-y-2">
                <label className="text-sm font-medium">User ID</label>
                <Input value={user.user_id} disabled className="font-mono" />
                <FieldHelp text="Stable system identifier for this account." />
              </div>
            )}

            <div className="space-y-2">
              <label className="text-sm font-medium">Username</label>
              <Input
                value={formData.username}
                onChange={(event) => setFormData((prev) => ({ ...prev, username: event.target.value }))}
                disabled={isEditing || isInviteMode}
                placeholder="e.g. analyst_01"
                autoFocus={!isEditing && !isInviteMode}
              />
              <FieldHelp text={usernameHelpText} />
            </div>

            <div className="space-y-2">
              <label className="text-sm font-medium">Role</label>
              <Select
                value={formData.role}
                onValueChange={(value) => setFormData((prev) => ({ ...prev, role: value }))}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {ROLES.map((role) => (
                    <SelectItem key={role} value={role}>
                      {role}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FieldHelp text="Access level for this account: user, service, dba, or system." />
            </div>

            {showPasswordField && (
              <div className="space-y-2">
                <label className="text-sm font-medium">
                  Password {isEditing ? "(leave empty to keep current password)" : ""}
                </label>
                <Input
                  type="password"
                  value={formData.password}
                  onChange={(event) => setFormData((prev) => ({ ...prev, password: event.target.value }))}
                  placeholder={isEditing ? "••••••••" : "Enter password"}
                />
                <FieldHelp text="Minimum 8 characters recommended." />
              </div>
            )}

            {showInviteExpiryField && (
              <div className="space-y-2">
                <label className="text-sm font-medium">Invite Expiration</label>
                <Input
                  type="number"
                  min="1"
                  step="1"
                  value={formData.inviteExpiresDays}
                  onChange={(event) => setFormData((prev) => ({ ...prev, inviteExpiresDays: event.target.value }))}
                />
                <FieldHelp text="Number of days this OIDC email invite remains usable." />
              </div>
            )}

            <div className="space-y-2">
              <label className="text-sm font-medium">Email</label>
              <Input
                type="email"
                value={formData.email}
                onChange={(event) => setFormData((prev) => ({ ...prev, email: event.target.value }))}
                placeholder="user@example.com"
              />
              <FieldHelp text="Optional contact email for this user." />
            </div>

            <div className="space-y-2">
              <label className="text-sm font-medium">Storage Mode</label>
              <Select
                value={formData.storageMode}
                onValueChange={(value) =>
                  setFormData((prev) => ({ ...prev, storageMode: value as "table" | "region" }))
                }
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {STORAGE_MODES.map((mode) => (
                    <SelectItem key={mode} value={mode}>
                      {mode}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FieldHelp text="Controls how this user resolves storage placement (table or region)." />
            </div>

            <div className="space-y-2">
              <label className="text-sm font-medium">Storage ID</label>
              <Select
                value={formData.storageId || NONE_STORAGE_ID}
                onValueChange={(value) =>
                  setFormData((prev) => ({ ...prev, storageId: value === NONE_STORAGE_ID ? "" : value }))
                }
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select storage target" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NONE_STORAGE_ID}>No preferred storage</SelectItem>
                  {storages.map((storage) => (
                    <SelectItem key={storage.storage_id} value={storage.storage_id}>
                      {storage.storage_name} ({storage.storage_id})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FieldHelp text="Optional preferred storage configuration for this user." />
            </div>

            {isEditing && user && (
              <div className="space-y-3 rounded-md border bg-muted/40 p-4">
                <h3 className="text-sm font-semibold">System Fields (Read-only)</h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                  <SystemField
                    label="Auth Type"
                    value={user.auth_type}
                    description="Current authentication strategy used by this user."
                  />
                  <SystemField
                    label="Auth Data"
                    value={user.auth_data}
                    description="Auth provider payload (OAuth/internal metadata)."
                    monospace
                    breakAll
                  />
                  <SystemField
                    label="Storage Mode"
                    value={user.storage_mode}
                    description="How user data is partitioned for storage selection."
                  />
                  <SystemField
                    label="Storage ID"
                    value={user.storage_id}
                    description="Preferred storage target when configured."
                  />
                  <SystemField
                    label="Failed Login Attempts"
                    value={user.failed_login_attempts}
                    description="Consecutive authentication failures before lockout."
                  />
                  <SystemField
                    label="Locked Until"
                    value={formatTimestampValue(user.locked_until)}
                    description="Lockout expiration timestamp if account is temporarily blocked."
                  />
                  <SystemField
                    label="Last Login"
                    value={formatTimestampValue(user.last_login_at)}
                    description="Most recent successful login timestamp."
                  />
                  <SystemField
                    label="Last Seen"
                    value={formatTimestampValue(user.last_seen)}
                    description="Latest authenticated activity timestamp."
                  />
                  <SystemField
                    label="Invite Expires"
                    value={formatTimestampValue(user.invite_expires_at)}
                    description="Expiration timestamp for pending OIDC email invites."
                  />
                  <SystemField
                    label="Invited By"
                    value={user.invited_by}
                    description="Admin user that created the invite."
                  />
                  <SystemField
                    label="Created At"
                    value={formatTimestampValue(user.created_at)}
                    description="Account creation timestamp."
                  />
                  <SystemField
                    label="Updated At"
                    value={formatTimestampValue(user.updated_at)}
                    description="Last profile update timestamp."
                  />
                </div>
                <FieldHelp text="These fields are currently system-managed and displayed for visibility." />
              </div>
            )}

            {error && (
              <Alert variant="destructive">
                <AlertCircle className="h-4 w-4" />
                <AlertTitle>Save failed</AlertTitle>
                <AlertDescription>{error}</AlertDescription>
              </Alert>
            )}
          </div>
        </form>

        <SheetFooter>
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button form="user-form" type="submit" disabled={isSubmitting || !canSubmit}>
            {isSubmitting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {isEditing ? "Save Changes" : "Create User"}
          </Button>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  );
}
