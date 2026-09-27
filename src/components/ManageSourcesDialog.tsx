import type { JSX, ReactNode } from "react";
import { Button, Callout, Card, Dialog, Heading, Text } from "@radix-ui/themes";
import { toAppError } from "../ipc/client";
import type { AppError } from "../ipc/client";
import type { AppState, ListedSource, RepositoryState, SourceState } from "../ipc/schemas";
import { ErrorMessage } from "./Notice";
import { FreshnessBadge } from "./SourceGroup";
import type { SourceAction } from "./SourceGroup";
import { returnFocus } from "../lib/returnFocus";

function ListedSourceCard({ name, description, children }: Readonly<{ name: string; description: string; children: ReactNode }>): JSX.Element {
  return (
    <Card className="listed-source-card">
      <div className="listed-source-copy">
        <Text as="p" size="2">
          {name}
        </Text>
        <Text as="p" color="gray" size="2">
          {description}
        </Text>
      </div>
      <div className="listed-source-actions">{children}</div>
    </Card>
  );
}

function listedSourceKey(listed: ListedSource): string {
  return listed.sourceId ?? listed.url;
}

function sourceForListed(state: AppState, listed: ListedSource): SourceState | null {
  return state.sources.find((source) => listed.sourceId !== null && source.sourceId === listed.sourceId) ?? state.sources.find((source) => source.url === listed.url) ?? null;
}

function orphanSources(state: AppState): readonly SourceState[] {
  const listedUrls = new Set(state.repositories.flatMap((repository) => repository.sources.map((listed) => listed.url)));
  const listedIds = new Set(state.repositories.flatMap((repository) => repository.sources.flatMap((listed) => (listed.sourceId === null ? [] : [listed.sourceId]))));
  return state.sources.filter((source) => !listedUrls.has(source.url) && !listedIds.has(source.sourceId));
}

export function ManageSourcesDialog({
  open,
  state,
  adding,
  error,
  removing,
  onOpenChange,
  onAddListed,
  onRemove,
  onError
}: Readonly<{
  open: boolean;
  state: AppState | null;
  /** The URL of the listed source being added. */
  adding: string | null;
  error: AppError | null;
  removing: ReadonlyMap<string, SourceAction>;
  onOpenChange: (open: boolean) => void;
  onAddListed: (repository: RepositoryState, listed: ListedSource) => Promise<void>;
  onRemove: (source: SourceState) => Promise<void>;
  onError: (error: AppError) => void;
}>): JSX.Element {
  const repository = state?.repositories[0] ?? null;
  const extras = state === null ? [] : orphanSources(state);

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content maxWidth="720px" {...returnFocus}>
        <Dialog.Title>Manage sources</Dialog.Title>
        <Dialog.Description>Adding a source makes its packages available. Nothing is installed until you choose it.</Dialog.Description>
        {error === null ? null : (
          <Callout.Root className="app-callout" color="red" role="alert">
            <ErrorMessage summary={error.summary} detail={error.detail} />
          </Callout.Root>
        )}
        {state === null || repository === null ? (
          <Text as="p" color="gray" size="2">
            The source catalog is not configured yet. Once the company catalog URL is set, sources will appear here.
          </Text>
        ) : (
          <section className="manage-section">
            <div className="skill-title-row">
              <Heading as="h3" size="3">
                {repository.name}
              </Heading>
              <FreshnessBadge status={repository.status} refreshFailed={repository.refreshFailed} lastSuccessAt={repository.lastSuccessAtEpochSeconds} message={repository.message} />
            </div>
            <Text as="p" color="gray" size="2">
              {repository.description}
            </Text>
            {repository.sources.length === 0 ? (
              <Text as="p" color="gray" size="2">
                This catalog does not list any sources yet.
              </Text>
            ) : (
              <ul className="listed-sources">
                {repository.sources.map((listed) => {
                  const added = listed.alreadyAdded ? sourceForListed(state, listed) : null;
                  return (
                    <li key={listedSourceKey(listed)}>
                      <ListedSourceCard name={listed.name} description={listed.description}>
                        {listed.alreadyAdded ? (
                          added === null ? null : (
                            <Button
                              color="red"
                              size="1"
                              variant="soft"
                              loading={removing.has(added.sourceId)}
                              disabled={removing.has(added.sourceId)}
                              onClick={() => {
                                onRemove(added).catch((reason: unknown) => {
                                  onError(toAppError(reason));
                                });
                              }}
                            >
                              Remove
                            </Button>
                          )
                        ) : (
                          <Button
                            size="1"
                            disabled={adding !== null}
                            loading={adding === listed.url}
                            onClick={() => {
                              onAddListed(repository, listed).catch((reason: unknown) => {
                                onError(toAppError(reason));
                              });
                            }}
                          >
                            Add
                          </Button>
                        )}
                      </ListedSourceCard>
                    </li>
                  );
                })}
              </ul>
            )}
          </section>
        )}
        {extras.length === 0 ? null : (
          <section className="manage-section">
            <Heading as="h3" size="3">
              Other sources
            </Heading>
            <Text as="p" color="gray" size="2">
              These sources are no longer listed in the catalog.
            </Text>
            <ul className="listed-sources">
              {extras.map((source) => (
                <li key={source.sourceKey}>
                  <ListedSourceCard name={source.name} description={source.description}>
                    <Button
                      color="red"
                      size="1"
                      variant="soft"
                      loading={removing.has(source.sourceId)}
                      disabled={removing.has(source.sourceId)}
                      onClick={() => {
                        onRemove(source).catch((reason: unknown) => {
                          onError(toAppError(reason));
                        });
                      }}
                    >
                      Remove
                    </Button>
                  </ListedSourceCard>
                </li>
              ))}
            </ul>
          </section>
        )}
        <Text as="p" color="gray" size="2">
          Need a new source? Ask the catalog owner to add it.
        </Text>
        <div className="dialog-actions">
          <Dialog.Close>
            <Button variant="soft">Done</Button>
          </Dialog.Close>
        </div>
      </Dialog.Content>
    </Dialog.Root>
  );
}
