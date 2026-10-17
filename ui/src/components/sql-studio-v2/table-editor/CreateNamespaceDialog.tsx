import { useEffect, useState } from "react";
import { FolderPlus } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

interface CreateNamespaceDialogProps {
  open: boolean;
  existingNames: string[];
  onSubmit: (name: string) => void;
  onClose: () => void;
}

const NAME_PATTERN = /^[a-zA-Z_][a-zA-Z0-9_]*$/;

export function CreateNamespaceDialog({
  open,
  existingNames,
  onSubmit,
  onClose,
}: CreateNamespaceDialogProps) {
  const [name, setName] = useState("");
  const [touched, setTouched] = useState(false);

  useEffect(() => {
    if (open) {
      setName("");
      setTouched(false);
    }
  }, [open]);

  const trimmed = name.trim();
  const error = (() => {
    if (!trimmed) return "Name is required";
    if (!NAME_PATTERN.test(trimmed)) {
      return "Use only letters, digits and underscores. Must start with a letter or underscore.";
    }
    if (existingNames.includes(trimmed)) {
      return `Schema "${trimmed}" already exists.`;
    }
    return null;
  })();

  const handleSubmit = (event: React.FormEvent) => {
    event.preventDefault();
    setTouched(true);
    if (error) return;
    onSubmit(trimmed);
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <FolderPlus className="h-4 w-4" />
            New schema
          </DialogTitle>
          <DialogDescription>
            Schemas group tables, the same way a PostgreSQL schema does.
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="flex flex-col gap-3">
          <label className="flex flex-col gap-1 text-sm">
            <span className="font-medium text-muted-foreground">Name</span>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. analytics"
              autoFocus
              className="font-mono"
            />
          </label>

          {touched && error && (
            <p className="text-xs text-destructive">{error}</p>
          )}

          <DialogFooter className="gap-2 sm:gap-2">
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit">Create</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
