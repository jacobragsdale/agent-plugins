import { useEffect, useState } from "react";
import type { JSX } from "react";
import { Button, Text, TextField } from "@radix-ui/themes";
import { invokeParsed, toAppError } from "../ipc/client";
import type { AppError } from "../ipc/client";
import { directorySchema } from "../ipc/schemas";
import type { Directory } from "../ipc/schemas";
import { typedAccount } from "../lib/marketplace";

/** Someone or a team picked from the directory, or a Windows account typed in full. */
export type DirectoryPick = Readonly<{ kind: "person"; account: string; displayName: string }> | Readonly<{ kind: "team"; namespace: string; displayName: string }>;

/** Finds people (and teams, when `teams`) as the person types, and hands back the one they choose. */
export function DirectorySearch({
  teams,
  placeholder,
  onPick,
  onError
}: Readonly<{ teams: boolean; placeholder: string; onPick: (pick: DirectoryPick) => void; onError: (error: AppError) => void }>): JSX.Element {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Directory | null>(null);

  useEffect(() => {
    const text = query.trim();
    if (text.length === 0) {
      setResults(null);
      return undefined;
    }
    let stale = false;
    // Wait for a pause in typing, so each keystroke does not cost a request.
    const timer = setTimeout(() => {
      invokeParsed("search_directory", directorySchema, { query: text })
        .then((found) => {
          if (!stale) {
            setResults(found);
          }
        })
        .catch((reason: unknown) => {
          if (!stale) {
            onError(toAppError(reason, "Couldn't search for people."));
          }
        });
    }, 250);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [query, onError]);

  const pick = (chosen: DirectoryPick): void => {
    onPick(chosen);
    setQuery("");
  };
  const account = typedAccount(query);
  const people = results?.people ?? [];
  const found = teams ? (results?.teams ?? []) : [];
  const empty = results !== null && people.length === 0 && found.length === 0 && account === null;
  return (
    <div className="directory-search">
      <TextField.Root
        placeholder={placeholder}
        value={query}
        aria-label={placeholder}
        onChange={(event) => {
          setQuery(event.currentTarget.value);
        }}
      />
      <ul className="directory-results">
        {people.map((person) => (
          <li key={person.account}>
            <DirectoryRow
              name={person.displayName}
              detail={person.account}
              onAdd={() => {
                pick({ kind: "person", ...person });
              }}
            />
          </li>
        ))}
        {found.map((team) => (
          <li key={team.namespace}>
            <DirectoryRow
              name={team.displayName}
              detail="Team"
              onAdd={() => {
                pick({ kind: "team", ...team });
              }}
            />
          </li>
        ))}
        {account === null || people.some((person) => person.account.toLowerCase() === account.toLowerCase()) ? null : (
          <li>
            <DirectoryRow
              name={account}
              detail="Windows account"
              onAdd={() => {
                pick({ kind: "person", account, displayName: account });
              }}
            />
          </li>
        )}
      </ul>
      {empty ? (
        <Text as="p" color="gray" size="1">
          No one found. Type their full Windows account, like CORP\jane, to add them anyway.
        </Text>
      ) : null}
    </div>
  );
}

function DirectoryRow({ name, detail, onAdd }: Readonly<{ name: string; detail: string; onAdd: () => void }>): JSX.Element {
  return (
    <div className="entry-row">
      <div className="entry-copy">
        <Text size="2">{name}</Text>
        <Text color="gray" size="1">
          {detail}
        </Text>
      </div>
      <Button size="1" variant="soft" onClick={onAdd}>
        Add
      </Button>
    </div>
  );
}
