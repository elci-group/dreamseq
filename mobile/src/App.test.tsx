// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

const config = {
  harnesses: [],
  output_dir: "/tmp/output",
  anthologies_dir: "/tmp/anthologies",
  allow_remote_analysis: false,
  auto_approve_remote_analysis: false,
};

function mockInitialLoad() {
  invokeMock.mockImplementation((command: string) => {
    if (command === "load_config") return Promise.resolve(config);
    if (command === "list_anthologies") return Promise.resolve([]);
    if (command === "cloud_status") return Promise.resolve(null);
    return Promise.reject(new Error(`unexpected command: ${command}`));
  });
}

describe("Dreamseq mobile app", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("loads the local configuration and renders the empty home state", async () => {
    mockInitialLoad();
    render(<App />);

    expect(await screen.findByText("No anthologies yet.")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("load_config");
    expect(invokeMock).toHaveBeenCalledWith("list_anthologies");
    expect(invokeMock).toHaveBeenCalledWith("cloud_status");
  });

  it("persists remote-analysis consent from settings", async () => {
    mockInitialLoad();
    invokeMock.mockImplementation((command: string, args?: { config?: typeof config }) => {
      if (command === "load_config") return Promise.resolve(config);
      if (command === "list_anthologies") return Promise.resolve([]);
      if (command === "cloud_status") return Promise.resolve(null);
      if (command === "save_config") return Promise.resolve(args?.config);
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    render(<App />);

    await screen.findByText("No anthologies yet.");
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Allow remote analysis" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("save_config", {
        config: { ...config, allow_remote_analysis: true },
      }),
    );
    expect(screen.getByRole("checkbox", { name: "Allow remote analysis" })).toBeChecked();
  });

  it("surfaces startup failures instead of hiding them", async () => {
    invokeMock.mockRejectedValue(new Error("configuration unavailable"));
    render(<App />);

    expect(await screen.findByText("Error: configuration unavailable")).toBeInTheDocument();
  });
});
