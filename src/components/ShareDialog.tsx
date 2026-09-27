import { useEffect, useState } from "react";
import type { JSX, ReactNode } from "react";
import { Button, Callout, Dialog, Heading, RadioGroup, Text, TextField } from "@radix-ui/themes";
import { confirm } from "@tauri-apps/plugin-dialog";
import { invokeParsed, toAppError } from "../ipc/client";
import type { AppError } from "../ipc/client";
import { shareSchema, textSchema } from "../ipc/schemas";
import type { Share } from "../ipc/schemas";
import { DirectorySearch } from "./DirectorySearch";
import type { DirectoryPick } from "./DirectorySearch";
import { ErrorMessage } from "./Notice";
import { returnFocus } from "../lib/returnFocus";

/** What the Share dialog is about: a space (`ns`) or one package or bundle (`ns/id`). */
export type ShareTarget = Readonly<{ target: string; label: string }>;

type Visibility = Share["visibility"];

function isVisibility(value: string): value is Visibility {
  return value === "inherit" || value === "public" || value === "private";
}

/**
 * Who can find and install a space, package, or bundle: everyone, or its owners
 * and a list of people and teams. The list is kept when the setting changes, so
 * making something private again brings the same people back.
 */
export function ShareDialog({ request, onClose, onSaved }: Readonly<{ request: ShareTarget | null; onClose: () => void; onSaved: () => void }>): JSX.Element {
  const [share, setShare] = useState<Share | null>(null);
  const [visibility, setVisibility] = useState<Visibility>("inherit");
  const [people, setPeople] = useState<readonly Readonly<{ account: string; displayName: string }>[]>([]);
  const [teams, setTeams] = useState<readonly Readonly<{ namespace: string; displayName: string }>[]>([]);
  const [groups, setGroups] = useState<readonly string[]>([]);
  const [link, setLink] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const target = request?.target ?? null;

  useEffect(() => {
    if (target === null) {
      return undefined;
    }
    let stale = false;
    setShare(null);
    setError(null);
    setCopied(false);
    invokeParsed("get_share", shareSchema, { target })
      .then((loaded) => {
        if (stale) {
          return;
        }
        setShare(loaded);
        setVisibility(loaded.visibility);
        setPeople(loaded.users);
        setTeams(loaded.teams);
        setGroups(loaded.groups);
        setLink(loaded.link);
      })
      .catch((reason: unknown) => {
        if (!stale) {
          setError(toAppError(reason, "Couldn't load who can see this."));
        }
      });
    return () => {
      stale = true;
    };
  }, [target]);

  const space = target !== null && !target.includes("/");

  function add(pick: DirectoryPick): void {
    if (pick.kind === "team") {
      setTeams((current) => (current.some((team) => team.namespace === pick.namespace) ? current : [...current, pick]));
    } else {
      setPeople((current) => (current.some((person) => person.account.toLowerCase() === pick.account.toLowerCase()) ? current : [...current, pick]));
    }
  }

  async function save(): Promise<void> {
    if (target === null) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await invokeParsed("set_share", shareSchema, { target, visibility, users: people.map((person) => person.account), teams: teams.map((team) => team.namespace), groups });
      onSaved();
      onClose();
    } catch (reason) {
      setError(toAppError(reason, "Couldn't save who can see this."));
    } finally {
      setBusy(false);
    }
  }

  async function copyLink(reset: boolean): Promise<void> {
    if (target === null) {
      return;
    }
    if (
      reset &&
      !(await confirm("Make a new link? The old one stops working. People who already opened it keep access until you remove them.", {
        title: "Reset link",
        kind: "warning",
        okLabel: "Reset",
        cancelLabel: "Cancel"
      }))
    ) {
      return;
    }
    setError(null);
    try {
      const url = await invokeParsed("share_link", textSchema, { target, reset });
      setLink(url);
      await navigator.clipboard.writeText(url);
      setCopied(true);
    } catch (reason) {
      setError(toAppError(reason, "Couldn't make a share link."));
    }
  }

  const label = request?.label ?? "";
  return (
    <Dialog.Root
      open={request !== null}
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
    >
      <Dialog.Content maxWidth="560px" {...returnFocus}>
        <Dialog.Title>Share “{label}”</Dialog.Title>
        <Dialog.Description size="2">Choose who can find and install it. Sharing never lets anyone change it.</Dialog.Description>
        {error === null ? null : (
          <Callout.Root className="app-callout" color="red" role="alert">
            <ErrorMessage summary={error.summary} detail={error.detail} />
          </Callout.Root>
        )}
        {share === null ? (
          error === null ? (
            <Text as="p" color="gray" size="2">
              Loading…
            </Text>
          ) : null
        ) : (
          <>
            <GeneralAccess space={space} effective={share.effective} value={visibility} onChange={setVisibility} />
            <section className="manage-section">
              <Heading as="h3" size="2">
                People and teams
              </Heading>
              {visibility === "public" ? (
                <Text as="p" color="gray" size="1">
                  Everyone can see it now. This list is kept for if you make it private again.
                </Text>
              ) : null}
              <ul className="entry-list">
                {people.map((person) => (
                  <li key={person.account}>
                    <EntryRow name={person.displayName} detail={person.account}>
                      <RemoveButton
                        name={person.displayName}
                        onRemove={() => {
                          setPeople((current) => current.filter((entry) => entry.account !== person.account));
                        }}
                      />
                    </EntryRow>
                  </li>
                ))}
                {teams.map((team) => (
                  <li key={team.namespace}>
                    <EntryRow name={team.displayName} detail="Team">
                      <RemoveButton
                        name={team.displayName}
                        onRemove={() => {
                          setTeams((current) => current.filter((entry) => entry.namespace !== team.namespace));
                        }}
                      />
                    </EntryRow>
                  </li>
                ))}
                {groups.map((group) => (
                  <li key={group}>
                    <EntryRow name={group} detail="Windows group">
                      <RemoveButton
                        name={group}
                        onRemove={() => {
                          setGroups((current) => current.filter((entry) => entry !== group));
                        }}
                      />
                    </EntryRow>
                  </li>
                ))}
              </ul>
              <DirectorySearch teams placeholder="Add a person or team…" onPick={add} onError={setError} />
            </section>
            <LinkSection
              link={link}
              copied={copied}
              onCopy={(reset) => {
                copyLink(reset).catch((reason: unknown) => {
                  setError(toAppError(reason));
                });
              }}
            />
          </>
        )}
        <div className="dialog-actions">
          <Dialog.Close>
            <Button variant="soft" color="gray">
              Cancel
            </Button>
          </Dialog.Close>
          <Button
            loading={busy}
            disabled={busy || share === null}
            onClick={() => {
              save().catch((reason: unknown) => {
                setError(toAppError(reason));
              });
            }}
          >
            Save
          </Button>
        </div>
      </Dialog.Content>
    </Dialog.Root>
  );
}

