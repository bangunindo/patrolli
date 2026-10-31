// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::sensitive::{Sensitive, SensitiveRefs};
use crate::common;
use anyhow::Result;
use async_snmp::{Auth, AuthProtocol, MasterKeys, Oid, PrivProtocol, UsmConfig};
use serde::{Deserialize, Serialize};
use tracing::error;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "config",
    deny_unknown_fields
)]
pub enum SecurityModel {
    Usm(Usm),
}

impl SecurityModel {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Usm(usm) => usm.validate()?,
        };
        Ok(())
    }

    pub fn sensitive_mut(&mut self) -> SensitiveRefs<'_> {
        match self {
            Self::Usm(usm) => usm.sensitive_mut(),
        }
    }
}

impl From<&SecurityModel> for Auth {
    fn from(value: &SecurityModel) -> Self {
        match value {
            SecurityModel::Usm(usm) => {
                let mut config = UsmConfig::new(usm.username.clone());
                // `with_master_keys` rejects keys the compiled crypto backend can't
                // use; as before, that degrades to noAuthNoPriv rather than failing
                // the conversion.
                match usm.generate_master_keys() {
                    Some(master_keys) => match config.clone().with_master_keys(master_keys) {
                        Ok(with_keys) => config = with_keys,
                        Err(e) => error!(
                            snmp_user = %usm.username,
                            error = %e,
                            "snmpv3 credentials unusable, falling back to noauthnopriv"
                        ),
                    },
                    None if usm.authentication.is_some() => error!(
                        snmp_user = %usm.username,
                        "snmpv3 master key generation failed, falling back to noauthnopriv"
                    ),
                    None => {}
                }
                if let Some(context_name) = &usm.context_name {
                    config = config.context_name(context_name.clone());
                }
                config.into()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Usm {
    pub username: String,
    pub context_name: Option<String>,
    pub authentication: Option<Authentication>,
    pub privacy: Option<Privacy>,
}

impl Usm {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.username.is_empty() && self.username.len() <= 32,
            "username must be between 1 and 32 bytes"
        );
        anyhow::ensure!(
            !(self.privacy.is_some() && self.authentication.is_none()),
            "authentication is required if privacy is specified"
        );
        if let Some(auth) = &self.authentication {
            auth.validate()?;
        }
        if let Some(privacy) = &self.privacy {
            privacy.validate()?;
        }
        Ok(())
    }

    pub fn generate_master_keys(&self) -> Option<MasterKeys> {
        if let Some(auth) = &self.authentication {
            let mut master_keys = MasterKeys::new(
                auth.protocol.into(),
                auth.passphrase.expose().ok()?.as_bytes(),
            )
            .ok()?;
            if let Some(privacy) = &self.privacy {
                master_keys = master_keys
                    .with_privacy(
                        privacy.protocol.into(),
                        privacy.passphrase.expose().ok()?.as_bytes(),
                    )
                    .ok()?;
            }
            Some(master_keys)
        } else {
            None
        }
    }

    pub fn sensitive_mut(&mut self) -> SensitiveRefs<'_> {
        let mut refs = Vec::new();
        if let Some(auth) = &mut self.authentication {
            refs.push(("authentication.passphrase", &mut auth.passphrase));
        }
        if let Some(privacy) = &mut self.privacy {
            refs.push(("privacy.passphrase", &mut privacy.passphrase));
        }
        refs
    }
}

#[derive(Copy, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AuthenticationProtocol {
    Md5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl From<AuthenticationProtocol> for AuthProtocol {
    fn from(value: AuthenticationProtocol) -> Self {
        match value {
            AuthenticationProtocol::Md5 => AuthProtocol::Md5,
            AuthenticationProtocol::Sha1 => AuthProtocol::Sha1,
            AuthenticationProtocol::Sha224 => AuthProtocol::Sha224,
            AuthenticationProtocol::Sha256 => AuthProtocol::Sha256,
            AuthenticationProtocol::Sha384 => AuthProtocol::Sha384,
            AuthenticationProtocol::Sha512 => AuthProtocol::Sha512,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Authentication {
    pub protocol: AuthenticationProtocol,
    pub passphrase: Sensitive,
}

impl Authentication {
    pub fn validate(&self) -> Result<()> {
        if let Ok(passphrase) = self.passphrase.expose() {
            anyhow::ensure!(
                passphrase.len() >= 8,
                "authentication passphrase is too short, minimum 8 bytes"
            );
        }
        Ok(())
    }
}

#[derive(Copy, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum PrivacyProtocol {
    Des,
    Des3,
    Aes128,
    Aes192,
    Aes192Reeder,
    Aes256,
    Aes256Reeder,
}

impl From<PrivacyProtocol> for PrivProtocol {
    fn from(value: PrivacyProtocol) -> Self {
        match value {
            PrivacyProtocol::Des => PrivProtocol::Des,
            PrivacyProtocol::Des3 => PrivProtocol::Des3,
            PrivacyProtocol::Aes128 => PrivProtocol::Aes128,
            PrivacyProtocol::Aes192 => PrivProtocol::Aes192Blumenthal,
            PrivacyProtocol::Aes192Reeder => PrivProtocol::Aes192Reeder,
            PrivacyProtocol::Aes256 => PrivProtocol::Aes256Blumenthal,
            PrivacyProtocol::Aes256Reeder => PrivProtocol::Aes256Reeder,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Privacy {
    pub protocol: PrivacyProtocol,
    pub passphrase: Sensitive,
}

impl Privacy {
    pub fn validate(&self) -> Result<()> {
        if let Ok(passphrase) = self.passphrase.expose() {
            anyhow::ensure!(
                passphrase.len() >= 8,
                "privacy passphrase is too short, minimum 8 bytes"
            );
        }
        Ok(())
    }
}

#[derive(Copy, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OidPositions {
    /// index of the first sub-identifier to take, inclusive
    pub start: usize,
    /// index of the last sub-identifier to take, inclusive
    pub end: usize,
}

impl OidPositions {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.start <= self.end, "invalid oid position range");
        Ok(())
    }

    pub fn apply(&self, oid: &Oid) -> Option<Oid> {
        oid.arcs().get(self.start..=self.end).map(Oid::from_slice)
    }

    pub fn apply_reversed(&self, oid: &Oid) -> Option<Oid> {
        let arcs = oid.arcs();
        let last = arcs.len().checked_sub(1)?; // empty oid
        let start = last.checked_sub(self.end)?; // range reaches past the front
        let end = last.checked_sub(self.start)?;
        arcs.get(start..=end).map(Oid::from_slice)
    }
}

#[derive(Copy, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(
    rename_all = "snake_case",
    tag = "strategy",
    content = "config",
    deny_unknown_fields
)]
pub enum ChainStrategy {
    #[default]
    Value,
    OidPosition(OidPositions),
    OidPositionReversed(OidPositions),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OidChain {
    #[serde(with = "common::serde::oid")]
    #[schema(value_type = String)]
    pub oid: Oid,
    // root chain ignores this field
    #[serde(default)]
    pub chain_strategy: ChainStrategy,
    #[serde(default)]
    #[schema(no_recursion)]
    pub chain: Vec<OidChain>,
}

impl OidChain {
    /// Validates a root entry of `SnmpWalk::oids`, where `chain_strategy` is meaningless.
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.chain_strategy == ChainStrategy::Value,
            "chain_strategy is ignored on the root oid {}; set it on nested chains instead",
            self.oid
        );
        for chain in &self.chain {
            chain.validate_nested()?;
        }
        Ok(())
    }

