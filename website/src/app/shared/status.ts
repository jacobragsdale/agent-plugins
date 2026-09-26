import type { PackageDetail } from "../api";

export interface PackageStatus {
  readonly label: string;
  readonly tone: "live" | "pending" | "rejected";
  readonly detail: string;
  /** The badge's CSS classes. */
  readonly badge: string;
}

/** What a publisher needs to know about their package, in plain words. */
export function packageStatus(detail: PackageDetail): PackageStatus {
  const status = describe(detail);
  return { ...status, badge: `badge ${status.tone}` };
}

function describe(detail: PackageDetail): Omit<PackageStatus, "badge"> {
  if (detail.revoked) {
    return { label: "Removed from every PC", tone: "rejected", detail: "Nobody can install it, and each PC that had it removes it at its next check. Restore it to offer it again." };
  }

  if (detail.liveVersion === null) {
    return { label: "Not live", tone: "rejected", detail: "Every version was withdrawn. Restore one or publish a new version to share it again." };
  }

  const review = detail.publicReview;
  if (detail.effective === "public" && review?.state === "waiting") {
    return {
      label: "Waiting for an admin",
      tone: "pending",
      detail: "Its MCP server runs a program on people's PCs, so an admin checks it before everyone can see it. You, your team, and people you share it with can use it now."
    };
  }

  if (detail.effective === "public" && review?.state === "declined") {
    return { label: "Not shown to everyone", tone: "rejected", detail: review.note ?? "An admin didn't approve its MCP server for everyone. Publishing a new version asks again." };
  }

  const who = detail.effective === "public" ? "everyone" : "the people it's shared with";
  return { label: "Live", tone: "live", detail: `Version ${detail.liveVersion} is available to ${who}.` };
}
