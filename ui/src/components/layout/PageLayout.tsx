import type { ReactNode } from "react";
import { cn } from "@/lib/utils";
import { PageHeader } from "./typography";

interface PageLayoutProps {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  breadcrumb?: ReactNode;
  children: ReactNode;
  className?: string;
  contentClassName?: string;
}

export function PageLayout({
  title,
  description,
  actions,
  breadcrumb,
  children,
  className,
  contentClassName,
}: PageLayoutProps) {
  return (
    <section className={cn("flex flex-col gap-6 p-4 lg:p-6", className)}>
      <header className="flex flex-col gap-4">
        {breadcrumb}
        <PageHeader title={title} description={description} actions={actions} />
      </header>
      <div className={cn("flex flex-col gap-4", contentClassName)}>{children}</div>
    </section>
  );
}
