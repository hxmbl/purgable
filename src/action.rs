//! The actions a user can choose for a reviewed directory, and the parsing of
//! raw interactive input into those actions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// `d` - delete the directory and everything in it.
    DeleteAll,
    /// `c` - delete everything inside, keep the directory.
    ClearContents,
    /// `s` - overwrite files, then empty the directory, keeping the directory.
    ShredContents,
    /// `x` - overwrite files, then delete the directory and everything in it.
    ShredAll,
    Skip,
    Exit,
}

impl Action {
    /// Whether this action empties the directory without removing it.
    pub(crate) fn keeps_dir(self) -> bool {
        matches!(self, Action::ClearContents | Action::ShredContents)
    }

    /// Whether this action overwrites file contents before removing them.
    pub(crate) fn shreds(self) -> bool {
        matches!(self, Action::ShredContents | Action::ShredAll)
    }
}

/// Options that apply to a whole run.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Opts {
    pub(crate) dry_run: bool,
}

/// Parses one line of interactive input into `(action, apply_to_all, ok)`.
///
/// Matching is case-insensitive and tolerates surrounding whitespace. A
/// trailing `-all` suffix sets the apply-to-all flag. When `ok` is false the
/// input was not recognized and the caller is expected to skip instead.
pub(crate) fn parse_action(s: &str) -> (Action, bool, bool) {
    let mut all = false;
    let s = if let Some(stripped) = s.strip_suffix("-all") {
        all = true;
        stripped
    } else {
        s
    };
    match s {
        "d" => (Action::DeleteAll, all, true),
        "c" => (Action::ClearContents, all, true),
        "s" => (Action::ShredContents, all, true),
        "x" => (Action::ShredAll, all, true),
        "k" => (Action::Skip, all, true),
        "e" => (Action::Exit, all, true),
        _ => (Action::Skip, false, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_action() {
        assert_eq!(parse_action("d"), (Action::DeleteAll, false, true));
        assert_eq!(parse_action("c"), (Action::ClearContents, false, true));
        assert_eq!(parse_action("s"), (Action::ShredContents, false, true));
        assert_eq!(parse_action("x"), (Action::ShredAll, false, true));
        assert_eq!(parse_action("k"), (Action::Skip, false, true));
        assert_eq!(parse_action("e"), (Action::Exit, false, true));
    }

    #[test]
    fn test_parse_action_all_suffix() {
        assert_eq!(parse_action("d-all"), (Action::DeleteAll, true, true));
        assert_eq!(parse_action("c-all"), (Action::ClearContents, true, true));
        assert_eq!(parse_action("x-all"), (Action::ShredAll, true, true));
    }

    #[test]
    fn test_parse_action_rejects_unknown() {
        let (action, all, ok) = parse_action("q");
        assert_eq!(action, Action::Skip);
        assert!(!all);
        assert!(!ok);
    }
}
