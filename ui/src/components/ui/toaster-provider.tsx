import {
  createContext,
  useCallback,
  useContext,
  type ReactNode,
} from "react";
import { Toaster as SonnerToaster } from "@/components/ui/sonner";
import { toast } from "sonner";

export type ToastVariant = "default" | "success" | "destructive";

export interface ToastInput {
  title: string;
  description?: string;
  variant?: ToastVariant;
  duration?: number;
}

interface ToastContextValue {
  notify: (input: ToastInput) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

const DEFAULT_DURATION_MS = 4000;
const ERROR_DURATION_MS = 8000;

export function Toaster({ children }: { children: ReactNode }) {
  const notify = useCallback((input: ToastInput) => {
    const duration =
      input.duration ?? (input.variant === "destructive" ? ERROR_DURATION_MS : DEFAULT_DURATION_MS);
    const options = {
      description: input.description,
      duration,
    };

    if (input.variant === "success") {
      toast.success(input.title, options);
      return;
    }

    if (input.variant === "destructive") {
      toast.error(input.title, options);
      return;
    }

    toast(input.title, options);
  }, []);

  return (
    <ToastContext.Provider value={{ notify }}>
      {children}
      <SonnerToaster />
    </ToastContext.Provider>
  );
}

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    throw new Error("useToast must be used within <Toaster>");
  }
  return ctx;
}
