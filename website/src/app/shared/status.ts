import type { PackageDetail, PackageVersion } from "../api";
import { newestFirst } from "../format";

export interface PackageStatus {
  readonly label: string;
  readonly tone: "live" | "pending" | "rejected";
  readonly detail: string;
  /** The badge's CSS classes. */
  readonly badge: string;
}

export interface VersionState {
  readonly label: string;
  readonly badge: string;
}

/** The newest version that has not been withdrawn, in whatever review state. */
export function newestVersion(detail: PackageDetail): PackageVersion | undefined {
  const [newest] = newestFirst(detail.versions.filter((version) => !version.yanked).map((version) => version.version));
  return detail.versions.find((version) => version.version === newest);
}

/** What a publisher needs to know about their package, in plain words. */
export function packageStatus(detail: PackageDetail): PackageStatus {
  const status = describe(detail);
  return { ...status, badge: `badge ${status.tone}` };
}

function describe(detail: PackageDetail): Omit<PackageStatus, "badge"> {
  const newest = newestVersion(detail);
  if (newest?.reviewState === "pending") {
    return {
      label: "Waiting for review",
      tone: "pending",
      detail:
        detail.liveVersion === null
          ? `An admin checks new skills before others can see them. Version ${newest.version} will go live once approved.`
          : `Version ${detail.liveVersion} stays live while an admin checks ${newest.version}.`
    };
  }

  if (newest?.reviewState === "rejected") {
    return { label: "Needs changes", tone: "rejected", detail: newest.reviewNote ?? "An admin asked for changes. Publish a new version to try again." };
  }

  return detail.liveVersion === null
    ? { label: "Not live", tone: "rejected", detail: "Every version was withdrawn. Publish a new version to share it again." }
    : { label: "Live", tone: "live", detail: `Version ${detail.liveVersion} is available to everyone who can see it.` };
}

export function versionState(version: PackageVersion): VersionState {
  if (version.yanked) {
    return { label: "Withdrawn", badge: "badge" };
  }

  switch (version.reviewState) {
    case "approved":
      return { label: "Approved", badge: "badge live" };
    case "pending":
      return { label: "Waiting for review", badge: "badge pending" };
    case "rejected":
      return { label: "Needs changes", badge: "badge rejected" };
  }
}
