import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import type { SystemInfo } from "./contracts";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const mockSystemInfo: SystemInfo = {
  os: "windows",
  architecture: "x86_64",
  hostname: "ATLAS-PC",
  appVersion: "0.1.0",
};

const mockedInvoke = vi.mocked(invoke);

describe("ATLAS App", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows the initial connection button", () => {
    render(<App />);

    expect(
      screen.getByRole("button", {
        name: /connect to this system/i,
      })
    ).toBeInTheDocument();
  });

  it("displays system information after a successful connection", async () => {
    mockedInvoke.mockImplementation(
      async () => mockSystemInfo as never
    );

    render(<App />);

    fireEvent.click(
      screen.getByRole("button", {
        name: /connect to this system/i,
      })
    );

    expect(await screen.findByText("windows")).toBeInTheDocument();
    expect(screen.getByText("x86_64")).toBeInTheDocument();
    expect(screen.getByText("ATLAS-PC")).toBeInTheDocument();
    expect(screen.getByText("0.1.0")).toBeInTheDocument();

    expect(mockedInvoke).toHaveBeenCalledWith("get_system_info");
  });

  it("displays an error when the connection fails", async () => {
    mockedInvoke.mockRejectedValueOnce(
      new Error("Connection failed")
    );

    render(<App />);

    fireEvent.click(
      screen.getByRole("button", {
        name: /connect to this system/i,
      })
    );

    expect(
      await screen.findByRole("alert")
    ).toHaveTextContent("Connection failed");
  });
  it("shows loading state while the connection is pending", async () => {
    let resolveRequest!: (value: SystemInfo) => void;

    mockedInvoke.mockImplementation(
        () =>
        new Promise<SystemInfo>((resolve) => {
         resolveRequest = resolve;
        })
    );

    render(<App />);

    const button = screen.getByRole("button", {
        name: /connect to this system/i,
    });

    fireEvent.click(button);

    // Verify loading state
    expect(button).toHaveTextContent("Connecting…");
    expect(button).toBeDisabled();

    // Complete the simulated IPC request
    resolveRequest(mockSystemInfo);

    // Verify loading state has ended
    expect(await screen.findByText("ATLAS-PC")).toBeInTheDocument();
    expect(button).toBeEnabled();
    });
});