import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import TrainingCenter from "./TrainingCenter";

const rpc = vi.hoisted(() => ({
  trainingOverview: vi.fn(), trainingJobs: vi.fn(), trainingCandidates: vi.fn(), trainingHistory: vi.fn(),
  datasetExplorer: vi.fn(), trainingSettingsGet: vi.fn(), trainingBackend: vi.fn(), reviewCoverage: vi.fn(),
  dropsExplorer: vi.fn(), readinessStatus: vi.fn(), reviewIntegrity: vi.fn(), semanticTeacherStatus: vi.fn(),
  reviewPriority: vi.fn(), mlCollectionStatus: vi.fn(), trainingStart: vi.fn(),
  settingsGet: vi.fn(),
}));
vi.mock("../lib/ipc", () => ({ api: rpc }));
beforeEach(() => {
  vi.resetAllMocks();
  sessionStorage.clear();
  rpc.trainingOverview.mockResolvedValue({ dataset_version: 1, rows: 7500, sessions: 10, models: [], eligibility: [], backend_available: false });
  for (const name of ["trainingJobs", "trainingCandidates", "trainingHistory", "dropsExplorer", "readinessStatus"] as const) rpc[name].mockResolvedValue([]);
  rpc.mlCollectionStatus.mockResolvedValue({ trace_on: false });
  rpc.settingsGet.mockResolvedValue({ fishing: { trace: false } });
  rpc.reviewPriority.mockResolvedValue({ items: [], total_matching: 0 });
  rpc.semanticTeacherStatus.mockResolvedValue(null);
});
afterEach(cleanup);

it("shows the simple workflow without scanning advanced datasets or starting training", async () => {
  render(<TrainingCenter />);
  expect(await screen.findByText(/7500 samples/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Review samples" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Training progress" })).toBeVisible();
  expect(screen.queryByRole("navigation", { name: "Training sections" })).not.toBeInTheDocument();
  expect(rpc.reviewPriority).not.toHaveBeenCalled();
  expect(rpc.datasetExplorer).not.toHaveBeenCalled();
  expect(rpc.mlCollectionStatus).not.toHaveBeenCalled();
  expect(rpc.trainingStart).not.toHaveBeenCalled();
});

it("loads advanced details only when requested", async () => {
  render(<TrainingCenter />);
  await userEvent.click(screen.getByRole("button", { name: "Advanced tools" }));
  expect(screen.getByRole("navigation", { name: "Training sections" })).toBeVisible();
  await waitFor(() => expect(rpc.datasetExplorer).toHaveBeenCalledTimes(1));
  await userEvent.click(screen.getByRole("button", { name: "Hide advanced" }));
  expect(screen.queryByRole("navigation", { name: "Training sections" })).not.toBeInTheDocument();
});

it("opens the review queue from the primary workflow", async () => {
  render(<TrainingCenter />);
  await userEvent.click(screen.getByRole("button", { name: "Review samples" }));
  await waitFor(() => expect(rpc.reviewPriority).toHaveBeenCalledTimes(1));
});
