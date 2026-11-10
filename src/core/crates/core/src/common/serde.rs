// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod duration_seconds {
    use serde::{Deserialize, Deserializer, Serializer};
    use tokio::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

pub mod duration_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use tokio::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u128(duration.as_millis())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

pub mod schedule {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};
    use tokio::time::Duration;

    use crate::config::item::Schedule;

    pub fn serialize<S>(schedule: &Schedule, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match schedule {
            Schedule::Interval(d) => serializer.serialize_u64(d.as_secs()),
            Schedule::Cron(c) => serializer.serialize_str(&c.to_string()),
            Schedule::Never => serializer.serialize_str("never"),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Schedule, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Number(u64),
            String(String),
        }

        match Raw::deserialize(deserializer)? {
            Raw::Number(secs) => Ok(Schedule::Interval(Duration::from_secs(secs))),
            Raw::String(s) => s.parse::<Schedule>().map_err(D::Error::custom),
        }
    }
}

pub mod datetime_tz {
    use chrono::{DateTime, FixedOffset, TimeZone, Utc};
    use chrono_tz::Tz;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Helper {
        dt: DateTime<FixedOffset>,
        tz: Tz,
    }

    #[derive(Serialize, Deserialize)]
    struct BinHelper(i64, String);

    fn to_bin(value: &DateTime<Tz>) -> Result<BinHelper, &'static str> {
        let nanos = value
            .timestamp_nanos_opt()
            .ok_or("timestamp out of range for i64 nanoseconds")?;
        Ok(BinHelper(nanos, value.timezone().name().to_string()))
    }

    fn from_bin(h: BinHelper) -> Result<DateTime<Tz>, chrono_tz::ParseError> {
        let tz: Tz = h.1.parse()?;
        let utc = DateTime::<Utc>::from_timestamp_nanos(h.0);
        Ok(utc.with_timezone(&tz))
    }

    pub fn serialize<S>(value: &DateTime<Tz>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            Helper {
                dt: value.fixed_offset(),
                tz: value.timezone(),
            }
            .serialize(serializer)
        } else {
            to_bin(value)
                .map_err(serde::ser::Error::custom)?
                .serialize(serializer)
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Tz>, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let h = Helper::deserialize(deserializer)?;
            Ok(h.tz.from_utc_datetime(&h.dt.naive_utc()))
        } else {
            let h = BinHelper::deserialize(deserializer)?;
            from_bin(h).map_err(D::Error::custom)
        }
    }

    pub mod option {
        use super::{BinHelper, Helper, from_bin, to_bin};
        use chrono::{DateTime, TimeZone};
        use chrono_tz::Tz;
        use serde::de::Error as _;
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        pub fn serialize<S: Serializer>(
            value: &Option<DateTime<Tz>>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            if s.is_human_readable() {
                value
                    .as_ref()
                    .map(|v| Helper {
                        dt: v.fixed_offset(),
                        tz: v.timezone(),
                    })
                    .serialize(s)
            } else {
                value
                    .as_ref()
                    .map(to_bin)
                    .transpose()
                    .map_err(serde::ser::Error::custom)?
                    .serialize(s)
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<DateTime<Tz>>, D::Error> {
            if d.is_human_readable() {
                Ok(Option::<Helper>::deserialize(d)?
                    .map(|h| h.tz.from_utc_datetime(&h.dt.naive_utc())))
            } else {
                Option::<BinHelper>::deserialize(d)?
                    .map(from_bin)
                    .transpose()
                    .map_err(D::Error::custom)
            }
        }
    }
}

pub mod oid {
    use crate::common::mib::get_name_by_oid;
    use crate::common::snmp::parse_oid;
    use async_snmp::Oid;
    use mib_rs::Oid as MibOid;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(oid: &Oid, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let oid_name = get_name_by_oid(&MibOid::from(oid.arcs()));
        match oid_name {
            Some(oid_name) => serializer.serialize_str(oid_name.as_str()),
            None => serializer.serialize_str(oid.to_string().as_str()),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Oid, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let oid = parse_oid(&s).map_err(serde::de::Error::custom)?;
        Ok(oid)
    }
}

pub mod oid_vec {
    use async_snmp::Oid;
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    struct OidAsString<'a>(&'a Oid);

    impl<'a> Serialize for OidAsString<'a> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            super::oid::serialize(self.0, serializer)
        }
    }

    pub fn serialize<S>(oids: &Vec<Oid>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(oids.len()))?;
        for oid in oids {
            seq.serialize_element(&OidAsString(oid))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Oid>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Vec::<String>::deserialize(deserializer)?;
        raw.into_iter()
            .map(|s| super::oid::deserialize(serde::de::IntoDeserializer::into_deserializer(s)))
            .collect()
    }
}

pub mod header_value_vec {
    use axum::http::HeaderValue;
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(values: &Vec<HeaderValue>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(values.len()))?;
        for value in values {
            let value_str = value.to_str().map_err(serde::ser::Error::custom)?;
            seq.serialize_element(value_str)?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<HeaderValue>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Vec::<String>::deserialize(deserializer)?;
        raw.into_iter()
            .map(|value| HeaderValue::from_str(&value).map_err(serde::de::Error::custom))
            .collect()
    }
}
