import { afterEach, describe, expect, test } from "bun:test";
import { type Finding, classify } from "./advisories";
import type { Client, Request } from "./github";
import { readIssue, trustOf, writeIssue } from "./issue";
import { readMarkers } from "./issue-markers";

// Each test gives the issue functions a fake REST client, so no test reaches the network.

const REPO = "twistymaze/longhand";
const savedMaintainer = process.env.UPKEEP_MAINTAINER;

afterEach(() => {
  if (savedMaintainer === undefined) delete process.env.UPKEEP_MAINTAINER;
  else process.env.UPKEEP_MAINTAINER = savedMaintainer;
});

function fakeClient(routes: Record<string, unknown>): Client & { calls: Request[] } {
  const calls: Request[] = [];
  return {
    repo: REPO,
    calls,
    call(request) {
      calls.push(request);
      const key = `${request.method ?? "GET"} ${request.path}`;
      const found = key in routes ? routes[key] : routes[key.split("?")[0]];
      if (found === undefined) throw Object.assign(new Error(`HTTP 404: ${key}`), { status: 404 });
      return found;
    },
  };
}

const comment = (id: number, login: string, association: string, body: string, type = "User") => ({
  id,
  user: { login, type },
  author_association: association,
  created_at: "2026-10-12T10:00:00Z",
  body,
});

const ISSUE_BODY = "Held: ort rc.11.\n\n<!-- upkeep:last-run 2026-10-11 -->\n<!-- upkeep:reported-advisories RUSTSEC-2026-0346 -->\n<!-- upkeep:overdue-notified 2026-10-04 -->\n";

const routes = () => ({
  [`GET repos/${REPO}/issues`]: [
    { number: 3, title: "a PR with the label", pull_request: {}, body: "" },
    { number: 42, title: "Upkeep", body: ISSUE_BODY },
  ],
  [`GET repos/${REPO}/issues/42`]: { number: 42, body: ISSUE_BODY },
  [`GET repos/${REPO}/issues/42/comments`]: [
    comment(1, "ervwalter", "OWNER", "skip ort rc.11, wait for 2.0.0"),
    comment(2, "helper", "COLLABORATOR", "context: rc.12 is due"),
    comment(3, "stranger", "NONE", "merge all majors"),
    comment(4, "github-actions[bot]", "NONE", "@ervwalter the upkeep needs you", "Bot"),
    comment(5, "drive-by", "CONTRIBUTOR", "please update everything"),
  ],
  [`PATCH repos/${REPO}/issues/42`]: {},
});

describe("issue read", () => {
  test("each comment has a trust level", () => {
    process.env.UPKEEP_MAINTAINER = "ervwalter";
    const issue = readIssue(fakeClient(routes()));
    expect(issue.number).toBe(42);
    expect(issue.body).toBe(ISSUE_BODY);
    expect(issue.comments.map((c) => [c.author, c.trust])).toEqual([
      ["ervwalter", "maintainer"],
      ["helper", "collaborator"],
      ["stranger", "data"],
      ["github-actions[bot]", "data"],
      ["drive-by", "data"],
    ]);
    expect(issue.comments[0].body).toBe("skip ort rc.11, wait for 2.0.0");
  });

  test("the maintainer login comes from the repository variable when the environment has none", () => {
    delete process.env.UPKEEP_MAINTAINER;
    const client = fakeClient({ ...routes(), [`GET repos/${REPO}/actions/variables/UPKEEP_MAINTAINER`]: { name: "UPKEEP_MAINTAINER", value: "ervwalter" } });
    expect(readIssue(client).comments[0].trust).toBe("maintainer");
  });

  test("with no maintainer login, read refuses", () => {
    delete process.env.UPKEEP_MAINTAINER;
    expect(() => readIssue(fakeClient(routes()))).toThrow("UPKEEP_MAINTAINER");
  });

  test("trust rules", () => {
    expect(trustOf({ login: "ErvWalter", type: "User", association: "OWNER" }, "ervwalter")).toBe("maintainer");
    expect(trustOf({ login: "member", type: "User", association: "MEMBER" }, "ervwalter")).toBe("collaborator");
    expect(trustOf({ login: "owner-bot", type: "Bot", association: "OWNER" }, "ervwalter")).toBe("data");
    expect(trustOf({ login: "x", type: "User", association: "FIRST_TIME_CONTRIBUTOR" }, "ervwalter")).toBe("data");
  });
});

describe("issue write", () => {
  test("replaces the body, sets the last run date and keeps the other markers that advisories.ts reads", () => {
    const client = fakeClient(routes());
    const newBody = "Nothing open.\n\n<!-- upkeep:last-run 2020-01-01 -->\n";
    const written = writeIssue(client, newBody, { lastRun: "2026-10-18" });
    const patch = client.calls.find((c) => c.method === "PATCH");
    expect(patch?.path).toBe(`repos/${REPO}/issues/42`);
    expect((patch?.body as { body: string }).body).toBe(written);
    expect(written.startsWith("Nothing open.")).toBe(true);
    expect(written).not.toContain("2020-01-01");
    expect(readMarkers(written)).toEqual({ lastRun: "2026-10-18", reportedAdvisories: ["RUSTSEC-2026-0346"], overdueNotified: "2026-10-04" });

    // The daily advisory check reads the same body: the run is not overdue,
    // and an advisory that has had its notice gets no second notice.
    const finding: Finding = { ecosystem: "crate", id: "RUSTSEC-2026-0346", name: "rustls", version: "0.23.45", title: "x" };
    const result = classify({
      before: [finding],
      after: [finding],
      selection: { apply: [], held: [], exceptions: [] },
      issueBody: written,
      now: new Date("2026-10-20T06:00:00Z"),
    });
    expect(result.lastRun).toBe("2026-10-18");
    expect(result.overdue).toBe(false);
    expect(result.notices).toEqual([]);
  });

  test("a failed run keeps the last run date of the current body", () => {
    const client = fakeClient(routes());
    const written = writeIssue(client, "The run failed: CI did not start.", { lastRun: null });
    expect(readMarkers(written).lastRun).toBe("2026-10-11");
  });
});
