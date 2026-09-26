import { useCallback, useEffect, useState } from "react";
import type { JSX } from "react";
import { Badge, Button, Callout, Card, Dialog, Heading, RadioGroup, Text, TextField } from "@radix-ui/themes";
import { confirm } from "@tauri-apps/plugin-dialog";
import { invokeParsed, toAppError } from "../ipc/client";
import type { AppError } from "../ipc/client";
import { teamSchema, teamSummariesSchema, textSchema, unitSchema } from "../ipc/schemas";
import type { AppIdentity, Team, TeamSummary } from "../ipc/schemas";
import { NAMESPACE_PATTERN, suggestNamespace } from "../lib/marketplace";
import { DirectorySearch } from "./DirectorySearch";
import { ErrorMessage } from "./Notice";
import { EntryRow } from "./ShareDialog";

type View = Readonly<{ kind: "list" }> | Readonly<{ kind: "team"; namespace: string }> | Readonly<{ kind: "create" }>;

/**
 * Teams this person belongs to: who is in each, the invite link, and creating
 * one. `onChanged` runs after anything that changes where they may publish, so
 * the window picks it up.
 */
export function TeamsDialog({
  open,
  identity,
  onOpenChange,
  onChanged,
  onShare,
  onOpenLink
}: Readonly<{
  open: boolean;
  identity: AppIdentity | null;
  onOpenChange: (open: boolean) => void;
  onChanged: () => void;
  onShare: (target: string, label: string) => void;
  onOpenLink: () => void;
}>): JSX.Element {
  const [view, setView] = useState<View>({ kind: "list" });
  const [error, setError] = useState<AppError | null>(null);

  const show = useCallback((next: View): void => {
    setError(null);
    setView(next);
  }, []);

  return (
    <Dialog.Root
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          show({ kind: "list" });
        }
        onOpenChange(next);
      }}
    >
      <Dialog.Content maxWidth="640px">
        <Dialog.Title>Teams</Dialog.Title>
        <Dialog.Description size="2">Everyone in a team can publish skills to it and see its private skills.</Dialog.Description>
        {error === null ? null : (
          <Callout.Root className="app-callout" color="red" role="alert">
            <ErrorMessage summary={error.summary} detail={error.detail} />
          </Callout.Root>
        )}
        {!open ? null : view.kind === "list" ? (
          <TeamList onShow={show} onOpenLink={onOpenLink} onError={setError} />
        ) : view.kind === "create" ? (
          <CreateTeam
            onCreated={(namespace) => {
              onChanged();
              show({ kind: "team", namespace });
            }}
            onBack={() => {
              show({ kind: "list" });
            }}
            onError={setError}
          />
        ) : (
          <TeamDetail
            namespace={view.namespace}
            account={identity?.account ?? null}
            onBack={() => {
              show({ kind: "list" });
            }}
            onLeft={() => {
              onChanged();
              show({ kind: "list" });
            }}
            onShare={onShare}
            onError={setError}
          />
        )}
        <div className="dialog-actions">
          <Dialog.Close>
            <Button variant="soft">Done</Button>
          </Dialog.Close>
        </div>
      </Dialog.Content>
    </Dialog.Root>
  );
}

function roleLabel(role: TeamSummary["role"]): string {
  switch (role) {
    case "owner":
      return "Owner";
    case "member":
      return "Member";
    case "admin":
      return "Admin";
  }
}

