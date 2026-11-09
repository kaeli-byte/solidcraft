//! The development identity table: one bearer token maps to exactly one [`Identity`].
//!
//! This is development authentication, not production authentication. The tokens come from the
//! environment, are compared without an early exit, and never appear in a response or a log. The
//! service enforces the permission sets on every route; a client's hidden control is never the
//! check.
//!
//! `SOLIDBOM_TOKEN` is the legacy combined identity (Engineer and Administrator), so an existing
//! single-token setup keeps working. `SOLIDBOM_ENGINEER_TOKEN` and `SOLIDBOM_ADMIN_TOKEN` add an
//! engineer-only and an administrator-only identity. Every token is at least
//! [`MIN_TOKEN_CHARS`] characters and all tokens are distinct; a duplicate is a startup failure.

use solidbom_contract::{Identity, Permission};
use uuid::Uuid;

/// The shortest development token the service accepts.
pub const MIN_TOKEN_CHARS: usize = 16;

/// The stable identity ids of the three development identities. They are constants so a client can
/// compare identities across a restart; they are development identities, not business records.
const DEVELOPMENT_ID: u128 = 0x5b09_0000_0000_4000_8000_0000_0000_0001;
const ENGINEER_ID: u128 = 0x5b09_0000_0000_4000_8000_0000_0000_0002;
const ADMINISTRATOR_ID: u128 = 0x5b09_0000_0000_4000_8000_0000_0000_0003;

/// The combined legacy identity: Engineer and Administrator, named `development`.
pub fn development_identity() -> Identity {
    Identity {
        identity_id: Uuid::from_u128(DEVELOPMENT_ID),
        name: "development".into(),
        permissions: vec![Permission::Administrator, Permission::Engineer],
    }
}

/// The engineer-only identity: project work but no shared-data maintenance.
pub fn engineer_identity() -> Identity {
    Identity { identity_id: Uuid::from_u128(ENGINEER_ID), name: "engineer".into(), permissions: vec![Permission::Engineer] }
}

/// The administrator-only identity: shared-data maintenance but no project work.
pub fn administrator_identity() -> Identity {
    Identity { identity_id: Uuid::from_u128(ADMINISTRATOR_ID), name: "administrator".into(), permissions: vec![Permission::Administrator] }
}

/// A token-to-identity table for one development deployment.
#[derive(Debug, Default, Clone)]
pub struct IdentityTable {
    entries: Vec<(String, Identity)>,
}

impl IdentityTable {
    /// An empty table. [`IdentityTable::validate`] refuses it at startup.
    pub fn new() -> Self {
        Self::default()
    }

    /// The legacy combined identity configured from `SOLIDBOM_TOKEN`.
    pub fn development(token: &str) -> Self {
        Self::new().with_token(token, development_identity())
    }

    /// Add one token and the identity it authenticates.
    pub fn with_token(mut self, token: impl Into<String>, identity: Identity) -> Self {
        self.entries.push((token.into(), identity));
        self
    }

    /// Add the engineer-only identity configured from `SOLIDBOM_ENGINEER_TOKEN`.
    pub fn with_engineer_token(self, token: &str) -> Self {
        self.with_token(token, engineer_identity())
    }

    /// Add the administrator-only identity configured from `SOLIDBOM_ADMIN_TOKEN`.
    pub fn with_administrator_token(self, token: &str) -> Self {
        self.with_token(token, administrator_identity())
    }

    /// Refuse a configuration the service cannot serve safely: no token, a token shorter than
    /// [`MIN_TOKEN_CHARS`], an invalid identity, or two entries that share one token.
    pub fn validate(&self) -> Result<(), String> {
        if self.entries.is_empty() {
            return Err("at least one development token is required".into());
        }
        for (index, (token, identity)) in self.entries.iter().enumerate() {
            if token.chars().count() < MIN_TOKEN_CHARS {
                return Err(format!("development token {} must be at least {MIN_TOKEN_CHARS} characters", index + 1));
            }
            identity.validate().map_err(|error| format!("development identity `{}` is invalid: {error}", identity.name))?;
            if self.entries[..index].iter().any(|(other, _)| token_matches(token, other)) {
                return Err("the development tokens must be distinct".into());
            }
        }
        Ok(())
    }

    /// The identity that owns `token`, if any. A missing or wrong token is `None`.
    pub fn authenticate(&self, token: &str) -> Option<Identity> {
        self.entries.iter().find(|(candidate, _)| token_matches(token, candidate)).map(|(_, identity)| identity.clone())
    }
}

/// Compare a bearer token without an early exit, so a wrong prefix is not measurably faster.
fn token_matches(given: &str, expected: &str) -> bool {
    let given = given.as_bytes();
    let expected = expected.as_bytes();
    if given.len() != expected.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in given.iter().zip(expected) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legacy_identity_combines_both_permission_sets() {
        let legacy = development_identity();
        assert!(legacy.has(Permission::Engineer));
        assert!(legacy.has(Permission::Administrator));
        assert_eq!(legacy.name, "development");
        assert!(legacy.validate().is_ok());

        let engineer = engineer_identity();
        assert!(engineer.has(Permission::Engineer));
        assert!(!engineer.has(Permission::Administrator));
        let administrator = administrator_identity();
        assert!(administrator.has(Permission::Administrator));
        assert!(!administrator.has(Permission::Engineer));
    }

    #[test]
    fn a_token_authenticates_only_its_own_identity() {
        let table = IdentityTable::development("development-token-012345").with_engineer_token("engineer-token-012345678");
        let found = table.authenticate("development-token-012345").map(|identity| identity.name);
        assert_eq!(found.as_deref(), Some("development"));
        let found = table.authenticate("engineer-token-012345678").map(|identity| identity.name);
        assert_eq!(found.as_deref(), Some("engineer"));
        assert!(table.authenticate("development-token-012346").is_none());
        assert!(table.authenticate("").is_none());
        assert!(table.authenticate("engineer-token-01234567").is_none());
    }

    #[test]
    fn a_configuration_with_too_short_or_duplicate_tokens_is_refused() {
        assert!(IdentityTable::new().validate().is_err());
        assert!(IdentityTable::development("short").validate().is_err());
        assert!(IdentityTable::development("long-enough-token-1").validate().is_ok());
        let duplicate = IdentityTable::development("the-same-token-012345").with_engineer_token("the-same-token-012345");
        assert!(duplicate.validate().is_err());
        let three = IdentityTable::development("legacy-token-0123456789")
            .with_engineer_token("engineer-token-0123456789")
            .with_administrator_token("administrator-token-0123456789");
        assert!(three.validate().is_ok());
    }
}