function GeneralAccess({ space, effective, value, onChange }: Readonly<{ space: boolean; effective: Share["effective"]; value: Visibility; onChange: (value: Visibility) => void }>): JSX.Element {
  return (
    <section className="manage-section">
      <Heading as="h3" size="2">
        General access
      </Heading>
      <RadioGroup.Root
        className="share-options"
        value={value}
        onValueChange={(next) => {
          if (isVisibility(next)) {
            onChange(next);
          }
        }}
      >
        {space ? null : <RadioGroup.Item value="inherit">Same as its space (now {effective === "public" ? "everyone" : "private"})</RadioGroup.Item>}
        <RadioGroup.Item value="private">Private: only {space ? "its owners" : "the space's owners"} and the people below</RadioGroup.Item>
        <RadioGroup.Item value="public">Everyone at the company</RadioGroup.Item>
      </RadioGroup.Root>
    </section>
  );
}

function LinkSection({ link, copied, onCopy }: Readonly<{ link: string | null; copied: boolean; onCopy: (reset: boolean) => void }>): JSX.Element {
  return (
    <section className="manage-section">
      <Heading as="h3" size="2">
        Share link
      </Heading>
      <Text as="p" color="gray" size="1">
        Anyone at the company who opens it can install it, and is added to the list above.
      </Text>
      <div className="link-row">
        {link === null ? null : <TextField.Root className="link-field" readOnly value={link} aria-label="Share link" />}
        <Button
          size="1"
          variant="soft"
          onClick={() => {
            onCopy(false);
          }}
        >
          {copied ? "Copied" : "Copy link"}
        </Button>
        {link === null ? null : (
          <Button
            size="1"
            variant="soft"
            color="gray"
            onClick={() => {
              onCopy(true);
            }}
          >
            Reset
          </Button>
        )}
      </div>
    </section>
  );
}

/** One person, team, or group on a list, with what can be done about them at the end. */
export function EntryRow({ name, detail, badge, children }: Readonly<{ name: string; detail: string; badge?: string | undefined; children?: ReactNode }>): JSX.Element {
  return (
    <div className="entry-row">
      <div className="entry-copy">
        <Text size="2">
          {name}
          {badge === undefined ? null : (
            <Text color="gray" size="1">
              {" "}
              · {badge}
            </Text>
          )}
        </Text>
        <Text color="gray" size="1">
          {detail}
        </Text>
      </div>
      {children === undefined ? null : <div className="listed-source-actions">{children}</div>}
    </div>
  );
}

function RemoveButton({ name, onRemove }: Readonly<{ name: string; onRemove: () => void }>): JSX.Element {
  return (
    <Button size="1" variant="ghost" color="red" onClick={onRemove} aria-label={`Remove ${name}`}>
      Remove
    </Button>
  );
}
