use std::path::PathBuf;

use crate::JSONRPCNotification;
use crate::JSONRPCRequest;
use crate::RequestId;
#[cfg(feature = "schema-export")]
use crate::export::GeneratedSchema;
#[cfg(feature = "schema-export")]
use crate::export::write_json_schema;
pub use codex_auth_types::AuthMode;
use codex_experimental_api_macros::ExperimentalApi;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use strum_macros::Display;
use ts_rs::TS;

use crate::protocol as v2;

include!("common_parts/envelopes.rs");
include!("common_parts/support_types.rs");

#[cfg(test)]
#[path = "common_tests.rs"]
mod common_tests;
