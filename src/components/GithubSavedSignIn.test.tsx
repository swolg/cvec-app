import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { EMPTY_SNAPSHOT } from "@/test/fixtures";

const load = vi.hoisted(() => vi.fn());
vi.mock("@/lib/api", () => ({ loadSavedGithubAuth: load }));
import { GithubSavedSignIn } from "./GithubSavedSignIn";

beforeEach(() => vi.clearAllMocks());
const auth = { ...EMPTY_SNAPSHOT.githubAuth, configured: true, deferred: true };

it("loads saved credentials only after the explicit action", async () => {
  const result = { ...auth, deferred: false, signedIn: true, login: "fixture-user" };
  load.mockResolvedValue(result);
  const onChange = vi.fn();
  render(<GithubSavedSignIn auth={auth} onChange={onChange} onError={vi.fn()} />);
  expect(load).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Use saved sign-in" }));
  await waitFor(() => expect(onChange).toHaveBeenCalledWith(result));
  expect(load).toHaveBeenCalledOnce();
});

it("exposes an actual failure and supports explicit retry", async () => {
  load.mockRejectedValueOnce(new Error("Synthetic denial")).mockResolvedValueOnce({ ...auth, deferred: false });
  const onError = vi.fn();
  render(<GithubSavedSignIn auth={auth} onChange={vi.fn()} onError={onError} />);
  await userEvent.click(screen.getByRole("button", { name: "Use saved sign-in" }));
  await waitFor(() => expect(onError).toHaveBeenCalledWith("Synthetic denial"));
  await userEvent.click(screen.getByRole("button", { name: "Use saved sign-in" }));
  await waitFor(() => expect(load).toHaveBeenCalledTimes(2));
});

it("offers no credential action for isolated or already connected accounts", () => {
  const { rerender } = render(<GithubSavedSignIn auth={{ ...auth, configured: false }} onChange={vi.fn()} onError={vi.fn()} />);
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  rerender(<GithubSavedSignIn auth={{ ...auth, signedIn: true }} onChange={vi.fn()} onError={vi.fn()} />);
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(load).not.toHaveBeenCalled();
});
