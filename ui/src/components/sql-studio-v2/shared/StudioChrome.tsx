import type { ComponentProps, ReactNode } from "react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { chromeLabelClassName } from "@/components/layout/typography";
import { cn } from "@/lib/utils";

export function StudioChromeLabel({ className, ...props }: ComponentProps<"span">) {
  return <span className={cn(chromeLabelClassName, className)} {...props} />;
}

type StudioIconButtonProps = Omit<ComponentProps<typeof Button>, "size" | "variant"> & {
  tooltip?: ReactNode;
  tone?: "neutral" | "destructive";
};

export function StudioIconButton({
  tooltip,
  tone = "neutral",
  className,
  children,
  disabled,
  ...props
}: StudioIconButtonProps) {
  const button = (
    <Button
      type="button"
      variant="ghost"
      size="icon-xxs"
      disabled={disabled}
      className={cn(
        "text-muted-foreground hover:text-foreground",
        tone === "destructive" && "hover:bg-destructive/10 hover:text-destructive",
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );

  if (!tooltip) {
    return button;
  }

  return (
    <TooltipProvider delayDuration={250}>
      <Tooltip>
        <TooltipTrigger asChild>
          {disabled ? <span className="inline-flex">{button}</span> : button}
        </TooltipTrigger>
        <TooltipContent>{tooltip}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}