    fn validate_nested(&self) -> Result<()> {
        match self.chain_strategy {
            ChainStrategy::OidPosition(pos) | ChainStrategy::OidPositionReversed(pos) => {
                pos.validate()?
            }
            ChainStrategy::Value => {}
        }
        for chain in &self.chain {
            chain.validate_nested()?;
        }
        Ok(())
    }

    /// This entry plus every entry nested under it, recursively.
    pub(super) fn oid_count(&self) -> usize {
        let mut oid_count = 1usize;
        for chain in &self.chain {
            oid_count += chain.oid_count();
        }
        oid_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(s: &str) -> Oid {
        Oid::parse(s).expect("valid oid")
    }

    fn pos(start: usize, end: usize) -> OidPositions {
        OidPositions { start, end }
    }

    #[test]
    fn apply_takes_an_inclusive_forward_slice() {
        let o = oid("10.20.30.40");
        assert_eq!(pos(0, 0).apply(&o), Some(oid("10")));
        assert_eq!(pos(0, 1).apply(&o), Some(oid("10.20")));
        assert_eq!(pos(1, 2).apply(&o), Some(oid("20.30")));
        assert_eq!(pos(3, 3).apply(&o), Some(oid("40")));
        assert_eq!(pos(0, 3).apply(&o), Some(o));
    }

    #[test]
    fn apply_reversed_counts_from_the_end_but_keeps_walk_order() {
        let o = oid("10.20.30.40");
        assert_eq!(pos(0, 0).apply_reversed(&o), Some(oid("40")));
        assert_eq!(pos(1, 1).apply_reversed(&o), Some(oid("30")));
        assert_eq!(pos(3, 3).apply_reversed(&o), Some(oid("10")));
        // the endpoints swap: reversed [0, 1] is the last two arcs, still left to right
        assert_eq!(pos(0, 1).apply_reversed(&o), Some(oid("30.40")));
        assert_eq!(pos(0, 3).apply_reversed(&o), Some(o));
    }

    #[test]
    fn oid_count_counts_the_entry_and_everything_nested_under_it() {
        fn chain(s: &str, chain: Vec<OidChain>) -> OidChain {
            OidChain {
                oid: oid(s),
                chain_strategy: ChainStrategy::Value,
                chain,
            }
        }

        assert_eq!(chain("10", vec![]).oid_count(), 1);
        assert_eq!(chain("10", vec![chain("20", vec![])]).oid_count(), 2);
        // two levels deep on one branch, flat on the other: 1 + (1 + 1) + 1
        assert_eq!(
            chain(
                "10",
                vec![chain("20", vec![chain("30", vec![])]), chain("40", vec![]),],
            )
            .oid_count(),
            4
        );
    }

    #[test]
    fn positions_past_the_end_of_the_oid_yield_none() {
        let o = oid("10.20.30.40");
        assert_eq!(pos(0, 4).apply(&o), None);
        assert_eq!(pos(4, 4).apply(&o), None);
        assert_eq!(pos(0, 4).apply_reversed(&o), None);
        assert_eq!(pos(4, 4).apply_reversed(&o), None);
    }

    #[test]
    fn empty_oid_yields_none_instead_of_underflowing() {
        let empty = Oid::empty();
        assert_eq!(pos(0, 0).apply(&empty), None);
        assert_eq!(pos(0, 0).apply_reversed(&empty), None);
    }

    #[test]
    fn an_inverted_range_is_none_rather_than_a_panic() {
        // OidPositions is Copy + Deserialize, so it can reach apply() without validate()
        let o = oid("10.20.30.40");
        assert_eq!(pos(3, 1).apply(&o), None);
        assert_eq!(pos(3, 1).apply_reversed(&o), None);
    }

    #[test]
    fn validate_rejects_an_inverted_range() {
        assert!(pos(0, 0).validate().is_ok());
        assert!(pos(1, 3).validate().is_ok());
        assert!(pos(3, 1).validate().is_err());
    }

    #[test]
    fn chain_strategy_is_adjacently_tagged() {
        assert_eq!(
            serde_json::to_value(ChainStrategy::Value).unwrap(),
            serde_json::json!({"strategy": "value"})
        );
        assert_eq!(
            serde_json::to_value(ChainStrategy::OidPositionReversed(pos(0, 1))).unwrap(),
            serde_json::json!({
                "strategy": "oid_position_reversed",
                "config": {"start": 0, "end": 1},
            })
        );
    }

    #[test]
    fn chain_strategy_deserializes_from_yaml() {
        let parsed: ChainStrategy =
            serde_yaml_ng::from_str("strategy: oid_position\nconfig:\n  start: 1\n  end: 2\n")
                .unwrap();
        assert_eq!(parsed, ChainStrategy::OidPosition(pos(1, 2)));
    }

    #[test]
    fn a_chain_without_a_strategy_still_deserializes_and_defaults_to_value() {
        // configs written before chain_strategy existed must keep working
        let chain: OidChain = serde_yaml_ng::from_str("oid: 1.3.6.1.2.1.2.2.1.2\n").unwrap();
        assert_eq!(chain.chain_strategy, ChainStrategy::Value);
        assert!(chain.chain.is_empty());
        chain.validate().unwrap();
    }

    #[test]
    fn validate_rejects_a_strategy_on_the_root_oid() {
        let chain: OidChain = serde_yaml_ng::from_str(
            r"
oid: 1.3.6.1.2.1.2.2.1.1
chain_strategy:
  strategy: oid_position
  config:
    start: 0
    end: 0
",
        )
        .unwrap();
        let err = chain.validate().unwrap_err().to_string();
        assert!(err.contains("ignored on the root oid"), "{err}");
    }

    #[test]
    fn validate_accepts_strategies_on_nested_chains() {
        let chain: OidChain = serde_yaml_ng::from_str(
            r"
oid: 1.3.6.1.2.1.2.2.1.1
chain:
  - oid: 1.3.6.1.2.1.31.1.1.1.18
    chain_strategy:
      strategy: oid_position_reversed
      config:
        start: 0
        end: 0
    chain:
      - oid: 1.3.6.1.2.1.4.20.1.2
        chain_strategy:
          strategy: oid_position
          config:
            start: 0
            end: 1
",
        )
        .unwrap();
        chain.validate().unwrap();
    }

    #[test]
    fn validate_recurses_into_grandchildren() {
        let chain: OidChain = serde_yaml_ng::from_str(
            r"
oid: 1.3.6.1.2.1.2.2.1.1
chain:
  - oid: 1.3.6.1.2.1.31.1.1.1.18
    chain:
      - oid: 1.3.6.1.2.1.4.20.1.2
        chain_strategy:
          strategy: oid_position
          config:
            start: 3
            end: 1
",
        )
        .unwrap();
        assert!(chain.validate().is_err());
    }
}