function TeamList({ onShow, onOpenLink, onError }: Readonly<{ onShow: (view: View) => void; onOpenLink: () => void; onError: (error: AppError) => void }>): JSX.Element {
  const [teams, setTeams] = useState<readonly TeamSummary[] | null>(null);
  useEffect(() => {
    let stale = false;
    invokeParsed("list_teams", teamSummariesSchema)
      .then((loaded) => {
        if (!stale) {
          setTeams(loaded);
        }
      })
      .catch((reason: unknown) => {
        if (!stale) {
          onError(toAppError(reason, "Couldn't load your teams."));
        }
      });
    return () => {
      stale = true;
    };
  }, [onError]);
  return (
    <section className="manage-section">
      {teams === null ? (
        <Text as="p" color="gray" size="2">
          Loading…
        </Text>
      ) : teams.length === 0 ? (
        <Text as="p" color="gray" size="2">
          You're not in any teams yet. Create one, or open an invite link someone sent you.
        </Text>
      ) : (
        <ul className="listed-sources">
          {teams.map((team) => (
            <li key={team.namespace}>
              <Card className="listed-source-card">
                <div className="listed-source-copy">
                  <div className="skill-title-row">
                    <Text size="2">{team.displayName}</Text>
                    <Badge color="gray">{roleLabel(team.role)}</Badge>
                    {team.visibility === "private" ? <Badge color="orange">Private</Badge> : null}
                  </div>
                  <Text as="p" color="gray" size="1">
                    {team.namespace} · {String(team.memberCount)} member{team.memberCount === 1 ? "" : "s"}
                  </Text>
                </div>
                <div className="listed-source-actions">
                  <Button
                    size="1"
                    variant="soft"
                    onClick={() => {
                      onShow({ kind: "team", namespace: team.namespace });
                    }}
                  >
                    Open
                  </Button>
                </div>
              </Card>
            </li>
          ))}
        </ul>
      )}
      <div className="dialog-actions">
        <Button variant="soft" color="gray" onClick={onOpenLink}>
          Open a link
        </Button>
        <Button
          onClick={() => {
            onShow({ kind: "create" });
          }}
        >
          Create a team
        </Button>
      </div>
    </section>
  );
}

