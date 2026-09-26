//! `agent-plugins://` links, which the marketplace portal's Install buttons
//! open. A link names a catalog item and nothing else: the app always asks in
//! its own window before it changes anything, because any web page can open
//! one.

use serde::Serialize;
use std::sync::Mutex;

pub(crate) const SCHEME: &str = "agent-plugins";
/// Tells the window a link is waiting; it answers with `take_pending_link`.
pub(crate) const EVENT: &str = "deep-link";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DeepLinkKind {
    /// Install a package or bundle, or one component of a package.
    Install,
    /// Bring the window forward, scrolled to the item.
    Open,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeepLink {
    pub(crate) kind: DeepLinkKind,
    pub(crate) namespace: String,
    pub(crate) id: String,
    pub(crate) component: Option<String>,
}

/// Reads `agent-plugins://install/<ns>/<id>[/<component>]` or
/// `agent-plugins://open/<ns>/<id>`. Anything else, including percent-encoding,
/// a query, or a fragment, is refused rather than guessed at.
pub(crate) fn parse(url: &str) -> Option<DeepLink> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case(SCHEME)
        || rest
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || "%?#@:\\".contains(ch))
    {
        return None;
    }
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let segments = rest.split('/').collect::<Vec<_>>();
    let (kind, namespace, id, component) = match segments.as_slice() {
        [verb, namespace, id] => (*verb, *namespace, *id, None),
        [verb, namespace, id, component] if verb.eq_ignore_ascii_case("install") => {
            (*verb, *namespace, *id, Some(*component))
        }
        _ => return None,
    };
    let kind = if kind.eq_ignore_ascii_case("install") {
        DeepLinkKind::Install
    } else if kind.eq_ignore_ascii_case("open") {
        DeepLinkKind::Open
    } else {
        return None;
    };
    crate::manifest::validate_source_id(namespace).ok()?;
    crate::manifest::validate_package_id(id, "package").ok()?;
    if let Some(component) = component {
        crate::manifest::validate_package_id(component, "component").ok()?;
    }
    Some(DeepLink {
        kind,
        namespace: namespace.to_string(),
        id: id.to_string(),
        component: component.map(str::to_string),
    })
}

/// The first argument that is a link: Windows passes the URL as its own
/// argument, after the executable.
pub(crate) fn from_args<S: AsRef<str>>(args: impl IntoIterator<Item = S>) -> Option<DeepLink> {
    args.into_iter().find_map(|arg| parse(arg.as_ref()))
}

/// True when `arg` is meant as a link, valid or not, so the command line hands
/// it to the window instead of calling it an unknown command.
pub(crate) fn looks_like_link(arg: &str) -> bool {
    arg.get(..SCHEME.len() + 1)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&format!("{SCHEME}:")))
}

static PENDING: Mutex<Option<DeepLink>> = Mutex::new(None);

/// Holds the latest link until the window takes it. A newer link replaces one
/// that is still waiting, so a page cannot stack up dialogs.
pub(crate) fn set_pending(link: DeepLink) {
    *PENDING.lock().unwrap_or_else(|error| error.into_inner()) = Some(link);
}

pub(crate) fn take_pending() -> Option<DeepLink> {
    PENDING
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(kind: DeepLinkKind, namespace: &str, id: &str, component: Option<&str>) -> DeepLink {
        DeepLink {
            kind,
            namespace: namespace.to_string(),
            id: id.to_string(),
            component: component.map(str::to_string),
        }
    }

    #[test]
    fn reads_install_and_open_links() {
        use DeepLinkKind::{Install, Open};
        assert_eq!(
            parse("agent-plugins://install/data-team/review"),
            Some(link(Install, "data-team", "review", None))
        );
        assert_eq!(
            parse("agent-plugins://install/jacob/tools/sql-style/"),
            Some(link(Install, "jacob", "tools", Some("sql-style")))
        );
        assert_eq!(
            parse("Agent-Plugins://OPEN/jacob/review"),
            Some(link(Open, "jacob", "review", None))
        );
        assert_eq!(
            from_args([
                "C:\\Apps\\agent-plugins.exe",
                "agent-plugins://open/jacob/review"
            ]),
            Some(link(Open, "jacob", "review", None))
        );
    }

    #[test]
    fn refuses_anything_else() {
        for url in [
            "https://install/jacob/review",
            "agent-plugins:install/jacob/review",
            "agent-plugins://install/jacob",
            "agent-plugins://open/jacob/review/extra",
            "agent-plugins://install/jacob/review/a/b",
            "agent-plugins://remove/jacob/review",
            "agent-plugins://install/Jacob/review",
            "agent-plugins://install/j/review",
            "agent-plugins://install/jacob/re--view",
            "agent-plugins://install/jacob/%72eview",
            "agent-plugins://install/jacob/review?approve=1",
            "agent-plugins://install/jacob/review#x",
            "agent-plugins://user@install/jacob/review",
            "agent-plugins://install:80/jacob/review",
            "agent-plugins://install/jacob\\review",
            "agent-plugins://install/jacob/review ",
            "agent-plugins://install//review",
            "agent-plugins://install/jacob/review//",
        ] {
            assert_eq!(parse(url), None, "{url}");
        }
        assert!(looks_like_link("AGENT-PLUGINS://nonsense"));
        assert!(!looks_like_link("--background"));
        assert!(!looks_like_link("agent"));
    }

    #[test]
    fn a_newer_link_replaces_one_still_waiting() {
        set_pending(link(DeepLinkKind::Install, "jacob", "one", None));
        set_pending(link(DeepLinkKind::Install, "jacob", "two", None));
        assert_eq!(take_pending().map(|link| link.id), Some("two".to_string()));
        assert_eq!(take_pending(), None);
    }
}
