// @vitest-environment jsdom

import "@testing-library/jest-dom/vitest";
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { PageBreadcrumb } from "@/components/layout/PageBreadcrumb";

describe("PageBreadcrumb", () => {
  afterEach(() => {
    cleanup();
  });

  it("renders ancestor links and the current page", () => {
    render(
      <MemoryRouter>
        <PageBreadcrumb
          items={[
            { label: "Streaming" },
            { label: "Topics", to: "/streaming/topics" },
            { label: "blog.summarizer", mono: true },
          ]}
        />
      </MemoryRouter>,
    );

    expect(screen.getByRole("navigation", { name: "breadcrumb" })).toBeTruthy();
    expect(screen.getByText("Streaming")).toBeTruthy();
    expect(screen.getByRole("link", { name: "Topics" })).toHaveAttribute("href", "/streaming/topics");
    expect(screen.getByText("blog.summarizer")).toHaveAttribute("aria-current", "page");
    expect(screen.queryByRole("link", { name: "Consumers" })).toBeNull();
  });
});