function CreateTeam({ onCreated, onBack, onError }: Readonly<{ onCreated: (namespace: string) => void; onBack: () => void; onError: (error: AppError) => void }>): JSX.Element {
  const [displayName, setDisplayName] = useState("");
  // The space name follows the display name until the person edits it themselves.
  const [namespace, setNamespace] = useState<string | null>(null);
  const [visibility, setVisibility] = useState<"private" | "public">("private");
  const [busy, setBusy] = useState(false);
  const name = namespace ?? suggestNamespace(displayName);
  const nameValid = NAMESPACE_PATTERN.test(name);
  const ready = displayName.trim().length > 0 && nameValid && !busy;

  async function create(): Promise<void> {
    setBusy(true);
    try {
      await invokeParsed("create_team", teamSchema, { namespace: name, displayName: displayName.trim(), visibility });
      onCreated(name);
    } catch (reason) {
      onError(toAppError(reason, "Couldn't create the team."));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="manage-section form-grid">
      <label className="form-field">
        <Text size="2">Team name</Text>
        <TextField.Root
          placeholder="Data Team"
          value={displayName}
          maxLength={120}
          onChange={(event) => {
            setDisplayName(event.currentTarget.value);
          }}
        />
      </label>
      <label className="form-field">
        <Text size="2">Short name</Text>
        <TextField.Root
          value={name}
          maxLength={16}
          onChange={(event) => {
            setNamespace(event.currentTarget.value.toLowerCase());
          }}
        />
        <Text color={name.length === 0 || nameValid ? "gray" : "red"} size="1">
          Goes in front of the team's skill names and can't change later. 2–16 lowercase letters, numbers, or hyphens, starting with a letter.
        </Text>
      </label>
      <div className="form-field">
        <Text size="2">Who can see the team's skills?</Text>
        <RadioGroup.Root
          value={visibility}
          onValueChange={(value) => {
            setVisibility(value === "public" ? "public" : "private");
          }}
        >
          <RadioGroup.Item value="private">Only team members, and people you share with</RadioGroup.Item>
          <RadioGroup.Item value="public">Everyone at the company</RadioGroup.Item>
        </RadioGroup.Root>
      </div>
      <div className="dialog-actions">
        <Button variant="soft" color="gray" onClick={onBack}>
          Back
        </Button>
        <Button
          loading={busy}
          disabled={!ready}
          onClick={() => {
            create().catch((reason: unknown) => {
              onError(toAppError(reason));
            });
          }}
        >
          Create team
        </Button>
      </div>
    </section>
  );
}

function TeamDetail({
  namespace,
  account,
  onBack,
  onLeft,
  onShare,
  onError
}: Readonly<{ namespace: string; account: string | null; onBack: () => void; onLeft: () => void; onShare: (target: string, label: string) => void; onError: (error: AppError) => void }>): JSX.Element {
  const [team, setTeam] = useState<Team | null>(null);
  const [rename, setRename] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let stale = false;
    invokeParsed("get_team", teamSchema, { namespace })
      .then((loaded) => {
        if (!stale) {
          setTeam(loaded);
        }
      })
      .catch((reason: unknown) => {
        if (!stale) {
          onError(toAppError(reason, "Couldn't load the team."));
        }
      });
    return () => {
      stale = true;
    };
  }, [namespace, onError]);

  /** Runs one change, showing its failure in the dialog; `next` replaces the shown team when it returns one. */
  async function change(context: string, task: () => Promise<Team | null>): Promise<void> {
    setBusy(true);
    try {
      const next = await task();
      if (next !== null) {
        setTeam(next);
      }
    } catch (reason) {
      onError(toAppError(reason, context));
    } finally {
      setBusy(false);
    }
  }

  if (team === null) {
    return (
      <Text as="p" color="gray" size="2">
        Loading…
      </Text>
    );
  }
  const owner = team.role === "owner" || team.role === "admin";
  const reload = async (): Promise<Team> => invokeParsed("get_team", teamSchema, { namespace });

  async function copyInvite(reset: boolean): Promise<void> {
    if (
      reset &&
      !(await confirm("Make a new invite link? The old one stops working. Everyone already in the team stays.", {
        title: "Reset invite link",
        kind: "warning",
        okLabel: "Reset",
        cancelLabel: "Cancel"
      }))
    ) {
      return;
    }
    await change("Couldn't make an invite link.", async () => {
      const url = await invokeParsed("team_invite", textSchema, { namespace, reset });
      await navigator.clipboard.writeText(url);
      setCopied(true);
      return reload();
    });
  }

  async function leave(): Promise<void> {
    if (
      account === null ||
      !(await confirm(`Leave ${team?.displayName ?? namespace}? You won't be able to publish there, and its private skills disappear from your list.`, {
        title: "Leave team",
        kind: "warning",
        okLabel: "Leave",
        cancelLabel: "Cancel"
      }))
    ) {
      return;
    }
    await change("Couldn't leave the team.", async () => {
      await invokeParsed("remove_team_member", unitSchema, { namespace, account });
      onLeft();
      return null;
    });
  }

  async function remove(): Promise<void> {
    if (
      !(await confirm(`Delete ${team?.displayName ?? namespace}? This only works while it has no skills or bundles.`, {
        title: "Delete team",
        kind: "warning",
        okLabel: "Delete",
        cancelLabel: "Cancel"
      }))
    ) {
      return;
    }
    await change("Couldn't delete the team.", async () => {
      await invokeParsed("delete_team", unitSchema, { namespace });
      onLeft();
      return null;
    });
  }

  const run = (task: Promise<void>): void => {
    task.catch((reason: unknown) => {
      onError(toAppError(reason));
    });
  };

  return (
    <section className="manage-section form-grid">
      <div className="skill-title-row">
        {rename === null ? (
          <Heading as="h3" size="4">
            {team.displayName}
          </Heading>
        ) : (
          <TextField.Root
            className="link-field"
            value={rename}
            maxLength={120}
            aria-label="Team name"
            onChange={(event) => {
              setRename(event.currentTarget.value);
            }}
          />
        )}
        {team.visibility === "private" ? <Badge color="orange">Private</Badge> : <Badge color="gray">Everyone can see</Badge>}
        {!owner ? null : rename === null ? (
          <Button
            size="1"
            variant="ghost"
            onClick={() => {
              setRename(team.displayName);
            }}
          >
            Rename
          </Button>
        ) : (
          <Button
            size="1"
            disabled={busy || rename.trim().length === 0}
            onClick={() => {
              const displayName = rename.trim();
              setRename(null);
              run(change("Couldn't rename the team.", () => invokeParsed("rename_team", teamSchema, { namespace, displayName })));
            }}
          >
            Save
          </Button>
        )}
      </div>
      <Text as="p" color="gray" size="1">
        {team.namespace}
      </Text>
      <div className="link-row">
        <Button
          size="1"
          variant="soft"
          color="gray"
          onClick={() => {
            onShare(team.namespace, team.displayName);
          }}
        >
          Who can see its skills…
        </Button>
      </div>

      <Heading as="h3" size="2">
        Members
      </Heading>
      <ul className="entry-list">
        {team.members.map((member) => (
          <li key={member.account}>
            <EntryRow name={member.displayName} detail={member.account} badge={member.owner ? "Owner" : undefined}>
              {owner ? (
                <>
                  <Button
                    size="1"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => {
                      run(change("Couldn't change the member.", () => invokeParsed("add_team_member", teamSchema, { namespace, account: member.account, owner: !member.owner })));
                    }}
                  >
                    {member.owner ? "Make member" : "Make owner"}
                  </Button>
                  {member.account === account ? null : (
                    <Button
                      size="1"
                      variant="ghost"
                      color="red"
                      disabled={busy}
                      onClick={() => {
                        run(
                          change("Couldn't remove the member.", async () => {
                            await invokeParsed("remove_team_member", unitSchema, { namespace, account: member.account });
                            return reload();
                          })
                        );
                      }}
                    >
                      Remove
                    </Button>
                  )}
                </>
              ) : undefined}
            </EntryRow>
          </li>
        ))}
      </ul>
      {owner ? (
        <DirectorySearch
          teams={false}
          placeholder="Add people by name or account…"
          onPick={(pick) => {
            if (pick.kind === "person") {
              run(change("Couldn't add them.", () => invokeParsed("add_team_member", teamSchema, { namespace, account: pick.account, owner: false })));
            }
          }}
          onError={onError}
        />
      ) : null}

      <Heading as="h3" size="2">
        Invite link
      </Heading>
      <Text as="p" color="gray" size="1">
        Anyone at the company who opens it can join and publish here.
      </Text>
      <div className="link-row">
        {team.invite === null ? null : <TextField.Root className="link-field" readOnly value={team.invite} aria-label="Invite link" />}
        <Button
          size="1"
          variant="soft"
          disabled={busy}
          onClick={() => {
            run(copyInvite(false));
          }}
        >
          {copied ? "Copied" : "Copy invite link"}
        </Button>
        {owner && team.invite !== null ? (
          <Button
            size="1"
            variant="soft"
            color="gray"
            disabled={busy}
            onClick={() => {
              run(copyInvite(true));
            }}
          >
            Reset
          </Button>
        ) : null}
      </div>

      <div className="dialog-actions">
        <Button variant="soft" color="gray" onClick={onBack}>
          Back
        </Button>
        {team.role === "admin" ? null : (
          <Button
            variant="soft"
            color="red"
            disabled={busy}
            onClick={() => {
              run(leave());
            }}
          >
            Leave team
          </Button>
        )}
        {owner ? (
          <Button
            variant="soft"
            color="red"
            disabled={busy}
            onClick={() => {
              run(remove());
            }}
          >
            Delete team
          </Button>
        ) : null}
      </div>
    </section>
  );
}
