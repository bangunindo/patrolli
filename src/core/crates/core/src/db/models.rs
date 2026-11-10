// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::{Serialize, Serializer};

pub fn serialize_utc_as_local<S>(dt: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let local_dt: DateTime<Tz> = dt.with_timezone(&crate::common::DEFAULT_TZ);
    local_dt.serialize(serializer)
}
