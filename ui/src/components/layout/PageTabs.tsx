import type { ComponentProps } from "react";
import { Tabs, TabsList } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";

export function PageTabs({ className, ...props }: ComponentProps<typeof Tabs>) {
  return <Tabs className={cn("flex-col gap-4", className)} {...props} />;
}

export function PageTabsList({
  className,
  ...props
}: Omit<ComponentProps<typeof TabsList>, "variant">) {
  return (
    <TabsList
      variant="line"
      className={cn("max-w-full justify-start", className)}
      {...props}
    />
  );
}
