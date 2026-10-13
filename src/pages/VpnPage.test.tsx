import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { VpnStatus } from "../lib/types";
import VpnPage from "./VpnPage";

const rpc = vi.hoisted(() => ({
  vpnGetStatus: vi.fn(), vpnConnect: vi.fn(), vpnDisconnect: vi.fn(), vpnTestPing: vi.fn(),
  vpnGetLogs: vi.fn(), vpnSetAutoReconnect: vi.fn(), vpnResetNetwork: vi.fn(),
}));
vi.mock("../lib/ipc", () => ({ api: rpc }));
let current: VpnStatus;
beforeEach(() => {
  vi.resetAllMocks();
  current = {
    connected: false, state: "disconnected", managed: false, engine: "none", engine_name: "Offline",
    ip: "", country: "", city: "", latency_ms: null, uptime_secs: 0, auto_reconnect: true,
    last_error: null, process_running: false, tunnel_detected: false, verification_detail: "",
    germany_verified: false, warp_verified: false, administrator: false, recovery_active: false,
  };
  rpc.vpnGetStatus.mockImplementation(async () => current);
  rpc.vpnTestPing.mockResolvedValue({ success: true, latency_ms: 63, target: "Frankfurt", error: null });
});
afterEach(cleanup);

it("defaults to the German relay and sends a verified connect request", async () => {
  rpc.vpnConnect.mockImplementation(async () => {
    current = { ...current, state: "connected", connected: true, managed: true, engine: "dedicated", country: "DE", germany_verified: true, administrator: true, recovery_active: true };
    return current;
  });
  render(<VpnPage />);
  expect(screen.getByRole("radio", { name: "Germany · Frankfurt relay" })).toBeChecked();
  await userEvent.click(screen.getByRole("button", { name: "Connect selected route" }));
  expect(rpc.vpnConnect).toHaveBeenCalledWith("dedicated");
  expect(await screen.findByText("German exit verified")).toBeVisible();
  expect(screen.getByText("Monitoring")).toBeVisible();
  expect(rpc.vpnDisconnect).not.toHaveBeenCalled();
});

it("does not treat a country string or a stale flag as German exit proof", async () => {
  current = { ...current, state: "connected", connected: true, engine: "dedicated", country: "DE", germany_verified: false };
  const { rerender } = render(<VpnPage />);
  expect(await screen.findByText("Tunnel connected")).toBeVisible();
  expect(screen.queryByText("German exit verified")).not.toBeInTheDocument();
  current = { ...current, state: "disconnected", connected: false, germany_verified: true };
  await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
  rerender(<VpnPage />);
  expect(screen.queryByText("German exit verified")).not.toBeInTheDocument();
});

it("keeps full actionable connection errors visible after the status refresh", async () => {
  const message = "Administrator rights required: restart the macro as Administrator to start the German TUN driver. The existing connection has been left running.";
  rpc.vpnConnect.mockRejectedValue(message);
  render(<VpnPage />);
  await userEvent.click(screen.getByRole("button", { name: "Connect selected route" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(message);
  await waitFor(() => expect(rpc.vpnGetStatus).toHaveBeenCalled());
  expect(screen.getByRole("alert")).toHaveTextContent(message);
});

it("adopts an external relay without issuing disconnect and explains missing elevation", async () => {
  current = { ...current, state: "connected", connected: true, engine: "dedicated", country: "DE", germany_verified: true };
  rpc.vpnConnect.mockImplementation(async () => (current = { ...current, managed: true }));
  render(<VpnPage />);
  await userEvent.click(await screen.findByRole("button", { name: "Use existing tunnel" }));
  expect(rpc.vpnConnect).toHaveBeenCalledWith("dedicated");
  expect(rpc.vpnDisconnect).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Disconnect" })).toBeDisabled();
  expect(screen.getByText(/Run the macro as Administrator to restart or stop this relay/)).toBeVisible();
});

it("warns about an Egyptian WARP exit without claiming German protection", async () => {
  current = { ...current, state: "connected", connected: true, engine: "warp", country: "EG", warp_verified: true };
  render(<VpnPage />);
  expect(await screen.findByText("Egyptian exit detected")).toBeVisible();
  expect(screen.getByText("WARP observed")).toBeVisible();
  expect(screen.queryByText("German exit verified")).not.toBeInTheDocument();
});

it("removes stale German proof when status requests fail and restores it after recovery", async () => {
  current = { ...current, state: "connected", connected: true, engine: "dedicated", country: "DE", germany_verified: true };
  render(<VpnPage />);
  expect(await screen.findByText("German exit verified")).toBeVisible();
  rpc.vpnGetStatus.mockRejectedValueOnce("IPC unavailable");
  await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("VPN status unavailable");
  expect(screen.queryByText("German exit verified")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
  expect(await screen.findByText("German exit verified")).toBeVisible();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});
