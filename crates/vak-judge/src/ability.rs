//! What an Agent's learned ability has earned by its uses
//! (docs/design/93-self-evolving-capabilities.md R2): promotion and
//! withdrawal by observed states, never a count. Pure: the caller keeps
//! what each version's calls did.

/// What one installed version of an ability has done since it was made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VersionFacts {
    /// Its tests passed as the runtime ran them.
    pub passed: bool,
    /// A call to it succeeded in a turn other than the one that made it.
    pub succeeded_elsewhere: bool,
    /// A call to it failed by the program's own fault.
    pub failed: bool,
}

/// What the runtime does about the current version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbilityVerdict {
    /// It has shown it works: offer it to every Agent.
    Offer,
    /// It failed, and it was itself the fix of a version that failed:
    /// withdraw it for this Agent.
    Withdraw,
    /// Nothing to do yet.
    Keep,
}

/// The verdict on `current`, whose `parent` is the version it replaced.
pub fn verdict(current: VersionFacts, parent: Option<VersionFacts>) -> AbilityVerdict {
    if current.failed {
        return if parent.is_some_and(|parent| parent.failed) {
            AbilityVerdict::Withdraw
        } else {
            AbilityVerdict::Keep
        };
    }
    if current.passed && current.succeeded_elsewhere {
        AbilityVerdict::Offer
    } else {
        AbilityVerdict::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKS: VersionFacts = VersionFacts {
        passed: true,
        succeeded_elsewhere: true,
        failed: false,
    };
    const FAILED: VersionFacts = VersionFacts {
        passed: true,
        succeeded_elsewhere: false,
        failed: true,
    };

    #[test]
    fn a_version_that_worked_elsewhere_and_never_failed_is_offered() {
        assert_eq!(verdict(WORKS, None), AbilityVerdict::Offer);
        let built_only = VersionFacts {
            succeeded_elsewhere: false,
            ..WORKS
        };
        assert_eq!(verdict(built_only, None), AbilityVerdict::Keep);
    }

    #[test]
    fn a_fix_that_fails_again_is_withdrawn_and_a_first_failure_is_not() {
        assert_eq!(verdict(FAILED, None), AbilityVerdict::Keep);
        assert_eq!(verdict(FAILED, Some(WORKS)), AbilityVerdict::Keep);
        assert_eq!(verdict(FAILED, Some(FAILED)), AbilityVerdict::Withdraw);
        assert_eq!(verdict(WORKS, Some(FAILED)), AbilityVerdict::Offer);
    }
}
