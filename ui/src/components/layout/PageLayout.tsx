import type { ReactNode } from "react";
import { cn } from "@/lib/utils";
import { PageHeader } from "./typography";

interface PageLayoutProps {
  title: string;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  contentClassName?: string;
}

export function PageLayout({
  title,
  description,
  actions,
  children,
  className,
  contentClassName,
}: PageLayoutProps) {
  return (
    <section className={cn("flex flex-col gap-6 p-4 lg:p-6", className)}>
      <PageHeader title={title} description={description} actions={actions} />
      <div className={cn("flex flex-col gap-4", contentClassName)}>{children}</div>
    </section>
  );
}
