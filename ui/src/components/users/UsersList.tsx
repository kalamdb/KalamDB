import { useMemo, useState } from "react";
import {
  useDeleteUserMutation,
  useGetInviteUsersListQuery,
  useGetUsersListQuery,
  useReinviteUserMutation,
} from "@/store/apiSlice";
import type { User } from "@/services/userService";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { TimestampDisplay } from "@/components/datatype-display/TimestampDisplay";
import { UserForm } from "./UserForm";
import { DeleteUserDialog } from "./DeleteUserDialog";
import {
  ChevronLeft,
  ChevronRight,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Send,
  Trash2,
} from "lucide-react";

const REINVITE_EXPIRES_IN_MS = 7 * 24 * 60 * 60 * 1000;
const PAGE_SIZE_OPTIONS = [10, 25, 50] as const;
const ROOT_USER_ID = "root";

function isInvite(user: User): boolean {
  return user.user_id.startsWith("invite_");
}

function inviteExpired(user: User, now: number): boolean {
  if (!user.invite_expires_at) {
    return false;
  }

  const expiresAt =
    typeof user.invite_expires_at === "number"
      ? user.invite_expires_at
      : new Date(user.invite_expires_at).getTime();
  return Number.isFinite(expiresAt) && expiresAt < now;
}

