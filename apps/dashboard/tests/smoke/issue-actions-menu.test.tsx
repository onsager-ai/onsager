import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { IssueActionsMenu } from "@/components/IssueActionsMenu";
import { api } from "@/lib/api";

function renderWith(ui: React.ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return {
    queryClient,
    ...render(
      <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
    ),
  };
}

describe("IssueActionsMenu", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("opens the kebab and shows the three actions for a hydrated row", async () => {
    const user = userEvent.setup();
    renderWith(
      <IssueActionsMenu
        projectId="proj_1"
        issueNumber={42}
        htmlUrl="https://github.com/acme/widgets/issues/42"
        listQueryKey={["project-issues", "proj_1", "open"]}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Issue actions" }));

    expect(
      await screen.findByText("Refresh this issue"),
    ).toBeInTheDocument();
    expect(screen.getByText("Replay trigger…")).toBeInTheDocument();
    expect(screen.getByText("Open in GitHub")).toBeInTheDocument();
  });

  it("disables Replay when there is no project context", async () => {
    const user = userEvent.setup();
    renderWith(
      <IssueActionsMenu
        projectId={null}
        issueNumber={null}
        htmlUrl={null}
        listQueryKey={["project-issues", null, "open"]}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Issue actions" }));
    const item = await screen.findByText("Replay trigger…");
    expect(item.closest('[role="menuitem"]')).toHaveAttribute("aria-disabled");
  });

  it("Refresh invalidates the issue list query", async () => {
    const user = userEvent.setup();
    const { queryClient } = renderWith(
      <IssueActionsMenu
        projectId="proj_1"
        issueNumber={42}
        htmlUrl={null}
        listQueryKey={["project-issues", "proj_1", "open"]}
      />,
    );
    const spy = vi.spyOn(queryClient, "invalidateQueries");

    await user.click(screen.getByRole("button", { name: "Issue actions" }));
    await user.click(await screen.findByText("Refresh this issue"));

    expect(spy).toHaveBeenCalledWith({
      queryKey: ["project-issues", "proj_1", "open"],
    });
  });

  it("Replay shows the dry-run preview before firing", async () => {
    const user = userEvent.setup();
    const previewSpy = vi
      .spyOn(api, "replayIssueTrigger")
      .mockResolvedValueOnce({
        project_id: "proj_1",
        issue_number: 42,
        dry_run: true,
        matches: [
          {
            workflow_id: "wf_1",
            workflow_name: "sdd",
            label: "spec",
          },
        ],
        event_ids: [],
      });

    renderWith(
      <IssueActionsMenu
        projectId="proj_1"
        issueNumber={42}
        htmlUrl={null}
        listQueryKey={["project-issues", "proj_1", "open"]}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Issue actions" }));
    await user.click(await screen.findByText("Replay trigger…"));

    // Server is called with dry_run=true first.
    await waitFor(() => {
      expect(previewSpy).toHaveBeenCalledWith("proj_1", 42, { dry_run: true });
    });
    expect(await screen.findByText(/Will fire 1 workflow/)).toBeInTheDocument();
    expect(screen.getByText("sdd")).toBeInTheDocument();
  });

  it("Replay reports zero matches without enabling Fire", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "replayIssueTrigger").mockResolvedValueOnce({
      project_id: "proj_1",
      issue_number: 42,
      dry_run: true,
      matches: [],
      event_ids: [],
    });

    renderWith(
      <IssueActionsMenu
        projectId="proj_1"
        issueNumber={42}
        htmlUrl={null}
        listQueryKey={["project-issues", "proj_1", "open"]}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Issue actions" }));
    await user.click(await screen.findByText("Replay trigger…"));

    expect(
      await screen.findByText(/No active workflows match/),
    ).toBeInTheDocument();
    const fireButton = screen.getByRole("button", { name: "Fire trigger" });
    expect(fireButton).toBeDisabled();
  });
});
