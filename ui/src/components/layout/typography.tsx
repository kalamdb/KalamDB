import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export const pageTitleClassName = "text-2xl font-semibold text-foreground";
export const pageDescriptionClassName = "text-sm text-muted-foreground";
export const panelTitleClassName = "text-sm font-semibold text-foreground";
export const panelDescriptionClassName = "text-xs/relaxed text-muted-foreground";
export const sectionTitleClassName = "text-sm font-semibold text-foreground";
export const chromeLabelClassName = "text-[10px] font-medium uppercase text-muted-foreground";
export const fieldLabelClassName = "text-xs font-medium text-muted-foreground";

interface PageHeaderProps {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  className?: string;
}

export function PageHeader({ title, description, actions, className }: PageHeaderProps) {
  return (
    <div className={cn("flex flex-wrap items-start justify-between gap-3", className)}>
      <div className="flex min-w-0 flex-col gap-1">
        <h1 className={pageTitleClassName}>{title}</h1>
        {description ? <p className={pageDescriptionClassName}>{description}</p> : null}
      </div>
      {actions ? <div className="shrink-0">{actions}</div> : null}
    </div>
  );
}

interface PanelHeaderProps {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  className?: string;
  titleId?: string;
  headingLevel?: "h2" | "h3";
}

export function PanelHeader({
  title,
  description,
  actions,
  className,
  titleId,
  headingLevel = "h2",
}: PanelHeaderProps) {
  const TitleTag = headingLevel;

  return (
    <div className={cn("flex min-w-0 items-start justify-between gap-3", className)}>
      <div className="min-w-0">
        <TitleTag id={titleId} className={panelTitleClassName}>
          {title}
        </TitleTag>
        {description ? <p className={panelDescriptionClassName}>{description}</p> : null}
      </div>
      {actions ? <div className="shrink-0">{actions}</div> : null}
    </div>
  );
}