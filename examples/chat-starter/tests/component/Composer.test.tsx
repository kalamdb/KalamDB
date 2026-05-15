import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Composer } from "../../src/components/Composer";

describe("Composer", () => {
  it("shows Send when idle and disables it on empty input", () => {
    render(<Composer onSend={vi.fn()} onStop={vi.fn()} isStreaming={false} canStop={false} />);
    const send = screen.getByRole("button", { name: "Send" });
    expect(send).toBeInTheDocument();
    expect(send).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });

  it("enables Send once the user types", async () => {
    const user = userEvent.setup();
    render(<Composer onSend={vi.fn()} onStop={vi.fn()} isStreaming={false} canStop={false} />);
    await user.type(screen.getByPlaceholderText(/Message KalamDB Chat/i), "hi");
    expect(screen.getByRole("button", { name: "Send" })).toBeEnabled();
  });

  it("swaps Send for Stop while streaming", () => {
    render(<Composer onSend={vi.fn()} onStop={vi.fn()} isStreaming={true} canStop={true} />);
    expect(screen.getByRole("button", { name: "Stop" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
  });

  it("disables Stop when canStop is false (e.g. after click, agent finalizing)", () => {
    render(<Composer onSend={vi.fn()} onStop={vi.fn()} isStreaming={true} canStop={false} />);
    expect(screen.getByRole("button", { name: "Stop" })).toBeDisabled();
  });

  it("calls onSend with the trimmed body and clears the input", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<Composer onSend={onSend} onStop={vi.fn()} isStreaming={false} canStop={false} />);
    const ta = screen.getByPlaceholderText(/Message KalamDB Chat/i) as HTMLTextAreaElement;
    await user.type(ta, "  hello world  ");
    await user.click(screen.getByRole("button", { name: "Send" }));
    expect(onSend).toHaveBeenCalledWith("hello world");
  });

  it("calls onStop when Stop is clicked", async () => {
    const user = userEvent.setup();
    const onStop = vi.fn().mockResolvedValue(undefined);
    render(<Composer onSend={vi.fn()} onStop={onStop} isStreaming={true} canStop={true} />);
    await user.click(screen.getByRole("button", { name: "Stop" }));
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it("submits on Enter (without shift)", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<Composer onSend={onSend} onStop={vi.fn()} isStreaming={false} canStop={false} />);
    const ta = screen.getByPlaceholderText(/Message KalamDB Chat/i);
    await user.type(ta, "hi{Enter}");
    expect(onSend).toHaveBeenCalledWith("hi");
  });

  it("inserts a newline on Shift+Enter (does not submit)", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    render(<Composer onSend={onSend} onStop={vi.fn()} isStreaming={false} canStop={false} />);
    const ta = screen.getByPlaceholderText(/Message KalamDB Chat/i);
    await user.type(ta, "line one{Shift>}{Enter}{/Shift}line two");
    expect(onSend).not.toHaveBeenCalled();
  });
});
