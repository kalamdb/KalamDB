import { Fragment } from "react";
import { Link } from "react-router-dom";
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";
import { cn } from "@/lib/utils";

export type PageBreadcrumbItem = {
  label: string;
  to?: string;
  mono?: boolean;
};

export function PageBreadcrumb({ items }: { items: PageBreadcrumbItem[] }) {
  if (items.length === 0) {
    return null;
  }

  return (
    <Breadcrumb>
      <BreadcrumbList>
        {items.map((item, index) => {
          const isLast = index === items.length - 1;
          const labelClassName = cn("max-w-[48ch] truncate", item.mono && "font-mono");

          return (
            <Fragment key={`${item.label}-${index}`}>
              {index > 0 ? <BreadcrumbSeparator /> : null}
              <BreadcrumbItem>
                {isLast ? (
                  <BreadcrumbPage className={labelClassName}>{item.label}</BreadcrumbPage>
                ) : item.to ? (
                  <BreadcrumbLink asChild>
                    <Link to={item.to} className={labelClassName}>
                      {item.label}
                    </Link>
                  </BreadcrumbLink>
                ) : (
                  <span className={labelClassName}>{item.label}</span>
                )}
              </BreadcrumbItem>
            </Fragment>
          );
        })}
      </BreadcrumbList>
    </Breadcrumb>
  );
}