export function UsersList() {
  const [searchQuery, setSearchQuery] = useState("");
  const [pageSize, setPageSize] = useState<(typeof PAGE_SIZE_OPTIONS)[number]>(25);
  const [page, setPage] = useState(0);
  const {
    data: invites = [],
    isFetching: isInvitesLoading,
    error: invitesError,
    refetch: refetchInvites,
  } = useGetInviteUsersListQuery();
  const usersQuery = useMemo(
    () => ({
      search: searchQuery.trim() || undefined,
      limit: pageSize,
      offset: page * pageSize,
    }),
    [page, pageSize, searchQuery],
  );
  const {
    data: userPage,
    isFetching: isUsersLoading,
    error: usersError,
    refetch: refetchUsers,
  } = useGetUsersListQuery(usersQuery);
  const [deleteUserMutation] = useDeleteUserMutation();
  const [reinviteUserMutation] = useReinviteUserMutation();
  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const [editingUser, setEditingUser] = useState<User | null>(null);
  const [deletingUser, setDeletingUser] = useState<User | null>(null);
  const now = Date.now();
  const users = userPage?.users ?? [];
  const hasMoreUsers = userPage?.hasMore ?? false;
  const isLoading = isInvitesLoading || isUsersLoading;

  const errorMessage = useMemo(() => {
    if (invitesError) {
      return "error" in invitesError && typeof invitesError.error === "string"
        ? invitesError.error
        : "Failed to fetch invites";
    }

    if (usersError) {
      return "error" in usersError && typeof usersError.error === "string"
        ? usersError.error
        : "Failed to fetch users";
    }

    return null;
  }, [invitesError, usersError]);

  const filteredInvites = useMemo(() => invites.filter(isInvite), [invites]);

  const getRoleBadgeColor = (role: string) => {
    switch (role) {
      case "system":
        return "bg-purple-100 text-purple-800";
      case "dba":
        return "bg-blue-100 text-blue-800";
      case "service":
        return "bg-green-100 text-green-800";
      default:
        return "bg-gray-100 text-gray-800";
    }
  };

  const handleReinvite = async (invite: User) => {
    await reinviteUserMutation({
      invite,
      inviteExpiresAt: Date.now() + REINVITE_EXPIRES_IN_MS,
    }).unwrap();
    void Promise.all([refetchInvites(), refetchUsers()]);
  };

  const handleSearchChange = (value: string) => {
    setPage(0);
    setSearchQuery(value);
  };

  const handlePageChange = (newPage: number) => {
    setPage(newPage);
  };

  const handlePageSizeChange = (newSize: number) => {
    setPage(0);
    setPageSize(newSize as (typeof PAGE_SIZE_OPTIONS)[number]);
  };

  const handleRefresh = () => {
    void Promise.all([refetchInvites(), refetchUsers()]);
  };

  if (errorMessage) {
    return (
      <div className="p-4 bg-red-50 border border-red-200 rounded-lg">
        <p className="text-red-700">{errorMessage}</p>
        <Button variant="outline" onClick={handleRefresh} className="mt-2">
          Retry
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {searchQuery.trim() && (
        <p className="text-sm text-muted-foreground">
          Showing results for <span className="font-medium text-foreground">{searchQuery.trim()}</span>
        </p>
      )}
      <div className="flex items-center justify-between gap-4">
        <div className="relative flex-1 max-w-sm">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search users..."
            value={searchQuery}
            onChange={(event) => handleSearchChange(event.target.value)}
            className="pl-9"
          />
        </div>
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Select value={String(pageSize)} onValueChange={(value) => handlePageSizeChange(Number(value))}>
              <SelectTrigger className="h-9 w-[82px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PAGE_SIZE_OPTIONS.map((size) => (
                  <SelectItem key={size} value={String(size)}>
                    {size}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <span>per page</span>
          </div>
          <div className="flex items-center gap-1">
            <Button
              variant="outline"
              size="icon"
              onClick={() => handlePageChange(page - 1)}
              disabled={page === 0}
              aria-label="Previous users page"
            >
              <ChevronLeft className="h-4 w-4" />
            </Button>
            <span className="text-sm text-muted-foreground px-2">Page {page + 1}</span>
            <Button
              variant="outline"
              size="icon"
              onClick={() => handlePageChange(page + 1)}
              disabled={!hasMoreUsers}
              aria-label="Next users page"
            >
              <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
          <Button
            variant="outline"
            size="icon"
            onClick={handleRefresh}
            disabled={isLoading}
            aria-label="Refresh users"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
          </Button>
          <Button onClick={() => setIsCreateOpen(true)}>
            <Plus className="h-4 w-4 mr-2" />
            Create User
          </Button>
        </div>
      </div>

      {isLoading && users.length === 0 && invites.length === 0 ? (
        <div className="flex items-center justify-center py-8">
          <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        </div>
      ) : (
        <div className="space-y-5">
          <section aria-label="Pending invites" className="space-y-2">
            <div className="flex items-center justify-between">
              <h2 className="text-sm font-semibold">Pending Invites</h2>
              <span className="text-xs text-muted-foreground">{filteredInvites.length} invites</span>
            </div>
            <div className="border rounded-lg">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>ID</TableHead>
                    <TableHead>Email</TableHead>
                    <TableHead>Role</TableHead>
                    <TableHead>Expire Date</TableHead>
                    <TableHead>Created Date</TableHead>
                    <TableHead className="w-[120px]">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {filteredInvites.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                        No invites found
                      </TableCell>
                    </TableRow>
                  ) : (
                    filteredInvites.map((invite, index) => {
                      const expired = inviteExpired(invite, now);
                      return (
                        <TableRow
                          key={invite.user_id || `invite-${index}`}
                          className={expired ? "bg-red-50 hover:bg-red-50/80" : undefined}
                        >
                          <TableCell className="font-medium">{invite.user_id}</TableCell>
                          <TableCell className="text-muted-foreground">
                            {invite.email || "—"}
                          </TableCell>
                          <TableCell>
                            <span className={`px-2 py-1 rounded-full text-xs font-medium ${getRoleBadgeColor(invite.role)}`}>
                              {invite.role}
                            </span>
                          </TableCell>
                          <TableCell className={expired ? "font-medium text-red-700" : "text-muted-foreground"}>
                            {invite.invite_expires_at ? (
                              <span className="inline-flex items-center gap-2">
                                <TimestampDisplay value={invite.invite_expires_at} />
                                {expired && <span className="text-xs font-semibold uppercase">Expired</span>}
                              </span>
                            ) : (
                              "—"
                            )}
                          </TableCell>
                          <TableCell className="text-muted-foreground">
                            {invite.created_at ? (
                              <TimestampDisplay value={invite.created_at} />
                            ) : (
                              "—"
                            )}
                          </TableCell>
                          <TableCell>
                            <div className="flex items-center gap-1">
                              <Button
                                variant="ghost"
                                size="icon"
                                onClick={() => void handleReinvite(invite)}
                                aria-label={`Reinvite ${invite.email || invite.user_id}`}
                                title={`Reinvite ${invite.email || invite.user_id}`}
                              >
                                <Send className="h-4 w-4" />
                              </Button>
                              <Button
                                variant="ghost"
                                size="icon"
                                onClick={() => setDeletingUser(invite)}
                                aria-label={`Delete invite ${invite.user_id}`}
                                title={`Delete invite ${invite.user_id}`}
                              >
                                <Trash2 className="h-4 w-4" />
                              </Button>
                            </div>
                          </TableCell>
                        </TableRow>
                      );
                    })
                  )}
                </TableBody>
              </Table>
            </div>
          </section>

          <section aria-label="Users list" className="space-y-2">
            <div className="flex items-center justify-between">
              <h2 className="text-sm font-semibold">Users</h2>
              <span className="text-xs text-muted-foreground">
                {users.length} users{hasMoreUsers ? "+" : ""}
              </span>
            </div>
            <div className="border rounded-lg">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Name</TableHead>
                    <TableHead>User ID</TableHead>
                    <TableHead>Role</TableHead>
                    <TableHead>Email</TableHead>
                    <TableHead>Created</TableHead>
                    <TableHead className="w-[100px]">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {users.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={6} className="text-center py-8 text-muted-foreground">
                        {searchQuery ? "No users match your search" : "No users found"}
                      </TableCell>
                    </TableRow>
                  ) : (
                    users.map((user, index) => (
                      <TableRow key={user.user_id || `user-${index}`}>
                        <TableCell className="font-medium">{user.name || "—"}</TableCell>
                        <TableCell className="text-muted-foreground">{user.user_id}</TableCell>
                        <TableCell>
                          <span className={`px-2 py-1 rounded-full text-xs font-medium ${getRoleBadgeColor(user.role)}`}>
                            {user.role}
                          </span>
                        </TableCell>
                        <TableCell className="text-muted-foreground">
                          {user.email || "—"}
                        </TableCell>
                        <TableCell className="text-muted-foreground">
                          {user.created_at ? (
                            <TimestampDisplay value={user.created_at} />
                          ) : (
                            "—"
                          )}
                        </TableCell>
                        <TableCell>
                          <div className="flex items-center gap-1">
                            <Button
                              variant="ghost"
                              size="icon"
                              onClick={() => setEditingUser(user)}
                              aria-label={`Edit ${user.user_id}`}
                              title={`Edit ${user.user_id}`}
                            >
                              <Pencil className="h-4 w-4" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              onClick={() => setDeletingUser(user)}
                              disabled={user.user_id === ROOT_USER_ID}
                              aria-label={`Delete ${user.user_id}`}
                              title={
                                user.user_id === ROOT_USER_ID
                                  ? "Root user cannot be deleted"
                                  : `Delete ${user.user_id}`
                              }
                            >
                              <Trash2 className="h-4 w-4" />
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    ))
                  )}
                </TableBody>
              </Table>
            </div>
          </section>
        </div>
      )}

      <UserForm
        open={isCreateOpen}
        onOpenChange={setIsCreateOpen}
        onSuccess={() => {
          setIsCreateOpen(false);
          void handleRefresh();
        }}
      />

      {editingUser && (
        <UserForm
          open={true}
          onOpenChange={() => setEditingUser(null)}
          user={editingUser}
          onSuccess={() => {
            setEditingUser(null);
            void handleRefresh();
          }}
        />
      )}

      {deletingUser && (
        <DeleteUserDialog
          open={true}
          onOpenChange={() => setDeletingUser(null)}
          user={deletingUser}
          onConfirm={async () => {
            if (deletingUser.user_id === ROOT_USER_ID) {
              throw new Error("Root user cannot be removed from the system");
            }
            await deleteUserMutation({ username: deletingUser.user_id }).unwrap();
            setDeletingUser(null);
            void handleRefresh();
          }}
        />
      )}
    </div>
  );
}
